pub use bytes::Bytes;
use once_cell::sync::Lazy;
use parking_lot::{Mutex, RwLock};
use serde_json::{Map, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EventId(u64);

#[derive(Debug, Clone)]
pub struct EventHeader {
    pub id: EventId,
    pub topic: String,
    pub ts_enqueue: Instant,
    pub priority: u8,
}

#[derive(Debug, Clone)]
pub struct Event {
    pub header: EventHeader,
    pub payload: Bytes,
}

pub trait EventBus: Send + Sync + 'static {
    fn publish(&self, event: Event);
    fn subscribe<F>(&self, topic: &str, handler: F)
    where
        F: Fn(&Event) + Send + Sync + 'static;
}

#[derive(Clone)]
pub struct Subscriber {
    pub id: u64,
    pub topic: String,
    pub handler: Arc<dyn Fn(&Event) + Send + Sync>,
}

const PRIORITY_LEVELS: usize = 8;

pub struct SimpleEventBus {
    counter: AtomicU64,
    subscribers: RwLock<HashMap<String, Vec<Subscriber>>>,
    wildcard: RwLock<Vec<Subscriber>>,
    queues: Mutex<Vec<VecDeque<Event>>>,
    sub_counter: AtomicU64,
    #[cfg(feature = "hot_reload")]
    resreg: parking_lot::Mutex<Option<Arc<dyn ResourceRegistryLike>>>,
}

impl SimpleEventBus {
    pub fn new() -> Arc<Self> {
        let mut qs = Vec::with_capacity(PRIORITY_LEVELS);
        for _ in 0..PRIORITY_LEVELS {
            qs.push(VecDeque::new());
        }
        Arc::new(Self {
            counter: AtomicU64::new(0),
            subscribers: RwLock::new(HashMap::new()),
            wildcard: RwLock::new(Vec::new()),
            queues: Mutex::new(qs),
            sub_counter: AtomicU64::new(0),
            #[cfg(feature = "hot_reload")]
            resreg: parking_lot::Mutex::new(None),
        })
    }

    pub fn next_id(&self) -> EventId {
        EventId(self.counter.fetch_add(1, Ordering::Relaxed) + 1)
    }

    fn deliver(&self, event: &Event) {
        if let Some(list) = self.subscribers.read().get(&event.header.topic).cloned() {
            for sub in list.iter() {
                (sub.handler)(event);
            }
        }
        if !self.wildcard.read().is_empty() {
            let wcs: Vec<Subscriber> = self.wildcard.read().iter().cloned().collect();
            for sub in wcs {
                if sub.topic.ends_with('*') {
                    let prefix = &sub.topic[..sub.topic.len() - 1];
                    if event.header.topic.starts_with(prefix) {
                        (sub.handler)(event);
                    }
                }
            }
        }
    }

    fn drain(&self) {
        loop {
            let next = {
                let mut queues = self.queues.lock();
                let mut ev: Option<Event> = None;
                for prio in 0..PRIORITY_LEVELS {
                    if let Some(e) = queues[prio].pop_front() {
                        ev = Some(e);
                        break;
                    }
                }
                ev
            };
            match next {
                Some(ev) => self.deliver(&ev),
                None => break,
            }
        }
    }

    pub const TOPIC_TICK: &str = "kernel.tick";

    pub fn publish_tick(&self, priority: u8) {
        let ev = build_event(self, Self::TOPIC_TICK, priority, Bytes::new());
        self.publish(ev);
    }
}

impl EventBus for SimpleEventBus {
    fn publish(&self, event: Event) {
        let prio = event.header.priority.min((PRIORITY_LEVELS - 1) as u8) as usize;
        {
            let mut qs = self.queues.lock();
            qs[prio].push_back(event);
        }
        self.drain();
    }

    fn subscribe<F>(&self, topic: &str, handler: F)
    where
        F: Fn(&Event) + Send + Sync + 'static,
    {
        let _ = self.subscribe_with_handle(topic, handler);
    }
}

impl SimpleEventBus {
    #[cfg(feature = "hot_reload")]
    pub fn set_resource_registry(&self, reg: Arc<dyn ResourceRegistryLike>) {
        *self.resreg.lock() = Some(reg);
    }
    pub fn subscribe_with_handle<F>(&self, topic: &str, handler: F) -> u64
    where
        F: Fn(&Event) + Send + Sync + 'static,
    {
        let id = self.sub_counter.fetch_add(1, Ordering::Relaxed) + 1;
        let sub = Subscriber {
            id,
            topic: topic.to_string(),
            handler: Arc::new(handler),
        };
        if topic.ends_with('*') {
            self.wildcard.write().push(sub);
        } else {
            let mut map = self.subscribers.write();
            map.entry(topic.to_string()).or_default().push(sub);
        }
        #[cfg(feature = "hot_reload")]
        if let Some(reg) = self.resreg.lock().as_ref() {
            // mark one subscriber attached
            reg.upsert("eventbus_sub", id, 2);
        }
        id
    }

