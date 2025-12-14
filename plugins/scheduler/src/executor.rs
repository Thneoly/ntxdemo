use crate::core::{
    dsl::ActionDef,
    error::SchedulerError,
    state_machine::StateMachine,
    wbs::{WbsEdge, WbsTask, WbsTree},
};
use anyhow::Result;

/// Trait implemented by action executors that drive the runtime.
pub trait ActionComponent {
    fn init(&mut self) -> Result<()>;
    fn do_action(
        &mut self,
        action: &ActionDef,
        ctx: &mut ActionContext<'_>,
    ) -> Result<ActionOutcome>;
    fn release(&mut self) -> Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionStatus {
    Success,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionOutcome {
    pub status: ActionStatus,
    pub detail: Option<String>,
}

impl ActionOutcome {
    pub fn success() -> Self {
        Self {
            status: ActionStatus::Success,
            detail: None,
        }
    }

    pub fn failure(detail: impl Into<String>) -> Self {
        Self {
            status: ActionStatus::Failed,
            detail: Some(detail.into()),
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

#[derive(Debug, Clone)]
pub struct ActionTrace {
    pub task_id: String,
    pub action_id: String,
    pub status: ActionStatus,
    pub detail: Option<String>,
    pub duration_ms: u64,
}

pub struct ActionContext<'a> {
    wbs: &'a WbsTree,
    events: Vec<SchedulerEvent>,
}

impl<'a> ActionContext<'a> {
    pub fn new(wbs: &'a WbsTree) -> Self {
        Self {
            wbs,
            events: Vec::new(),
        }
    }

    pub fn register_action(&mut self, action: ActionDef) {
        self.events.push(SchedulerEvent::RegisterAction(action));
    }

    pub fn add_task(&mut self, task: WbsTask) {
        self.events.push(SchedulerEvent::AddTask(task));
    }

    pub fn remove_task(&mut self, task_id: impl Into<String>) {
        self.events.push(SchedulerEvent::RemoveTask {
            task_id: task_id.into(),
        });
    }

    pub fn update_task(&mut self, task: WbsTask) {
        self.events.push(SchedulerEvent::UpdateTask(task));
    }

    pub fn add_edge(&mut self, from_id: impl Into<String>, edge: WbsEdge) {
        self.events.push(SchedulerEvent::AddEdge {
            from_id: from_id.into(),
            edge,
        });
    }

    pub fn remove_edge(&mut self, from_id: impl Into<String>, target: impl Into<String>) {
        self.events.push(SchedulerEvent::RemoveEdge {
            from_id: from_id.into(),
            target: target.into(),
        });
    }

    pub fn get_task(&self, task_id: &str) -> Option<&'a WbsTask> {
        self.wbs.get_task(task_id)
    }

    pub fn into_events(self) -> Vec<SchedulerEvent> {
        self.events
    }
}

#[derive(Debug, Clone)]
pub enum SchedulerEvent {
    RegisterAction(ActionDef),
    AddTask(WbsTask),
    RemoveTask { task_id: String },
    UpdateTask(WbsTask),
    AddEdge { from_id: String, edge: WbsEdge },
    RemoveEdge { from_id: String, target: String },
}

impl SchedulerEvent {
    pub fn apply(
        self,
        wbs: &mut WbsTree,
        state_machine: &mut StateMachine,
    ) -> Result<(), SchedulerError> {
        match self {
            SchedulerEvent::RegisterAction(action) => {
                wbs.register_action(action);
            }
            SchedulerEvent::AddTask(task) => {
                let task_id = task.id.clone();
                wbs.insert_task(task);
                if let Some(inserted) = wbs.get_task(&task_id) {
                    state_machine.sync_task(inserted, wbs);
                }
            }
            SchedulerEvent::RemoveTask { task_id } => {
                wbs.remove_task(&task_id)
                    .ok_or_else(|| SchedulerError::TaskNotFound(task_id.clone()))?;
                state_machine.remove_task(&task_id);
            }
            SchedulerEvent::UpdateTask(task) => {
                let task_id = task.id.clone();
                wbs.update_task(&task_id, |existing| *existing = task.clone())?;
                if let Some(updated) = wbs.get_task(&task_id) {
                    state_machine.sync_task(updated, wbs);
                }
            }
            SchedulerEvent::AddEdge { from_id, edge } => {
                wbs.insert_edge(&from_id, edge)?;
                if let Some(task) = wbs.get_task(&from_id) {
                    state_machine.sync_task(task, wbs);
                }
            }
            SchedulerEvent::RemoveEdge { from_id, target } => {
                wbs.remove_edge(&from_id, &target)?;
                if let Some(task) = wbs.get_task(&from_id) {
                    state_machine.sync_task(task, wbs);
                }
            }
        }
        Ok(())
    }
}
