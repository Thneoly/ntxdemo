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

#[derive(Debug, Clone, Copy)]
pub(crate) struct StepArgsLite {
    pub max_events: u32,
    pub max_dispatch: usize,
    pub timeout_ms: u32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct StepStats {
    pub did_work: bool,
    pub processed_events: u32,
    pub dispatched: u32,
    pub rx_batches: u32,
    pub suggested_wait_ms: u32,
}

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

/// Execute a single scheduler step.
///
/// This is used by both:
/// - the legacy long-running `run_event_loop` (guest-driven)
/// - the host-driven `step` export (host calls once per tick)
pub(crate) fn step_once(
    ctx: &SchedulerContext,
    sub_tx: Option<&str>,
    sub_send: Option<&str>,
    sub_ar: Option<&str>,
    sub_rx: Option<&str>,
    sub_ctrl: Option<&str>,
    sub_timer: Option<&str>,
    sub_user: Option<&str>,
    sub_topo: Option<&str>,
    args: StepArgsLite,
) -> Result<StepStats, String> {
    let mut stats = StepStats::default();

    fn take_budget(remaining: &mut u32, want: u32) -> u32 {
        if *remaining == 0 {
            return 0;
        }
        let n = want.min(*remaining);
        *remaining = remaining.saturating_sub(n);
        n
    }

    // Fallback RX pumping for runtimes without WASI threads.
    // Non-blocking (0ms wait), so it won't stall the event loop.
    let rx_batches = pump_rx_once_nonblocking();
    stats.rx_batches = rx_batches;
    if rx_batches > 0 {
        stats.did_work = true;
    }

    let mut remaining = args.max_events;

    // 1) control events (bounded wait)
    if remaining > 0 {
        if let Some(id) = sub_ctrl {
            let n = take_budget(&mut remaining, 64);
            let events = crate::bindmod::ntx::scenario_eventbus::event_bus::wait_events(
                id,
                n,
                args.timeout_ms,
            )
            .map_err(|e| format!("poll_events(control): {e}"))?;
            if !events.is_empty() {
                stats.did_work = true;
                stats.processed_events = stats.processed_events.saturating_add(events.len() as u32);
            }
            for ev in events {
                handlers::on_control_event(&ev);
            }
        }
    }

    // stop flag
    if RUNTIME.lock().map(|rt| rt.stop).unwrap_or(false) {
        // Let caller converge state.
        stats.suggested_wait_ms = 0;
        return Ok(stats);
    }

    // 2) packet.tx-request
    if remaining > 0 {
        if let Some(id) = sub_tx {
            let n = take_budget(&mut remaining, 64);
            let events = crate::bindmod::ntx::scenario_eventbus::event_bus::wait_events(id, n, 0)
                .map_err(|e| format!("poll_events(packet.tx-request): {e}"))?;
            if !events.is_empty() {
                stats.did_work = true;
                stats.processed_events = stats.processed_events.saturating_add(events.len() as u32);
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
    }

    // 2.5) send schedule requests
    if remaining > 0 {
        if let Some(id) = sub_send {
            let n = take_budget(&mut remaining, 64);
            let events = crate::bindmod::ntx::scenario_eventbus::event_bus::wait_events(id, n, 0)
                .map_err(|e| format!("poll_events(send.schedule-request): {e}"))?;
            if !events.is_empty() {
                stats.did_work = true;
                stats.processed_events = stats.processed_events.saturating_add(events.len() as u32);
            }
            for ev in events {
                if ev.kind == EventKind::SendScheduleRequest.as_str() {
                    if let Err(e) = send_scheduler::on_send_schedule_request(&ev) {
                        println!("[scheduler] handle send.schedule-request failed: {e}");
                    }
                }
            }
        }
    }

    // 3) action-result
    if remaining > 0 {
        if let Some(id) = sub_ar {
            let n = take_budget(&mut remaining, 64);
            let events = crate::bindmod::ntx::scenario_eventbus::event_bus::wait_events(id, n, 0)
                .map_err(|e| format!("poll_events(action-result): {e}"))?;
            if !events.is_empty() {
                stats.did_work = true;
                stats.processed_events = stats.processed_events.saturating_add(events.len() as u32);
            }
            for ev in events {
                if ev.kind == EventKind::SchedulerActionResult.as_str() {
                    action_result::on_action_result_event(ctx, &ev)?;
                }
            }
        }
    }

    // 4) packet.rx events
    if remaining > 0 {
        if let Some(id) = sub_rx {
            let n = take_budget(&mut remaining, 64);
            let events = crate::bindmod::ntx::scenario_eventbus::event_bus::wait_events(id, n, 0)
                .map_err(|e| format!("poll_events(packet.rx): {e}"))?;
            if !events.is_empty() {
                stats.did_work = true;
                stats.processed_events = stats.processed_events.saturating_add(events.len() as u32);
            }
            for ev in events {
                if ev.kind == EventKind::PacketRx.as_str() {
                    handlers::on_packet_rx(ctx, &ev)?;
                }
            }
        }
    }

    // 5) timer events
    if remaining > 0 {
        if let Some(id) = sub_timer {
            let n = take_budget(&mut remaining, 64);
            let events = crate::bindmod::ntx::scenario_eventbus::event_bus::wait_events(id, n, 0)
                .map_err(|e| format!("poll_events(timer): {e}"))?;
            if !events.is_empty() {
                stats.did_work = true;
                stats.processed_events = stats.processed_events.saturating_add(events.len() as u32);
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
    }

    // 6) user lifecycle events
    if remaining > 0 {
        if let Some(id) = sub_user {
            let n = take_budget(&mut remaining, 64);
            let events = crate::bindmod::ntx::scenario_eventbus::event_bus::wait_events(id, n, 0)
                .map_err(|e| format!("poll_events(user): {e}"))?;
            if !events.is_empty() {
                stats.did_work = true;
                stats.processed_events = stats.processed_events.saturating_add(events.len() as u32);
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
    }

    // 6.1) topology change events
    if remaining > 0 {
        if let Some(id) = sub_topo {
            let n = take_budget(&mut remaining, 16);
            let events = crate::bindmod::ntx::scenario_eventbus::event_bus::wait_events(id, n, 0)
                .map_err(|e| format!("poll_events(topology): {e}"))?;
            if !events.is_empty() {
                stats.did_work = true;
                stats.processed_events = stats.processed_events.saturating_add(events.len() as u32);
            }
            for ev in events {
                if ev.kind == EventKind::TopologyChanged.as_str() {
                    if let Err(e) = handlers::on_topology_changed_event(ctx, &ev) {
                        println!("[scheduler] warn: topology.changed rejected: {}", e);
                    }
                }
            }
        }
    }

    // 7) drive load/timers + dispatch
    let now = crate::now_ms();
    load_controller::tick_load_controller(now);
    timers::tick_timers(now);

    let paused = RUNTIME.lock().map(|rt| rt.paused).unwrap_or(false);
    if !paused {
        let dispatched = dispatch::dispatch_ready_tasks_count(ctx, args.max_dispatch)?;
        if dispatched > 0 {
            stats.did_work = true;
            stats.dispatched = dispatched;
        }
    }

    // send-scheduler tick
    send_scheduler::tick_send_scheduler(crate::now_ms());

    // Simple suggestion: when idle, recommend a short sleep to avoid busy looping.
    if !stats.did_work {
        stats.suggested_wait_ms = 1;
    }

    Ok(stats)
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
    let _ = publish_scheduler_state;
    let _ = SchedulerState::Running;

    loop {
        let stats = step_once(
            ctx,
            sub_tx,
            sub_send,
            sub_ar,
            sub_rx,
            sub_ctrl,
            sub_timer,
            sub_user,
            sub_topo,
            StepArgsLite {
                max_events: 512,
                max_dispatch: 16,
                timeout_ms: 50,
            },
        )?;

        if RUNTIME.lock().map(|rt| rt.stop).unwrap_or(false) {
            return Ok(());
        }

        // Preserve legacy behavior: avoid tight spinning in idle loops.
        if !stats.did_work {
            // Best-effort idle backoff.
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}
