use anyhow::Result;

use crate::dsl::ActionDef;
use crate::error::SchedulerError;
use crate::state_machine::StateMachine;
use crate::wbs::{WbsEdge, WbsTask, WbsTree};

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

#[derive(Debug, Clone)]
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

    pub fn failure() -> Self {
        Self {
            status: ActionStatus::Failed,
            detail: None,
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
}

#[derive(Debug, Clone)]
pub enum SchedulerEvent {
    RegisterAction(ActionDef),
    InsertTask(WbsTask),
    RemoveTask { task_id: String },
    UpdateTask { task: WbsTask },
    AddEdge { from: String, edge: WbsEdge },
    RemoveEdge { from: String, target: String },
}

impl SchedulerEvent {
    pub(crate) fn apply(
        self,
        wbs: &mut WbsTree,
        state_machine: &mut StateMachine,
    ) -> Result<(), SchedulerError> {
        match self {
            SchedulerEvent::RegisterAction(action) => {
                wbs.register_action(action);
            }
            SchedulerEvent::InsertTask(task) => {
                let task_id = task.id.clone();
                wbs.insert_task(task);
                if let Some(snapshot) = wbs.get_task(&task_id).cloned() {
                    state_machine.sync_task(&snapshot, wbs);
                }
            }
            SchedulerEvent::RemoveTask { task_id } => {
                if wbs.remove_task(&task_id).is_some() {
                    state_machine.remove_task(&task_id);
                }
            }
            SchedulerEvent::UpdateTask { task } => {
                let task_id = task.id.clone();
                let replacement = task.clone();
                wbs.update_task(&task_id, |existing| {
                    *existing = replacement.clone();
                })?;
                if let Some(snapshot) = wbs.get_task(&task_id).cloned() {
                    state_machine.sync_task(&snapshot, wbs);
                }
            }
            SchedulerEvent::AddEdge { from, edge } => {
                wbs.insert_edge(&from, edge)?;
                if let Some(snapshot) = wbs.get_task(&from).cloned() {
                    state_machine.sync_task(&snapshot, wbs);
                }
            }
            SchedulerEvent::RemoveEdge { from, target } => {
                wbs.remove_edge(&from, &target)?;
                if let Some(snapshot) = wbs.get_task(&from).cloned() {
                    state_machine.sync_task(&snapshot, wbs);
                }
            }
        }

        Ok(())
    }
}

pub struct ActionContext<'a> {
    wbs: &'a WbsTree,
    pending_events: Vec<SchedulerEvent>,
}

impl<'a> ActionContext<'a> {
    pub(crate) fn new(wbs: &'a WbsTree) -> Self {
        Self {
            wbs,
            pending_events: Vec::new(),
        }
    }

    pub(crate) fn into_events(self) -> Vec<SchedulerEvent> {
        self.pending_events
    }

    pub fn register_action(&mut self, action: ActionDef) {
        self.pending_events
            .push(SchedulerEvent::RegisterAction(action));
    }

    pub fn add_task(&mut self, task: WbsTask) {
        self.pending_events.push(SchedulerEvent::InsertTask(task));
    }

    pub fn remove_task(&mut self, task_id: &str) {
        self.pending_events.push(SchedulerEvent::RemoveTask {
            task_id: task_id.to_string(),
        });
    }

    pub fn update_task(&mut self, replacement: WbsTask) {
        self.pending_events
            .push(SchedulerEvent::UpdateTask { task: replacement });
    }

    pub fn get_task(&self, task_id: &str) -> Option<&WbsTask> {
        self.wbs.get_task(task_id)
    }

    pub fn add_edge(&mut self, from: &str, edge: WbsEdge) {
        self.pending_events.push(SchedulerEvent::AddEdge {
            from: from.to_string(),
            edge,
        });
    }

    pub fn remove_edge(&mut self, from: &str, target: &str) {
        self.pending_events.push(SchedulerEvent::RemoveEdge {
            from: from.to_string(),
            target: target.to_string(),
        });
    }
}

#[derive(Default)]
pub struct DefaultActionComponent;

impl ActionComponent for DefaultActionComponent {
    fn init(&mut self) -> Result<()> {
        Ok(())
    }

    fn do_action(
        &mut self,
        action: &ActionDef,
        _ctx: &mut ActionContext<'_>,
    ) -> Result<ActionOutcome> {
        Ok(ActionOutcome::success().with_detail(format!("call={}", action.call)))
    }

    fn release(&mut self) -> Result<()> {
        Ok(())
    }
}
