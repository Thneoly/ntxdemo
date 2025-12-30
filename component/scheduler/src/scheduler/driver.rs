//! Scheduler driver / orchestration.
//!
//! This module owns the long-running event loop and subscription wiring.
//! It exists to keep `lib.rs` focused on exports + shared state.

use crate::eventing::{action_result, topics::EventKind, topics::TopicFilter};
use crate::io::{rx_pump::pump_rx_once_nonblocking, send_scheduler, tx};
use crate::runtime::runtime_state::RUNTIME;
use crate::scenario::scenario_registry::SCENARIOS;
use crate::scheduler::{dispatch, handlers, load_controller, timers};
use crate::{publish_scheduler_state, SchedulerContext, SchedulerState};

pub(crate) fn subscribe_or_log(filter: TopicFilter) -> Option<String> {
    let filter_str = filter.as_filter_str();
    match crate::bindmod::ntx::scenario_eventbus::event_bus::subscribe(filter_str) {
        Ok(id) => {
            println!("[scheduler] subscribed {} -> {}", filter_str, id);
            Some(id)
        }
        Err(e) => {
            println!("[scheduler] subscribe {} failed: {}", filter_str, e);
            None
        }
    }
}

pub(crate) fn init_runtime(ctx: &SchedulerContext) -> Result<(), String> {
    let Some(sc) = &ctx.scenario.parsed else {
        return Ok(());
    };

    // reset runtime flags
    if let Ok(mut rt) = RUNTIME.lock() {
        rt.users.clear();
        rt.ready.clear();
        rt.paused = false;
        rt.stop = false;
    }

    // init load controller
    if let Ok(mut lc) = load_controller::LOAD.lock() {
        lc.started_at_ms = crate::now_ms();
        lc.next_phase = 0;
        lc.next_user_seq = 1;
        lc.phases = sc.load.ramp_up.phases.clone();
    }

    // init scenario registry with version=1
    if let Ok(mut reg) = SCENARIOS.lock() {
        reg.reset_with(sc.clone());
    }

    // 若没有 ramp-up phases，默认立即启动 1 个用户
    if sc.load.ramp_up.phases.is_empty() {
        load_controller::publish_user_start_event(crate::now_ms(), 1, None);
    } else {
        // 允许 phase.at_second == 0 立即触发
        load_controller::tick_load_controller(crate::now_ms());
    }

    Ok(())
}

