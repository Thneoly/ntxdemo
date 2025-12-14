use indexmap::IndexMap;

use crate::core::dsl::{ActionDef, ResourceDef, Scenario, WorkflowNodeType};
use crate::core::error::SchedulerError;

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
