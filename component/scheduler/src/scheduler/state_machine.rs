use std::collections::{HashMap, VecDeque};

use crate::scenario::scenario_types::{NodeKind, Scenario, WaitEvent, WorkflowNodeDef};
use crate::scheduler::workflow_helpers::{
    edge_trigger_allows, find_start_nodes, node_priority, wait_match,
};
use crate::{PacketRxPayload, TaskState, WorkflowIndex};

/// StateMachine（方案B）：权威的 workflow 引擎（per-user task 状态 + 边推进）。
///
/// 约束：TaskRuntime(vars/exports/resources) 仍在 RUNTIME 内；StateMachine 只负责
/// “哪个节点在什么状态、收到什么事件后如何沿 workflow 边推进”。
#[derive(Default)]
pub(crate) struct StateMachine {
    pub(crate) users: HashMap<String, HashMap<String, TaskMeta>>, // user_id -> (node_id -> meta)
    pub(crate) history: HashMap<String, VecDeque<SmEventDigest>>, // user_id -> recent applied event digests
}

#[derive(Clone, Debug)]
pub(crate) struct TaskMeta {
    pub(crate) state: TaskState,
    pub(crate) last_update_ms: u64,
    /// action-node 内部的 step index（从 0 开始）。wait/end 节点保持 0。
    pub(crate) step_idx: u32,
}