    pub fn unsubscribe(&self, id: u64) -> bool {
        let mut removed = false;
        {
            let mut w = self.wildcard.write();
            let before = w.len();
            w.retain(|s| s.id != id);
            removed |= w.len() != before;
        }
        {
            let mut map = self.subscribers.write();
            let keys: Vec<String> = map.keys().cloned().collect();
            let mut to_remove: Vec<String> = Vec::new();
            for k in keys {
                if let Some(list) = map.get_mut(&k) {
                    let before = list.len();
                    list.retain(|s| s.id != id);
                    if list.is_empty() {
                        to_remove.push(k.clone());
                    }
                    removed |= list.len() != before;
                }
            }
            for k in to_remove {
                map.remove(&k);
            }
        }
        #[cfg(feature = "hot_reload")]
        if removed {
            if let Some(reg) = self.resreg.lock().as_ref() {
                reg.remove("eventbus_sub", id);
            }
        }
        removed
    }
}

pub fn build_event(bus: &SimpleEventBus, topic: &str, priority: u8, payload: Bytes) -> Event {
    Event {
        header: EventHeader {
            id: bus.next_id(),
            topic: topic.to_string(),
            ts_enqueue: Instant::now(),
            priority,
        },
        payload,
    }
}

// A tiny trait to avoid hard-linking to core's ResourceRegistry when building this crate standalone.
#[cfg(feature = "hot_reload")]
pub trait ResourceRegistryLike: Send + Sync {
    fn upsert(&self, ns: &'static str, id: u64, strong_count: usize);
    fn remove(&self, ns: &'static str, id: u64);
}

const SCHEDULER_EVENTS_TOPIC: &str = "scheduler.events";
const SCHEDULER_EVENT_PRIORITY: u8 = 2;

wit_bindgen::generate!({
    world: "scheduler:event-bus/event-bus-provider",
    path: ["../wit/core", "../wit/eventbus"],
    generate_all,
    debug: true,
});

use exports::scheduler::event_bus::event_bus::{
    self as wit_event_bus, Guest as EventBusGuest, SchedulerEvent as WitSchedulerEvent,
    SchedulerEventKind as WitSchedulerEventKind, WbsEdge as WitWbsEdge, WbsTask as WitWbsTask,
};
use scheduler::core_libs::types::{ActionDef as WitActionDef, ExportDef as WitExportDef};

struct EventBusComponent;

static EVENT_BUS_STATE: Lazy<EventBusState> = Lazy::new(EventBusState::new);

struct EventBusState {
    inner: Arc<SimpleEventBus>,
    scheduler_events: Mutex<VecDeque<WitSchedulerEvent>>,
}

impl EventBusState {
    fn new() -> Self {
        Self {
            inner: SimpleEventBus::new(),
            scheduler_events: Mutex::new(VecDeque::new()),
        }
    }

    fn enqueue_scheduler_event(&self, event: WitSchedulerEvent) {
        println!(
            "[EventBus] enqueue kind={:?} action={:?} task={:?}",
            event.kind,
            event.action.as_ref().map(|a| a.id.clone()),
            event.task.as_ref().map(|t| t.id.clone())
        );
        {
            let mut queue = self.scheduler_events.lock();
            queue.push_back(event.clone());
        }

        if let Ok(payload) = encode_scheduler_event(&event) {
            let event = build_event(
                &self.inner,
                SCHEDULER_EVENTS_TOPIC,
                SCHEDULER_EVENT_PRIORITY,
                payload,
            );
            self.inner.publish(event);
        }
    }

    fn drain_scheduler_events(&self, limit: u32) -> Vec<WitSchedulerEvent> {
        if limit == 0 {
            return Vec::new();
        }

        let mut drained = Vec::new();
        let mut queue = self.scheduler_events.lock();
        for _ in 0..limit as usize {
            if let Some(event) = queue.pop_front() {
                drained.push(event);
            } else {
                break;
            }
        }
        if !drained.is_empty() {
            println!(
                "[EventBus] drain limit={} -> {} event(s)",
                limit,
                drained.len()
            );
        }
        drained
    }
}

impl EventBusGuest for EventBusComponent {
    fn enqueue(event: WitSchedulerEvent) -> Result<(), String> {
        EVENT_BUS_STATE.enqueue_scheduler_event(event);
        Ok(())
    }

    fn drain(max: u32) -> Result<Vec<WitSchedulerEvent>, String> {
        Ok(EVENT_BUS_STATE.drain_scheduler_events(max))
    }
}

export!(EventBusComponent);

fn encode_scheduler_event(event: &WitSchedulerEvent) -> Result<Bytes, String> {
    let mut obj = Map::new();
    obj.insert(
        "kind".into(),
        Value::String(scheduler_event_kind_label(&event.kind).to_string()),
    );

    if let Some(action) = event.action.as_ref() {
        obj.insert("action".into(), encode_action(action));
    }

    if let Some(task) = event.task.as_ref() {
        obj.insert("task".into(), encode_task(task));
    }

    if let Some(task_id) = event.task_id.as_ref() {
        obj.insert("taskId".into(), Value::String(task_id.clone()));
    }

    if let Some(edge) = event.edge.as_ref() {
        obj.insert("edge".into(), encode_edge(edge));
    }

    if let Some(from) = event.from_id.as_ref() {
        obj.insert("fromId".into(), Value::String(from.clone()));
    }

    if let Some(target) = event.target.as_ref() {
        obj.insert("target".into(), Value::String(target.clone()));
    }

    serde_json::to_vec(&Value::Object(obj))
        .map(Bytes::from)
        .map_err(|e| format!("encode scheduler event: {e}"))
}

fn encode_action(action: &WitActionDef) -> Value {
    let mut obj = Map::new();
    obj.insert("id".into(), Value::String(action.id.clone()));
    obj.insert("call".into(), Value::String(action.call.clone()));
    match serde_json::from_str::<Value>(&action.with_params) {
        Ok(value) => {
            obj.insert("with".into(), value);
        }
        Err(_) => {
            obj.insert("with".into(), Value::String(action.with_params.clone()));
        }
    }
    obj.insert("exports".into(), encode_exports(&action.exports));
    Value::Object(obj)
}

fn encode_exports(exports: &[WitExportDef]) -> Value {
    Value::Array(
        exports
            .iter()
            .map(|export| {
                let mut obj = Map::new();
                obj.insert("type".into(), Value::String(export.export_type.clone()));
                obj.insert("name".into(), Value::String(export.name.clone()));
                if let Some(scope) = export.scope.as_ref() {
                    obj.insert("scope".into(), Value::String(scope.clone()));
                }
                if let Some(default) = export.default_value.as_ref() {
                    match serde_json::from_str::<Value>(default) {
                        Ok(value) => {
                            obj.insert("default".into(), value);
                        }
                        Err(_) => {
                            obj.insert("default".into(), Value::String(default.clone()));
                        }
                    }
                }
                Value::Object(obj)
            })
            .collect(),
    )
}

fn encode_task(task: &WitWbsTask) -> Value {
    let mut obj = Map::new();
    obj.insert("id".into(), Value::String(task.id.clone()));
    if let Some(action_id) = task.action_id.as_ref() {
        obj.insert("actionId".into(), Value::String(action_id.clone()));
    }
    obj.insert(
        "kind".into(),
        Value::String(task_kind_label(&task.kind).to_string()),
    );
    let outgoing = task.outgoing.iter().map(encode_edge).collect::<Vec<_>>();
    obj.insert("outgoing".into(), Value::Array(outgoing));
    Value::Object(obj)
}

fn encode_edge(edge: &WitWbsEdge) -> Value {
    let mut obj = Map::new();
    obj.insert("target".into(), Value::String(edge.target.clone()));
    if let Some(condition) = edge.condition.as_ref() {
        obj.insert("condition".into(), Value::String(condition.clone()));
    }
    if let Some(label) = edge.label.as_ref() {
        obj.insert("label".into(), Value::String(label.clone()));
    }
    Value::Object(obj)
}

fn scheduler_event_kind_label(kind: &WitSchedulerEventKind) -> &'static str {
    match kind {
        WitSchedulerEventKind::RegisterAction => "register-action",
        WitSchedulerEventKind::AddTask => "add-task",
        WitSchedulerEventKind::RemoveTask => "remove-task",
        WitSchedulerEventKind::UpdateTask => "update-task",
        WitSchedulerEventKind::AddEdge => "add-edge",
        WitSchedulerEventKind::RemoveEdge => "remove-edge",
    }
}

fn task_kind_label(kind: &wit_event_bus::WbsTaskKind) -> &'static str {
    match kind {
        wit_event_bus::WbsTaskKind::Action => "action",
        wit_event_bus::WbsTaskKind::End => "end",
    }
}
