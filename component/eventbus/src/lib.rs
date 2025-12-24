use crate::exports::ntx::scenario_eventbus::event_bus::Guest;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};

wit_bindgen::generate!({
    world: "ntx:scenario-eventbus/event-bus-world@0.1.0",
    path: ["../wit/eventbus"],
    generate_all,
    debug: true,
});

type WitEvent = exports::ntx::scenario_eventbus::event_bus::Event;

/// 简单内存事件队列（stub），满足 publish 接口，便于后续替换为真实总线。
#[derive(Default)]
struct EventStore {
    queue: VecDeque<WitEvent>,
}

#[derive(Clone)]
struct Subscription {
    id: String,
    topic_filter: String,
    events: VecDeque<WitEvent>,
}

static STORE: Lazy<Mutex<EventStore>> = Lazy::new(|| Mutex::new(EventStore::default()));
static SUBSCRIPTIONS: Lazy<Mutex<HashMap<String, Subscription>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static SUBSCRIPTION_COUNTER: AtomicU64 = AtomicU64::new(1);

fn matches_filter(topic: &str, filter: &str) -> bool {
    if filter.ends_with('*') {
        let prefix = &filter[..filter.len() - 1];
        topic.starts_with(prefix)
    } else {
        topic == filter
    }
}

struct EventBusComponent;

impl exports::ntx::scenario_eventbus::event_bus::Guest for EventBusComponent {
    fn publish(event: WitEvent) -> Result<(), String> {
        {
            let mut store = STORE.lock();
            store.queue.push_back(event.clone());
        }

        // 检查所有订阅，将匹配的事件加入订阅队列
        {
            let mut subs = SUBSCRIPTIONS.lock();
            for sub in subs.values_mut() {
                if matches_filter(&event.kind, &sub.topic_filter) {
                    sub.events.push_back(event.clone());
                }
            }
        }

        // 便于调试观测
        println!(
            "[eventbus] publish kind={} user={:?} task={:?} action={:?}",
            event.kind, event.user_id, event.task_id, event.action_id
        );
        Ok(())
    }

    fn subscribe(topic_filter: String) -> Result<String, String> {
        let id = format!(
            "sub-{}",
            SUBSCRIPTION_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let subscription = Subscription {
            id: id.clone(),
            topic_filter: topic_filter.clone(),
            events: VecDeque::new(),
        };

        {
            let mut subs = SUBSCRIPTIONS.lock();
            subs.insert(id.clone(), subscription);
        }

        println!("[eventbus] subscribe id={} filter={}", id, topic_filter);
        Ok(id)
    }

    fn unsubscribe(subscription_id: String) -> Result<(), String> {
        let mut subs = SUBSCRIPTIONS.lock();
        if subs.remove(&subscription_id).is_some() {
            println!("[eventbus] unsubscribe id={}", subscription_id);
            Ok(())
        } else {
            Err(format!("subscription not found: {}", subscription_id))
        }
    }

    fn poll_events(subscription_id: String, max_events: u32) -> Result<Vec<WitEvent>, String> {
        let mut subs = SUBSCRIPTIONS.lock();
        if let Some(sub) = subs.get_mut(&subscription_id) {
            let mut result = Vec::new();
            let limit = max_events.min(sub.events.len() as u32);
            for _ in 0..limit {
                if let Some(event) = sub.events.pop_front() {
                    result.push(event);
                }
            }
            Ok(result)
        } else {
            Err(format!("subscription not found: {}", subscription_id))
        }
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