#[derive(Debug, Clone)]
pub(crate) enum SmEffect {
    SetState {
        user_id: String,
        node_id: String,
        state: TaskState,
    },
    EnqueueReady {
        user_id: String,
        node_id: String,
        priority: i32,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum SmEvent {
    /// 初始化/重置某个 user 的 workflow 实例：Created 全量投影 + start 节点入队 Ready。
    UserReset { user_id: String },

    /// 调度器从 ready queue 取出一个 task，准备派发执行（Ready -> Running）。
    DispatchStart { user_id: String, node_id: String },

    /// 收到 packet.rx 事件后，尝试触发 wait 节点（Waiting -> Completed + edge advance）。
    PacketRx {
        user_id: String,
        action_id: String,
        task_id: String,
        payload: PacketRxPayload,
        eval_ctx: serde_json::Value,
    },

    /// 收到 scheduler.action-result 事件后，更新节点状态并按需要推进边。
    ActionResult {
        user_id: String,
        node_id: String,
        reason: String, // success/failed/timeout
        success: bool,
        should_advance: bool,
        /// success 且 node 还有下一步 action：不沿边推进，而是将自身置回 Ready 并重新入队
        continue_node: bool,
        eval_ctx: serde_json::Value,
    },

    /// 重试 timer 到期（Failed -> Ready 入队）。
    RetryTimer { user_id: String, node_id: String },

    /// timeout timer 到期：Running -> Failed（后续由 action-result(timeout) 再推进）。
    TimeoutTimer { user_id: String, node_id: String },
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct SmEventDigest {
    pub(crate) ts_ms: u64,
    pub(crate) kind: String,
    pub(crate) node_id: Option<String>,
    pub(crate) reason: Option<String>,
}

impl StateMachine {
    pub(crate) fn ensure_user(&mut self, user_id: &str) {
        self.users.entry(user_id.to_string()).or_default();
        self.history.entry(user_id.to_string()).or_default();
    }

    pub(crate) fn get_state(&self, user_id: &str, node_id: &str) -> Option<TaskState> {
        self.users
            .get(user_id)
            .and_then(|m| m.get(node_id))
            .map(|m| m.state)
    }

    pub(crate) fn get_step(&self, user_id: &str, node_id: &str) -> u32 {
        self.users
            .get(user_id)
            .and_then(|m| m.get(node_id))
            .map(|m| m.step_idx)
            .unwrap_or(0)
    }

    pub(crate) fn set_step(&mut self, user_id: &str, node_id: &str, step_idx: u32, now_ms: u64) {
        self.ensure_user(user_id);
        let m = self.users.get_mut(user_id).unwrap();
        m.entry(node_id.to_string())
            .and_modify(|x| {
                x.step_idx = step_idx;
                x.last_update_ms = now_ms;
            })
            .or_insert(TaskMeta {
                state: TaskState::Created,
                last_update_ms: now_ms,
                step_idx,
            });
    }

    pub(crate) fn set_state(
        &mut self,
        user_id: &str,
        node_id: &str,
        state: TaskState,
        now_ms: u64,
    ) {
        self.ensure_user(user_id);
        let m = self.users.get_mut(user_id).unwrap();
        m.entry(node_id.to_string())
            .and_modify(|x| {
                x.state = state;
                x.last_update_ms = now_ms;
            })
            .or_insert(TaskMeta {
                state,
                last_update_ms: now_ms,
                step_idx: 0,
            });
    }

    fn record_event(&mut self, user_id: &str, d: SmEventDigest) {
        self.ensure_user(user_id);
        let h = self.history.get_mut(user_id).unwrap();
        h.push_back(d);
        while h.len() > 256 {
            h.pop_front();
        }
    }

    fn digest(now_ms: u64, ev: &SmEvent) -> SmEventDigest {
        match ev {
            SmEvent::UserReset { .. } => SmEventDigest {
                ts_ms: now_ms,
                kind: "user.reset".to_string(),
                node_id: None,
                reason: None,
            },
            SmEvent::DispatchStart { node_id, .. } => SmEventDigest {
                ts_ms: now_ms,
                kind: "dispatch.start".to_string(),
                node_id: Some(node_id.clone()),
                reason: None,
            },
            SmEvent::PacketRx { .. } => SmEventDigest {
                ts_ms: now_ms,
                kind: crate::EventKind::PacketRx.as_str().to_string(),
                node_id: None,
                reason: None,
            },
            SmEvent::ActionResult {
                node_id, reason, ..
            } => SmEventDigest {
                ts_ms: now_ms,
                kind: "action.result".to_string(),
                node_id: Some(node_id.clone()),
                reason: Some(reason.clone()),
            },
            SmEvent::RetryTimer { node_id, .. } => SmEventDigest {
                ts_ms: now_ms,
                kind: crate::EventKind::SchedulerTimerRetry.as_str().to_string(),
                node_id: Some(node_id.clone()),
                reason: None,
            },
            SmEvent::TimeoutTimer { node_id, .. } => SmEventDigest {
                ts_ms: now_ms,
                kind: crate::EventKind::SchedulerTimerTimeout.as_str().to_string(),
                node_id: Some(node_id.clone()),
                reason: None,
            },
        }
    }

    /// 唯一入口：对状态机应用一个事件，返回需要落地到 runtime/ready-queue 的 effects。
    pub(crate) fn apply(
        &mut self,
        sc: &Scenario,
        wf_index: &WorkflowIndex,
        now_ms: u64,
        ev: SmEvent,
    ) -> Vec<SmEffect> {
        let uid = match &ev {
            SmEvent::UserReset { user_id }
            | SmEvent::DispatchStart { user_id, .. }
            | SmEvent::PacketRx { user_id, .. }
            | SmEvent::ActionResult { user_id, .. }
            | SmEvent::RetryTimer { user_id, .. }
            | SmEvent::TimeoutTimer { user_id, .. } => user_id.as_str(),
        };
        self.record_event(uid, Self::digest(now_ms, &ev));

        match ev {
            SmEvent::UserReset { user_id } => self.reset_user(sc, &user_id, now_ms),
            SmEvent::DispatchStart { user_id, node_id } => {
                if self.get_state(&user_id, &node_id) != Some(TaskState::Ready) {
                    return Vec::new();
                }
                self.set_state(&user_id, &node_id, TaskState::Running, now_ms);
                vec![SmEffect::SetState {
                    user_id,
                    node_id,
                    state: TaskState::Running,
                }]
            }
            SmEvent::PacketRx {
                user_id,
                action_id,
                task_id,
                payload,
                eval_ctx,
            } => self.on_packet_rx(
                sc, wf_index, &user_id, &action_id, &task_id, &payload, &eval_ctx, now_ms,
            ),
            SmEvent::ActionResult {
                user_id,
                node_id,
                reason,
                success,
                should_advance,
                continue_node,
                eval_ctx,
            } => self.on_action_result(
                sc,
                &user_id,
                &node_id,
                &reason,
                &eval_ctx,
                should_advance,
                continue_node,
                now_ms,
                success,
            ),
            SmEvent::RetryTimer { user_id, node_id } => {
                self.on_retry_timer(sc, &user_id, &node_id, now_ms)
            }
            SmEvent::TimeoutTimer { user_id, node_id } => {
                if self.get_state(&user_id, &node_id) != Some(TaskState::Running) {
                    return Vec::new();
                }
                self.set_state(&user_id, &node_id, TaskState::Failed, now_ms);
                vec![SmEffect::SetState {
                    user_id,
                    node_id,
                    state: TaskState::Failed,
                }]
            }
        }
    }

    fn reset_user(&mut self, sc: &Scenario, user_id: &str, now_ms: u64) -> Vec<SmEffect> {
        self.ensure_user(user_id);
        let mut eff = Vec::new();
        // all nodes Created
        for n in &sc.workflows.nodes {
            self.set_state(user_id, &n.id, TaskState::Created, now_ms);
            self.set_step(user_id, &n.id, 0, now_ms);
            eff.push(SmEffect::SetState {
                user_id: user_id.to_string(),
                node_id: n.id.clone(),
                state: TaskState::Created,
            });
        }
        // start nodes Ready
        for nid in find_start_nodes(sc) {
            self.set_state(user_id, &nid, TaskState::Ready, now_ms);
            eff.push(SmEffect::SetState {
                user_id: user_id.to_string(),
                node_id: nid.clone(),
                state: TaskState::Ready,
            });
            eff.push(SmEffect::EnqueueReady {
                user_id: user_id.to_string(),
                node_id: nid.clone(),
                priority: node_priority(sc, &nid),
            });
        }
        eff
    }

    fn on_retry_timer(
        &mut self,
        sc: &Scenario,
        user_id: &str,
        node_id: &str,
        now_ms: u64,
    ) -> Vec<SmEffect> {
        // only Failed -> Ready
        if self.get_state(user_id, node_id) != Some(TaskState::Failed) {
            return Vec::new();
        }
        self.set_state(user_id, node_id, TaskState::Ready, now_ms);
        vec![
            SmEffect::SetState {
                user_id: user_id.to_string(),
                node_id: node_id.to_string(),
                state: TaskState::Ready,
            },
            SmEffect::EnqueueReady {
                user_id: user_id.to_string(),
                node_id: node_id.to_string(),
                priority: node_priority(sc, node_id),
            },
        ]
    }

    fn on_packet_rx(
        &mut self,
        sc: &Scenario,
        wf_index: &WorkflowIndex,
        user_id: &str,
        action_id: &str,
        task_id: &str,
        p: &PacketRxPayload,
        eval_ctx: &serde_json::Value,
        now_ms: u64,
    ) -> Vec<SmEffect> {
        // 候选 wait 节点：优先按 match.action_id 命中索引，否则走 wait_any
        let mut candidates: Vec<String> = Vec::new();
        candidates.extend(wf_index.wait_any.iter().cloned());
        if !action_id.is_empty() {
            if let Some(v) = wf_index.wait_by_action_id.get(action_id) {
                candidates.extend(v.iter().cloned());
            }
        }

        let wait_nodes: Vec<String> = candidates
            .into_iter()
            .filter(|nid| {
                sc.workflows
                    .nodes
                    .iter()
                    .find(|n| &n.id == nid)
                    .map(|n| {
                        n.kind == NodeKind::Wait
                            && n.on
                                .as_ref()
                                .map(|o| o.event == WaitEvent::PacketRx)
                                .unwrap_or(false)
                            && wait_match(n.on.as_ref(), action_id, task_id, p)
                    })
                    .unwrap_or(false)
            })
            .collect();

        if wait_nodes.is_empty() {
            return Vec::new();
        }

        let mut eff = Vec::new();
        for wait_id in wait_nodes {
            if self.get_state(user_id, &wait_id) != Some(TaskState::Waiting) {
                continue;
            }
            self.set_state(user_id, &wait_id, TaskState::Completed, now_ms);
            eff.push(SmEffect::SetState {
                user_id: user_id.to_string(),
                node_id: wait_id.clone(),
                state: TaskState::Completed,
            });
            eff.extend(self.advance_edges(
                sc,
                user_id,
                &wait_id,
                crate::EventKind::PacketRx.as_str(),
                Some(eval_ctx),
                now_ms,
            ));
        }
        eff
    }

    fn on_action_result(
        &mut self,
        sc: &Scenario,
        user_id: &str,
        node_id: &str,
        reason: &str,
        eval_ctx: &serde_json::Value,
        should_advance: bool,
        continue_node: bool,
        now_ms: u64,
        success: bool,
    ) -> Vec<SmEffect> {
        // success + continue_node: Ready + re-enqueue same node
        if success && continue_node {
            self.set_state(user_id, node_id, TaskState::Ready, now_ms);
            return vec![
                SmEffect::SetState {
                    user_id: user_id.to_string(),
                    node_id: node_id.to_string(),
                    state: TaskState::Ready,
                },
                SmEffect::EnqueueReady {
                    user_id: user_id.to_string(),
                    node_id: node_id.to_string(),
                    priority: node_priority(sc, node_id),
                },
            ];
        }

        // Running -> Completed/Failed (best-effort; allow idempotent replays)
        let st = if success {
            TaskState::Completed
        } else {
            TaskState::Failed
        };
        self.set_state(user_id, node_id, st, now_ms);

        let mut eff = vec![SmEffect::SetState {
            user_id: user_id.to_string(),
            node_id: node_id.to_string(),
            state: st,
        }];

        if should_advance {
            eff.extend(self.advance_edges(sc, user_id, node_id, reason, Some(eval_ctx), now_ms));
        }
        eff
    }

    pub(crate) fn is_end_reached(&self, sc: &Scenario, user_id: &str) -> bool {
        let end_nodes: Vec<&WorkflowNodeDef> = sc
            .workflows
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::End)
            .collect();
        if end_nodes.is_empty() {
            return false;
        }
        end_nodes
            .into_iter()
            .any(|n| self.get_state(user_id, &n.id) == Some(TaskState::Completed))
    }

    fn advance_edges(
        &mut self,
        sc: &Scenario,
        user_id: &str,
        from_node_id: &str,
        reason: &str,
        eval_ctx: Option<&serde_json::Value>,
        now_ms: u64,
    ) -> Vec<SmEffect> {
        let Some(from) = sc.workflows.nodes.iter().find(|n| n.id == from_node_id) else {
            return Vec::new();
        };

        let mut eff = Vec::new();
        for e in &from.edges {
            if !edge_trigger_allows(e.trigger.as_ref(), reason, eval_ctx) {
                continue;
            }
            let Some(to) = sc.workflows.nodes.iter().find(|n| n.id == e.to) else {
                continue;
            };
            match to.kind {
                NodeKind::Wait => {
                    self.set_state(user_id, &to.id, TaskState::Waiting, now_ms);
                    eff.push(SmEffect::SetState {
                        user_id: user_id.to_string(),
                        node_id: to.id.clone(),
                        state: TaskState::Waiting,
                    });
                }
                NodeKind::Action => {
                    self.set_state(user_id, &to.id, TaskState::Ready, now_ms);
                    eff.push(SmEffect::SetState {
                        user_id: user_id.to_string(),
                        node_id: to.id.clone(),
                        state: TaskState::Ready,
                    });
                    eff.push(SmEffect::EnqueueReady {
                        user_id: user_id.to_string(),
                        node_id: to.id.clone(),
                        priority: node_priority(sc, &to.id),
                    });
                }
                NodeKind::End => {
                    self.set_state(user_id, &to.id, TaskState::Completed, now_ms);
                    eff.push(SmEffect::SetState {
                        user_id: user_id.to_string(),
                        node_id: to.id.clone(),
                        state: TaskState::Completed,
                    });
                }
            }
        }
        eff
    }
}
