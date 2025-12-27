//! Runtime state (users/tasks/queues) and the global runtime lock.

use once_cell::sync::Lazy;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Mutex;

/// 运行态的 User/Task/Ready 队列（极简版）
#[derive(Default)]
pub struct RuntimeState {
    pub users: HashMap<String, UserInstance>,
    pub ready: ReadyQueues,
    pub paused: bool,
    pub stop: bool,
}

/// Global runtime state.
pub static RUNTIME: Lazy<Mutex<RuntimeState>> = Lazy::new(|| Mutex::new(RuntimeState::default()));

/// 多级就绪队列：按 priority 分桶，优先取更大的 priority。
#[derive(Default)]
pub struct ReadyQueues {
    pub by_prio: BTreeMap<i32, VecDeque<(String, String)>>,
}

impl ReadyQueues {
    pub fn clear(&mut self) {
        self.by_prio.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.by_prio.values().all(|q| q.is_empty())
    }

    pub fn push(&mut self, prio: i32, user_id: String, node_id: String) {
        self.by_prio
            .entry(prio)
            .or_insert_with(VecDeque::new)
            .push_back((user_id, node_id));
    }

    pub fn pop_next(&mut self) -> Option<(String, String)> {
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

    pub fn retain_user(&mut self, user_id: &str) {
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

// NOTE: more runtime structs (UserInstance/TaskRuntime/etc.) will be moved here next.
