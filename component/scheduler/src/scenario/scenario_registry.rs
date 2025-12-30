//! Scenario registry + workflow index
//!
//! 目标：把 `lib.rs` 中与 scenario 管理/版本化/索引相关的结构抽离出来。

use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::runtime::runtime_state::RUNTIME;
use crate::scenario::scenario_types::{NodeKind, Scenario, WaitEvent};
use crate::WorkflowIndex;

#[derive(Default)]
pub(crate) struct ScenarioRegistry {
    active_version: u64,
    next_version: u64,
    scenarios: HashMap<u64, Arc<Scenario>>,
    wf_index: HashMap<u64, WorkflowIndex>,
}

impl ScenarioRegistry {
    pub(crate) fn reset_with(&mut self, sc: Scenario) {
        self.active_version = 1;
        self.next_version = 2;
        self.scenarios.clear();
        self.wf_index.clear();
        let arc = Arc::new(sc);
        self.wf_index.insert(1, build_workflow_index(&arc));
        self.scenarios.insert(1, arc);
    }

    pub(crate) fn active(&self) -> Option<(u64, Arc<Scenario>, WorkflowIndex)> {
        let v = self.active_version;
        let sc = self.scenarios.get(&v)?.clone();
        let idx = self.wf_index.get(&v).cloned().unwrap_or_default();
        Some((v, sc, idx))
    }

    pub(crate) fn by_version(&self, v: u64) -> Option<(Arc<Scenario>, WorkflowIndex)> {
        let sc = self.scenarios.get(&v)?.clone();
        let idx = self.wf_index.get(&v).cloned().unwrap_or_default();
        Some((sc, idx))
    }

    pub(crate) fn install_new_active(&mut self, sc: Scenario) -> u64 {
        let v = self.next_version.max(1);
        self.next_version = v.saturating_add(1);
        let arc = Arc::new(sc);
        let idx = build_workflow_index(&arc);
        self.scenarios.insert(v, arc);
        self.wf_index.insert(v, idx);
        self.active_version = v;
        v
    }
}

pub(crate) static SCENARIOS: Lazy<Mutex<ScenarioRegistry>> =
    Lazy::new(|| Mutex::new(ScenarioRegistry::default()));

pub(crate) fn build_workflow_index(sc: &Scenario) -> WorkflowIndex {
    let mut idx = WorkflowIndex::default();
    for n in &sc.workflows.nodes {
        if n.kind != NodeKind::Wait {
            continue;
        }
        let Some(on) = n.on.as_ref() else {
            idx.wait_any.push(n.id.clone());
            continue;
        };
        if on.event != WaitEvent::PacketRx {
            idx.wait_any.push(n.id.clone());
            continue;
        }
        let action_id = on.r#match.action_id.clone();
        if let Some(aid) = action_id {
            idx.wait_by_action_id
                .entry(aid)
                .or_default()
                .push(n.id.clone());
        } else {
            idx.wait_any.push(n.id.clone());
        }
    }
    idx
}

pub(crate) fn get_active_scenario_ctx() -> Result<(u64, Arc<Scenario>, WorkflowIndex), String> {
    let reg = SCENARIOS
        .lock()
        .map_err(|_| "lock scenario registry".to_string())?;
    reg.active().ok_or_else(|| "no active scenario".to_string())
}

pub(crate) fn get_user_scenario_ctx(
    user_id: &str,
) -> Result<(u64, Arc<Scenario>, WorkflowIndex), String> {
    let ver = {
        let rt = RUNTIME.lock().map_err(|_| "lock runtime".to_string())?;
        rt.users
            .get(user_id)
            .map(|u| u.meta.scenario_version)
            .unwrap_or(1)
    };
    let reg = SCENARIOS
        .lock()
        .map_err(|_| "lock scenario registry".to_string())?;
    let (sc, idx) = reg
        .by_version(ver)
        .ok_or_else(|| format!("scenario version not found: {}", ver))?;
    Ok((ver, sc, idx))
}
