use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::VecDeque;

wit_bindgen::generate!({
    world: "scheduler:event-bus/event-bus-world@0.2.0",
    path: ["../wit/eventbus"],
    generate_all,
    debug: true,
});

type WitEvent = exports::scheduler::event_bus::event_bus::Event;

/// 简单内存事件队列（stub），满足 publish 接口，便于后续替换为真实总线。
#[derive(Default)]
struct EventStore {
    queue: VecDeque<WitEvent>,
}

static STORE: Lazy<Mutex<EventStore>> = Lazy::new(|| Mutex::new(EventStore::default()));

struct EventBusComponent;

impl exports::scheduler::event_bus::event_bus::Guest for EventBusComponent {
    fn publish(event: WitEvent) -> Result<(), String> {
        {
            let mut store = STORE.lock();
            store.queue.push_back(event.clone());
        }
        // 便于调试观测
        println!(
            "[eventbus] publish kind={} user={:?} task={:?} action={:?}",
            event.kind, event.user_id, event.task_id, event.action_id
        );
        Ok(())
    }
}

/// 便于单测/调试读取已发布事件。
pub fn drain_events() -> Vec<WitEvent> {
    let mut store = STORE.lock();
    store.queue.drain(..).collect()
}

/// 辅助函数：将 scheduler 的 Action 结果事件编码为统一的 Event 并入队。
///
/// 该函数不会改变 WIT 接口，只是为需要以 `"scheduler.action-result"` 形式
/// 观察 action 结果的调用方提供一个简化入口。
pub fn emit_scheduler_action_result(
    user_id: Option<String>,
    task_id: Option<String>,
    action_id: Option<String>,
    status: String,
    detail: Option<String>,
) {
    use std::time::{SystemTime, UNIX_EPOCH};

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let payload = serde_json::json!({
        "status": status,
        "detail": detail,
    })
    .to_string();

    let event = WitEvent {
        id: format!("ar-{}", now_ms),
        kind: "scheduler.action-result".to_string(),
        user_id,
        task_id,
        action_id,
        payload,
        correlation_id: None,
        timestamp_ms: now_ms,
    };

    // 复用与 publish 相同的入队与打印逻辑
    let _ = EventBusComponent::publish(event);
}

export!(EventBusComponent);
