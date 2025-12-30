//! 运行态（users / tasks / ready queues 等）
//!
//! 目标：把 `lib.rs` 里的 runtime 数据结构抽出去，减少入口文件体积。

use once_cell::sync::Lazy;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Mutex;

use crate::scenario::scenario_types::{UserLifetime, UserLifetimeMode};
use crate::TaskState;

/// 运行态的 User/Task/Ready 队列（极简版）
#[derive(Default)]
pub(crate) struct RuntimeState {
    pub(crate) users: HashMap<String, UserInstance>,
    pub(crate) ready: ReadyQueues, // priority queues of (user_id, node_id)
    pub(crate) paused: bool,
    pub(crate) stop: bool,
}

/// 多级就绪队列：按 priority 分桶，优先取更大的 priority。
#[derive(Default)]
pub(crate) struct ReadyQueues {
    by_prio: BTreeMap<i32, VecDeque<(String, String)>>,
}

impl ReadyQueues {
    pub(crate) fn clear(&mut self) {
        self.by_prio.clear();
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.by_prio.values().all(|q| q.is_empty())
    }

    pub(crate) fn push(&mut self, prio: i32, user_id: String, node_id: String) {
        self.by_prio
            .entry(prio)
            .or_insert_with(VecDeque::new)
            .push_back((user_id, node_id));
    }

    pub(crate) fn pop_next(&mut self) -> Option<(String, String)> {
        // pick highest priority with non-empty queue
        let prio = self
            .by_prio
            .iter()
            .rev()
            .find(|(_, q)| !q.is_empty())
            .map(|(p, _)| *p)?;
        let q = self.by_prio.get_mut(&prio)?;
        let item = q.pop_front();
        if q.is_empty() {
            self.by_prio.remove(&prio);
        }
        item
    }

    pub(crate) fn retain_user(&mut self, user_id: &str) {
        let mut empty_keys: Vec<i32> = Vec::new();
        for (p, q) in self.by_prio.iter_mut() {
            q.retain(|(uid, _)| uid != user_id);
            if q.is_empty() {
                empty_keys.push(*p);
            }
        }
        for k in empty_keys {
            self.by_prio.remove(&k);
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct UserInstance {
    pub(crate) tasks: HashMap<String, TaskRuntime>,
    pub(crate) resources: serde_json::Value,
    pub(crate) meta: UserMeta,
}

#[derive(Debug, Clone)]
pub(crate) struct UserMeta {
    pub(crate) mode: UserLifetimeMode,  // once / loop
    pub(crate) iterations: Option<u64>, // max iterations in loop mode
    pub(crate) think_ms: Option<u64>,   // optional think-time between iterations
    pub(crate) iteration: u64,          // completed iterations
    pub(crate) end_event_sent: bool,    // prevent duplicate exit events per iteration
    pub(crate) running: usize,          // current running tasks
    pub(crate) max_running: usize,      // per-user concurrency cap
    pub(crate) scenario_version: u64, // topology version bound to this user (old users do not migrate)
}

impl Default for UserMeta {
    fn default() -> Self {
        Self {
            mode: UserLifetimeMode::Once,
            iterations: None,
            think_ms: None,
            iteration: 0,
            end_event_sent: false,
            running: 0,
            max_running: 1,
            scenario_version: 1,
        }
    }
}

impl UserMeta {
    pub(crate) fn apply_lifetime(&mut self, ul: &UserLifetime) {
        self.mode = ul.mode;
        self.iterations = ul.iterations;
        self.think_ms = ul.think_time.as_deref().and_then(crate::parse_duration_ms);
        self.max_running = ul.max_concurrency.map(|v| v as usize).unwrap_or(1);
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TaskRuntime {
    pub(crate) state: TaskState,
    pub(crate) vars: serde_json::Value,
    pub(crate) exports: serde_json::Value,
}

pub(crate) static RUNTIME: Lazy<Mutex<RuntimeState>> =
    Lazy::new(|| Mutex::new(RuntimeState::default()));
