use indexmap::IndexMap;

use crate::dsl::{ActionDef, ResourceDef, Scenario, WorkflowNodeType};
use crate::error::SchedulerError;

pub type TaskId = String;

#[derive(Debug, Clone)]
pub struct WbsTree {
    pub name: String,
    pub resources: IndexMap<String, ResourceDef>,
    pub actions: IndexMap<String, ActionDef>,
    pub tasks: IndexMap<TaskId, WbsTask>,
}

impl WbsTree {
    pub fn new_empty() -> Self {
        Self {
            name: String::new(),
            resources: IndexMap::new(),
            actions: IndexMap::new(),
            tasks: IndexMap::new(),
        }
    }

    pub fn build(scenario: &Scenario) -> Result<Self, SchedulerError> {
        let mut resources = IndexMap::new();
        for resource in &scenario.workbook.resources {
            resources.insert(resource.id.clone(), resource.clone());
        }

        let mut actions = IndexMap::new();
        for action in &scenario.actions.actions {
            actions.insert(action.id.clone(), action.clone());
        }

        let mut tasks = IndexMap::new();
        for node in &scenario.workflows.nodes {
            let kind = match node.node_type {
                WorkflowNodeType::Action => WbsTaskKind::Action,
                WorkflowNodeType::End => WbsTaskKind::End,
            };

            let outgoing = node
                .edges
                .iter()
                .map(|edge| WbsEdge {
                    target: edge.to.clone(),
                    condition: edge
                        .trigger
                        .as_ref()
                        .and_then(|trigger| trigger.condition.clone()),
                    label: edge.label.clone(),
                })
                .collect();

            tasks.insert(
                node.id.clone(),
                WbsTask {
                    id: node.id.clone(),
                    action_id: node.action.clone(),
                    kind,
                    outgoing,
                },
            );
        }

        Ok(Self {
            name: scenario.name.clone(),
            resources,
            actions,
            tasks,
        })
    }

    pub fn task_count(&self) -> usize {
        self.tasks.len()
    }

    pub fn action_task_ids(&self) -> Vec<String> {
        self.tasks
            .values()
            .filter(|task| task.action_id.is_some())
            .map(|task| task.id.clone())
            .collect()
    }

    pub fn get_task(&self, id: &str) -> Option<&WbsTask> {
        self.tasks.get(id)
    }

    pub fn insert_task(&mut self, task: WbsTask) -> Option<WbsTask> {
        self.tasks.insert(task.id.clone(), task)
    }

    pub fn remove_task(&mut self, task_id: &str) -> Option<WbsTask> {
        self.tasks.shift_remove(task_id)
    }

    pub fn update_task<F>(&mut self, task_id: &str, updater: F) -> Result<(), SchedulerError>
    where
        F: FnOnce(&mut WbsTask),
    {
        if let Some(task) = self.tasks.get_mut(task_id) {
            updater(task);
            Ok(())
        } else {
            Err(SchedulerError::TaskNotFound(task_id.to_string()))
        }
    }

    pub fn insert_edge(&mut self, task_id: &str, edge: WbsEdge) -> Result<(), SchedulerError> {
        if let Some(task) = self.tasks.get_mut(task_id) {
            task.outgoing.push(edge);
            Ok(())
        } else {
            Err(SchedulerError::TaskNotFound(task_id.to_string()))
        }
    }

    pub fn remove_edge(&mut self, task_id: &str, target: &str) -> Result<(), SchedulerError> {
        if let Some(task) = self.tasks.get_mut(task_id) {
            task.outgoing.retain(|edge| edge.target != target);
            Ok(())
        } else {
            Err(SchedulerError::TaskNotFound(task_id.to_string()))
        }
    }

    pub fn register_action(&mut self, action: ActionDef) -> Option<ActionDef> {
        self.actions.insert(action.id.clone(), action)
    }

    pub fn get_action(&self, action_id: &str) -> Option<&ActionDef> {
        self.actions.get(action_id)
    }
}

#[derive(Debug, Clone)]
pub struct WbsTask {
    pub id: TaskId,
    pub action_id: Option<String>,
    pub kind: WbsTaskKind,
    pub outgoing: Vec<WbsEdge>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WbsTaskKind {
    Action,
    End,
}

#[derive(Debug, Clone)]
pub struct WbsEdge {
    pub target: TaskId,
    pub condition: Option<String>,
    pub label: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::{
        ActionDef, ActionsSection, Scenario, TriggerDef, WorkbookSection, WorkflowEdge,
        WorkflowNode, WorkflowNodeType, WorkflowSection,
    };
    use indexmap::IndexMap;

    const SAMPLE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../res/http_scenario.yaml"
    ));

    fn branchy_scenario() -> Scenario {
        Scenario {
            version: "1.0".into(),
            name: "branchy".into(),
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
                                label: Some("ok".into()),
                            },
                            WorkflowEdge {
                                to: "retry".into(),
                                trigger: None,
                                label: Some("retry".into()),
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
                        id: "retry".into(),
                        node_type: WorkflowNodeType::End,
                        action: None,
                        edges: vec![],
                    },
                ],
            },
        }
    }

    #[test]
    fn builds_tree_from_sample() {
        let scenario = Scenario::from_yaml_str(SAMPLE).expect("parse sample");
        scenario.validate().expect("valid scenario");
        let tree = WbsTree::build(&scenario).expect("build tree");
        assert_eq!(tree.name, scenario.name);
        assert_eq!(tree.resources.len(), scenario.workbook.resources.len());
        assert_eq!(tree.actions.len(), scenario.actions.actions.len());
        assert_eq!(tree.tasks.len(), scenario.workflows.nodes.len());
    }

    #[test]
    fn preserves_edge_conditions_and_labels() {
        let scenario = branchy_scenario();
        scenario.validate().expect("branchy valid");
        let tree = WbsTree::build(&scenario).expect("build branchy");
        let start = tree.tasks.get("start").expect("start node");
        assert_eq!(start.action_id.as_deref(), Some("action-a"));
        assert_eq!(start.outgoing.len(), 2);

        let first = &start.outgoing[0];
        assert_eq!(first.target, "success");
        assert_eq!(
            first.condition.as_deref(),
            Some("{{action-a.status == 200}}")
        );
        assert_eq!(first.label.as_deref(), Some("ok"));

        let second = &start.outgoing[1];
        assert_eq!(second.target, "retry");
        assert!(second.condition.is_none());
        assert_eq!(second.label.as_deref(), Some("retry"));
    }

    #[test]
    fn supports_dynamic_task_mutations() {
        let scenario = branchy_scenario();
        let mut tree = WbsTree::build(&scenario).expect("build branchy");

        let dynamic = WbsTask {
            id: "dynamic".into(),
            action_id: Some("action-a".into()),
            kind: WbsTaskKind::Action,
            outgoing: vec![],
        };

        tree.insert_task(dynamic.clone());
        assert!(tree.get_task("dynamic").is_some());

        tree.insert_edge(
            "dynamic",
            WbsEdge {
                target: "success".into(),
                condition: None,
                label: Some("from-dynamic".into()),
            },
        )
        .expect("insert edge");

        tree.update_task("dynamic", |task| {
            task.action_id = Some("action-a".into());
        })
        .expect("update dynamic task");

        tree.remove_edge("start", "retry")
            .expect("remove existing edge");
        tree.remove_task("dynamic");
        assert!(tree.get_task("dynamic").is_none());
    }
}
