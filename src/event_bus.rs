pub use bytes::Bytes;
use parking_lot::{Mutex, RwLock};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar as StdCondvar, Mutex as StdMutex};
use std::thread::JoinHandle;
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

    worker: Worker,
}

struct Worker {
    notify: StdCondvar,
    state: StdMutex<WorkerState>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Default)]
struct WorkerState {
    stopping: bool,
}

impl SimpleEventBus {
    pub fn new() -> Arc<Self> {
        let mut qs = Vec::with_capacity(PRIORITY_LEVELS);
        for _ in 0..PRIORITY_LEVELS {
            qs.push(VecDeque::new());
        }
        let bus = Arc::new(Self {
            counter: AtomicU64::new(0),
            subscribers: RwLock::new(HashMap::new()),
            wildcard: RwLock::new(Vec::new()),
            queues: Mutex::new(qs),
            sub_counter: AtomicU64::new(0),
            worker: Worker {
                notify: StdCondvar::new(),
                state: StdMutex::new(WorkerState::default()),
                handle: Mutex::new(None),
            },
        });

        SimpleEventBus::start_worker(&bus);
        bus
    }

    fn start_worker(this: &Arc<Self>) {
        let weak = Arc::downgrade(this);
        let handle = std::thread::Builder::new()
            .name("ntx-eventbus".to_string())
            .spawn(move || {
                // Loop until stopped or bus dropped.
                loop {
                    let Some(bus) = weak.upgrade() else {
                        return;
                    };

                    // Wait until there is work or stopping.
                    let mut guard = bus
                        .worker
                        .state
                        .lock()
                        .expect("eventbus worker mutex poisoned");
                    while !guard.stopping && !bus.has_pending() {
                        guard = bus
                            .worker
                            .notify
                            .wait(guard)
                            .expect("eventbus worker mutex poisoned");
                    }
                    if guard.stopping {
                        return;
                    }
                    drop(guard);

                    // Drain pending events.
                    bus.drain();
                }
            })
            .expect("failed to spawn eventbus worker");

        *this.worker.handle.lock() = Some(handle);
    }

    fn has_pending(&self) -> bool {
        let qs = self.queues.lock();
        qs.iter().any(|q| !q.is_empty())
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

    /// Stop the background worker thread and join it.
    ///
    /// This is primarily useful in tests; typical application code can rely on `Drop`.
    pub fn shutdown(&self) {
        {
            let mut st = self
                .worker
                .state
                .lock()
                .expect("eventbus worker mutex poisoned");
            st.stopping = true;
        }
        self.worker.notify.notify_all();

        if let Some(h) = self.worker.handle.lock().take() {
            let _ = h.join();
        }
    }

    pub const TOPIC_TICK: &str = "kernel.tick";

    /// Timer fired (payload is user-defined, typically contains task/action metadata).
    pub const TOPIC_TIMER_FIRE: &str = "timer.fire";

    pub fn publish_tick(&self, priority: u8) {
        let ev = build_event(self, Self::TOPIC_TICK, priority, Bytes::new());
        self.publish(ev);
    }
}

impl Drop for SimpleEventBus {
    fn drop(&mut self) {
        // Best-effort shutdown. Joining here is okay because this is the owner.
        {
            let mut st = self
                .worker
                .state
                .lock()
                .expect("eventbus worker mutex poisoned");
            st.stopping = true;
        }
        self.worker.notify.notify_all();
        if let Some(h) = self.worker.handle.lock().take() {
            let _ = h.join();
        }
    }
}

impl EventBus for SimpleEventBus {
    fn publish(&self, event: Event) {
        let prio = event.header.priority.min((PRIORITY_LEVELS - 1) as u8) as usize;
        {
            let mut qs = self.queues.lock();
            qs[prio].push_back(event);
        }
        self.worker.notify.notify_one();
    }

    fn subscribe<F>(&self, topic: &str, handler: F)
    where
        F: Fn(&Event) + Send + Sync + 'static,
    {
        let _ = self.subscribe_with_handle(topic, handler);
    }
}

impl SimpleEventBus {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    #[test]
    fn async_drain_invokes_handler() {
        let bus = SimpleEventBus::new();

        let hit = Arc::new(AtomicUsize::new(0));
        let hit2 = hit.clone();
        bus.subscribe("t", move |_ev| {
            hit2.fetch_add(1, AtomicOrdering::SeqCst);
        });

        bus.publish(build_event(&bus, "t", 0, Bytes::new()));

        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);
        while hit.load(AtomicOrdering::SeqCst) == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        assert_eq!(hit.load(AtomicOrdering::SeqCst), 1);
        bus.shutdown();
    }

    #[test]
    fn async_drain_respects_priority_order() {
        let bus = SimpleEventBus::new();

        let seen = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
        let seen2 = seen.clone();
        bus.subscribe("p", move |ev| {
            seen2.lock().push(ev.payload.clone().to_vec());
        });

        // Publish low prio then high prio; drain should deliver high first.
        let low = build_event(&bus, "p", 7, Bytes::from_static(b"low"));
        let high = build_event(&bus, "p", 0, Bytes::from_static(b"high"));
        bus.publish(low);
        bus.publish(high);

        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);
        loop {
            let len = seen.lock().len();
            if len >= 2 {
                break;
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        let out = seen.lock();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].as_slice(), b"high");
        assert_eq!(out[1].as_slice(), b"low");

        bus.shutdown();
    }
}
