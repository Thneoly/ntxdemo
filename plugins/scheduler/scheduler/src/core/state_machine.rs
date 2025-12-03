use indexmap::IndexMap;

use crate::core::dsl::ActionDef;
use crate::core::wbs::{WbsTask, WbsTaskKind, WbsTree};

#[derive(Debug, Clone)]
pub struct StateMachine {
    pub nodes: IndexMap<String, StateNode>,
}

impl StateMachine {
    pub fn from_wbs(tree: &WbsTree) -> Self {
        let nodes = tree
            .tasks
            .values()
            .map(|task| (task.id.clone(), StateNode::from_task(task, tree)))
            .collect();

        Self { nodes }
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn transition_count(&self) -> usize {
        self.nodes.values().map(|node| node.transitions.len()).sum()
    }

    pub fn sync_task(&mut self, task: &WbsTask, tree: &WbsTree) {
        let node = StateNode::from_task(task, tree);
        self.nodes.insert(task.id.clone(), node);
    }

    pub fn remove_task(&mut self, task_id: &str) -> Option<StateNode> {
        let removed = self.nodes.shift_remove(task_id);
        if removed.is_some() {
            self.detach_target(task_id);
        }
        removed
    }

    pub fn detach_target(&mut self, target: &str) {
        for node in self.nodes.values_mut() {
            node.transitions
                .retain(|transition| transition.to != target);
        }
    }
}

#[derive(Debug, Clone)]
pub struct StateNode {
    pub id: String,
    pub kind: StateNodeKind,
    pub action: Option<ActionDef>,
    pub transitions: Vec<StateTransition>,
}

impl StateNode {
    fn from_task(task: &WbsTask, tree: &WbsTree) -> Self {
        let action = task
            .action_id
            .as_ref()
            .and_then(|action_id| tree.actions.get(action_id))
            .cloned();

        let transitions = task
            .outgoing
            .iter()
            .map(|edge| StateTransition {
                to: edge.target.clone(),
                trigger: edge
                    .condition
                    .as_ref()
                    .map(|cond| Trigger::Condition(cond.clone()))
                    .unwrap_or(Trigger::Always),
                label: edge.label.clone(),
            })
            .collect();

        Self {
            id: task.id.clone(),
            kind: StateNodeKind::from(task.kind),
            action,
            transitions,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateNodeKind {
    Action,
    End,
}

impl From<WbsTaskKind> for StateNodeKind {
    fn from(value: WbsTaskKind) -> Self {
        match value {
            WbsTaskKind::Action => StateNodeKind::Action,
            WbsTaskKind::End => StateNodeKind::End,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StateTransition {
    pub to: String,
    pub trigger: Trigger,
    pub label: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Trigger {
    Always,
    Condition(String),
}
