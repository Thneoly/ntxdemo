use std::collections::HashMap;
use std::sync::Mutex;

use once_cell::sync::Lazy;

use crate::eventing::events::publish_event_with_corr;
use crate::eventing::topics::EventKind;
use crate::scheduler::time;
use crate::{ntx, SendRequest, SendRequestState, SendSchedule};

use crate::net::net_hooks;

#[derive(Clone)]
struct SendJob {
    req: SendRequest,
    next_send_ms: u64,
    total_sent: u32,
    last_sent_time_ms: Option<u64>,
    last_error: Option<String>,
}

static SEND_JOBS: Lazy<Mutex<HashMap<String, SendJob>>> = Lazy::new(|| Mutex::new(HashMap::new()));

pub fn on_send_schedule_request(
    ev: &ntx::scenario_eventbus::event_bus::Event,
) -> Result<(), String> {
    // Parse executor-published JSON payload and enqueue job.
    // Payload is expected to be compatible with core-types SendRequest fields.
    #[derive(serde::Deserialize)]
    struct SendScheduleReq {
        request_id: String,
        user_id: String,
        task_id: String,
        socket_id: u64,
        #[serde(default)]
        max_count: Option<u32>,
        #[serde(default)]
        timeout_ms: Option<u64>,
        #[serde(default)]
        payload_bytes: Option<Vec<u8>>,
        #[serde(default)]
        schedule: serde_json::Value,
    }

    fn parse_schedule(v: &serde_json::Value) -> Result<SendSchedule, String> {
        let mode = v
            .get("mode")
            .and_then(|x| x.as_str())
            .unwrap_or("once")
            .trim()
            .to_ascii_lowercase();
        match mode.as_str() {
            "once" => Ok(SendSchedule::Once),
            "periodic" => {
                let interval_ms = v
                    .get("interval_ms")
                    .and_then(|x| x.as_u64())
                    .ok_or_else(|| "missing schedule.interval_ms".to_string())?;
                let start_delay_ms = v.get("start_delay_ms").and_then(|x| x.as_u64());
                Ok(SendSchedule::Periodic(
                    crate::ntx::core_types::types::PeriodicSchedule {
                        interval_ms,
                        start_delay_ms,
                    },
                ))
            }
            "timetable" => {
                let ts = v
                    .get("timestamps_ms")
                    .and_then(|x| x.as_array())
                    .ok_or_else(|| "missing schedule.timestamps_ms".to_string())?;
                let mut out: Vec<u64> = Vec::with_capacity(ts.len());
                for x in ts {
                    let n = x
                        .as_u64()
                        .ok_or_else(|| "timestamps_ms must be u64".to_string())?;
                    out.push(n);
                }
                Ok(SendSchedule::Timetable(
                    crate::ntx::core_types::types::TimetableSchedule { timestamps_ms: out },
                ))
            }
            "rate-limited" | "rate_limited" | "ratelimited" => {
                let pps = v
                    .get("pps")
                    .and_then(|x| x.as_u64())
                    .ok_or_else(|| "missing schedule.pps".to_string())?;
                let burst_size = v.get("burst_size").and_then(|x| x.as_u64());
                Ok(SendSchedule::RateLimited(
                    crate::ntx::core_types::types::RateLimitedSchedule {
                        pps: pps as u32,
                        burst_size: burst_size.map(|b| b as u32),
                    },
                ))
            }
            other => Err(format!("unsupported schedule mode: {other}")),
        }
    }

    let req: SendScheduleReq =
        serde_json::from_str(&ev.payload).map_err(|e| format!("parse payload json: {e}"))?;

    let schedule = parse_schedule(&req.schedule)?;

    let payload = req
        .payload_bytes
        .ok_or_else(|| "missing payload_bytes for send.schedule-request".to_string())?;

    let core_req = SendRequest {
        request_id: req.request_id.clone(),
        user_id: req.user_id.clone(),
        task_id: req.task_id.clone(),
        socket_id: req.socket_id,
        schedule,
        payload: Some(payload),
        payload_generator: None,
        max_count: req.max_count,
        timeout_ms: req.timeout_ms,
    };

    // basic validation
    if let SendSchedule::RateLimited(r) = &core_req.schedule {
        if r.pps == 0 {
            return Err("pps must be > 0".to_string());
        }
    }

    let now = time::now_ms();
    let next_ms = calc_initial_next_send_ms(&core_req, now);
    let job = SendJob {
        req: core_req.clone(),
        next_send_ms: next_ms,
        total_sent: 0,
        last_sent_time_ms: None,
        last_error: None,
    };
    if let Ok(mut map) = SEND_JOBS.lock() {
        map.insert(core_req.request_id.clone(), job);
    }

    // optional: publish a status/ack event
    publish_event_with_corr(
        EventKind::SendScheduled.as_str(),
        Some(core_req.user_id.as_str()),
        Some(core_req.task_id.as_str()),
        ev.action_id.as_deref(),
        ev.correlation_id.as_deref(),
        serde_json::to_value(crate::SendScheduledPayload {
            request_id: core_req.request_id.clone(),
            socket_id: core_req.socket_id,
            state: "pending".to_string(),
            next_send_ms: next_ms,
        })
        .unwrap_or(serde_json::json!({})),
    );

    Ok(())
}