/// Simple event loop: poll multiple subscriptions and dispatch.
///
/// Notes:
/// - Includes best-effort fallback RX pumping for runtimes without WASI threads.
/// - Never exits unless stop flag is set.
pub(crate) fn run_event_loop(
    ctx: &SchedulerContext,
    sub_tx: Option<&str>,
    sub_send: Option<&str>,
    sub_ar: Option<&str>,
    sub_rx: Option<&str>,
    sub_ctrl: Option<&str>,
    sub_timer: Option<&str>,
    sub_user: Option<&str>,
    sub_topo: Option<&str>,
) -> Result<(), String> {
    let mut idle = 0u32;
    loop {
        let mut did_work = false;

        // Fallback RX pumping for runtimes without WASI threads.
        // Non-blocking (0ms wait), so it won't stall the event loop.
        pump_rx_once_nonblocking();

        // 1) control events (blocking wait)
        if let Some(id) = sub_ctrl {
            let events = crate::bindmod::ntx::scenario_eventbus::event_bus::wait_events(id, 64, 50)
                .map_err(|e| format!("poll_events(control): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                handlers::on_control_event(&ev);
            }
        }

        // stop flag
        if RUNTIME.lock().map(|rt| rt.stop).unwrap_or(false) {
            return Ok(());
        }

        if let Some(id) = sub_tx {
            let events = crate::bindmod::ntx::scenario_eventbus::event_bus::wait_events(id, 64, 0)
                .map_err(|e| format!("poll_events(packet.tx-request): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                if ev.kind == EventKind::PacketTxRequest.as_str() {
                    if let Err(e) = tx::handle_tx_request(&ev.payload, ev.correlation_id.as_deref())
                    {
                        println!("[scheduler] process tx-request failed: {e}");
                    }
                }
            }
        }

        // 1.5) send schedule requests (executor -> scheduler)
        if let Some(id) = sub_send {
            let events = crate::bindmod::ntx::scenario_eventbus::event_bus::wait_events(id, 64, 0)
                .map_err(|e| format!("poll_events(send.schedule-request): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                if ev.kind == EventKind::SendScheduleRequest.as_str() {
                    if let Err(e) = send_scheduler::on_send_schedule_request(&ev) {
                        println!("[scheduler] handle send.schedule-request failed: {e}");
                    }
                }
            }
        }

        // 2) action-result events
        if let Some(id) = sub_ar {
            let events = crate::bindmod::ntx::scenario_eventbus::event_bus::wait_events(id, 64, 0)
                .map_err(|e| format!("poll_events(action-result): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                if ev.kind == EventKind::SchedulerActionResult.as_str() {
                    action_result::on_action_result_event(ctx, &ev)?;
                }
            }
        }

        if let Some(id) = sub_rx {
            let events = crate::bindmod::ntx::scenario_eventbus::event_bus::wait_events(id, 64, 0)
                .map_err(|e| format!("poll_events(packet.rx): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                if ev.kind == EventKind::PacketRx.as_str() {
                    handlers::on_packet_rx(ctx, &ev)?;
                }
            }
        }

        // 3) timer events
        if let Some(id) = sub_timer {
            let events = crate::bindmod::ntx::scenario_eventbus::event_bus::wait_events(id, 64, 0)
                .map_err(|e| format!("poll_events(timer): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                if ev.kind.starts_with(
                    TopicFilter::SchedulerTimerAll
                        .as_filter_str()
                        .trim_end_matches('*'),
                ) {
                    handlers::on_timer_event(ctx, &ev)?;
                }
            }
        }

        // 4) user lifecycle events
        if let Some(id) = sub_user {
            let events = crate::bindmod::ntx::scenario_eventbus::event_bus::wait_events(id, 64, 0)
                .map_err(|e| format!("poll_events(user): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                match ev.kind.as_str() {
                    k if k == EventKind::SchedulerUserStart.as_str() => {
                        handlers::on_user_start_event(ctx, &ev)?
                    }
                    k if k == EventKind::SchedulerUserExit.as_str() => {
                        handlers::on_user_exit_event(ctx, &ev)?
                    }
                    _ => {}
                }
            }
        }

        // 4.1) topology change events (affect only NEW users)
        if let Some(id) = sub_topo {
            let events = crate::bindmod::ntx::scenario_eventbus::event_bus::wait_events(id, 16, 0)
                .map_err(|e| format!("poll_events(topology): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                if ev.kind == EventKind::TopologyChanged.as_str() {
                    if let Err(e) = handlers::on_topology_changed_event(ctx, &ev) {
                        println!("[scheduler] warn: topology.changed rejected: {}", e);
                    }
                }
            }
        }

        // 5) drive load/timers
        let now = crate::now_ms();
        load_controller::tick_load_controller(now);
        timers::tick_timers(now);

        let paused = RUNTIME.lock().map(|rt| rt.paused).unwrap_or(false);
        if !paused {
            // 调度 ready tasks
            did_work |= dispatch::dispatch_ready_tasks(ctx, 16)?;
        }

        // send-scheduler tick
        send_scheduler::tick_send_scheduler(crate::now_ms());

        if did_work {
            idle = 0;
        } else {
            idle = idle.saturating_add(1);
        }

        if idle >= 10 {
            let has_active_send = send_scheduler::has_active_jobs();
            let has_ready = RUNTIME
                .lock()
                .map(|rt| !rt.ready.is_empty())
                .unwrap_or(false);
            if !has_active_send && !has_ready {
                idle = 0;
            }
        }

        // State is published elsewhere on start/stop; keep this for future hooks.
        let _ = publish_scheduler_state;
        let _ = SchedulerState::Running;
    }
}
