use indexmap::IndexMap;

use crate::dsl::ActionDef;
use crate::wbs::{WbsTask, WbsTaskKind, WbsTree};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::{
        ActionDef, ActionsSection, Scenario, TriggerDef, WorkbookSection, WorkflowEdge,
        WorkflowNode, WorkflowNodeType, WorkflowSection,
    };
    use crate::wbs::WbsTree;
    use indexmap::IndexMap;

    const SAMPLE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../res/http_scenario.yaml"
    ));

    fn branchy_tree() -> WbsTree {
        let scenario = Scenario {
            version: "1.0".into(),
            name: "branchy_sm".into(),
            workbook: WorkbookSection::default(),
            actions: ActionsSection {
                actions: vec![ActionDef {
                    id: "action-a".into(),
                    call: "get".into(),
                    with: IndexMap::new(),
                    export: vec![],
                }],
            },
            workflows: WorkflowSection {
                nodes: vec![
                    WorkflowNode {
                        id: "start".into(),
                        node_type: WorkflowNodeType::Action,
                        action: Some("action-a".into()),
                        edges: vec![
                            WorkflowEdge {
                                to: "success".into(),
                                trigger: Some(TriggerDef {
                                    condition: Some("{{action-a.status == 200}}".into()),
                                }),
                                label: None,
                            },
                            WorkflowEdge {
                                to: "fail".into(),
                                trigger: None,
                                label: None,
                            },
                        ],
                    },
                    WorkflowNode {
                        id: "success".into(),
                        node_type: WorkflowNodeType::End,
                        action: None,
                        edges: vec![],
                    },
                    WorkflowNode {
                        id: "fail".into(),
                        node_type: WorkflowNodeType::End,
                        action: None,
                        edges: vec![],
                    },
                ],
            },
        };

        WbsTree::build(&scenario).expect("branchy tree")
    }

    #[test]
    fn builds_state_machine_from_tree() {
        let scenario = Scenario::from_yaml_str(SAMPLE).expect("parse");
        scenario.validate().expect("valid");
        let tree = WbsTree::build(&scenario).expect("tree");
        let machine = StateMachine::from_wbs(&tree);
        assert_eq!(machine.node_count(), tree.tasks.len());
        assert!(machine.transition_count() > 0);
    }

    #[test]
    fn transitions_capture_trigger_types() {
        let tree = branchy_tree();
        let machine = StateMachine::from_wbs(&tree);

        let start = machine.nodes.get("start").expect("start node");
        assert_eq!(start.transitions.len(), 2);

        match &start.transitions[0].trigger {
            Trigger::Condition(expr) => {
                assert_eq!(expr, "{{action-a.status == 200}}");
            }
            Trigger::Always => panic!("expected conditional trigger"),
        }

        match &start.transitions[1].trigger {
            Trigger::Always => {}
            other => panic!("expected unconditional trigger, got {other:?}"),
        }
    }

    #[test]
    fn dynamic_sync_updates_nodes() {
        let mut tree = branchy_tree();
        let mut machine = StateMachine::from_wbs(&tree);

        let dynamic = WbsTask {
            id: "dynamic".into(),
            action_id: Some("action-a".into()),
            kind: WbsTaskKind::Action,
            outgoing: vec![],
        };

        tree.insert_task(dynamic.clone());
        machine.sync_task(tree.get_task("dynamic").unwrap(), &tree);
        assert!(machine.nodes.contains_key("dynamic"));

        machine.remove_task("dynamic");
        assert!(!machine.nodes.contains_key("dynamic"));
    }
}