pub fn tick_send_scheduler(now_ms: u64) {
    let mut to_send: Vec<String> = Vec::new();
    {
        if let Ok(jobs) = SEND_JOBS.lock() {
            for (id, job) in jobs.iter() {
                if is_job_active(job) && job.next_send_ms <= now_ms {
                    to_send.push(id.clone());
                }
            }
        }
    }

    for id in to_send {
        let mut remove = false;
        if let Some(mut job) = SEND_JOBS.lock().ok().and_then(|mut m| m.remove(&id)) {
            if let Err(e) = net_hooks::send_on_socket(
                job.req.socket_id,
                job.req.payload.as_deref().unwrap_or(&[]),
                Some(job.req.user_id.as_str()),
                Some(job.req.task_id.as_str()),
                None,
                None,
            ) {
                job.last_error = Some(e);
            } else {
                job.total_sent = job.total_sent.saturating_add(1);
                job.last_sent_time_ms = Some(now_ms);

                if let Some(max) = job.req.max_count {
                    if job.total_sent >= max {
                        remove = true;
                    }
                }

                if !remove {
                    if let Some(next) = next_due_after(&job, now_ms) {
                        job.next_send_ms = next;
                    } else {
                        remove = true;
                    }
                }
            }

            if !remove {
                if let Ok(mut m) = SEND_JOBS.lock() {
                    m.insert(id.clone(), job);
                }
            }
        }
    }
}

/// Returns whether there are any jobs in Pending/Active state.
/// Used by the main loop to decide whether it's safe to block longer.
pub fn has_active_jobs() -> bool {
    SEND_JOBS
        .lock()
        .map(|m| m.values().any(is_job_active))
        .unwrap_or(true)
}

/// Best-effort cancel all queued jobs owned by a user.
pub fn cancel_jobs_for_user(user_id: &str) {
    if let Ok(mut jobs) = SEND_JOBS.lock() {
        jobs.retain(|_, j| j.req.user_id != user_id);
    }
}

fn is_job_active(job: &SendJob) -> bool {
    matches!(
        job_state(job),
        SendRequestState::Pending | SendRequestState::Active
    )
}

fn job_state(job: &SendJob) -> SendRequestState {
    if job
        .last_error
        .as_ref()
        .map(|s| !s.is_empty())
        .unwrap_or(false)
    {
        return SendRequestState::Error;
    }

    if let Some(max) = job.req.max_count {
        if job.total_sent >= max {
            return SendRequestState::Completed;
        }
    }

    if job.total_sent > 0 {
        SendRequestState::Active
    } else {
        SendRequestState::Pending
    }
}

fn calc_initial_next_send_ms(req: &SendRequest, base_ms: u64) -> u64 {
    match &req.schedule {
        SendSchedule::Once => base_ms,
        SendSchedule::Periodic(p) => base_ms + p.start_delay_ms.unwrap_or(0),
        SendSchedule::Timetable(t) => base_ms + t.timestamps_ms.first().cloned().unwrap_or(0),
        SendSchedule::RateLimited(_) => base_ms,
    }
}

fn next_due_after(job: &SendJob, now_ms: u64) -> Option<u64> {
    match &job.req.schedule {
        SendSchedule::Once => None,
        SendSchedule::Periodic(p) => Some(now_ms + p.interval_ms),
        SendSchedule::Timetable(t) => {
            let idx = job.total_sent as usize;
            t.timestamps_ms
                .get(idx + 1)
                .map(|delta| now_ms.saturating_add(*delta))
        }
        SendSchedule::RateLimited(r) => {
            if r.pps == 0 {
                None
            } else {
                let interval = 1000u64 / (r.pps as u64);
                Some(now_ms + interval)
            }
        }
    }
}
