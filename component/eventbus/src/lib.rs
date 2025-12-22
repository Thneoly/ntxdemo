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

export!(EventBusComponent);
