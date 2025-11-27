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

pub struct ActionContext<'a> {
    wbs: &'a mut WbsTree,
    state_machine: &'a mut StateMachine,
}

impl<'a> ActionContext<'a> {
    pub(crate) fn new(wbs: &'a mut WbsTree, state_machine: &'a mut StateMachine) -> Self {
        Self { wbs, state_machine }
    }

    pub fn register_action(&mut self, action: ActionDef) {
        self.wbs.register_action(action);
    }

    pub fn add_task(&mut self, task: WbsTask) {
        let task_id = task.id.clone();
        self.wbs.insert_task(task);
        if let Some(snapshot) = self.wbs.get_task(&task_id).cloned() {
            self.state_machine.sync_task(&snapshot, self.wbs);
        }
    }

    pub fn remove_task(&mut self, task_id: &str) -> Option<WbsTask> {
        let removed = self.wbs.remove_task(task_id);
        if removed.is_some() {
            self.state_machine.remove_task(task_id);
        }
        removed
    }

    pub fn update_task<F>(&mut self, task_id: &str, updater: F) -> Result<(), SchedulerError>
    where
        F: FnOnce(&mut WbsTask),
    {
        self.wbs.update_task(task_id, updater)?;
        if let Some(snapshot) = self.wbs.get_task(task_id).cloned() {
            self.state_machine.sync_task(&snapshot, self.wbs);
        }
        Ok(())
    }

    pub fn add_edge(&mut self, from: &str, edge: WbsEdge) -> Result<(), SchedulerError> {
        self.wbs.insert_edge(from, edge)?;
        if let Some(snapshot) = self.wbs.get_task(from).cloned() {
            self.state_machine.sync_task(&snapshot, self.wbs);
        }
        Ok(())
    }

    pub fn remove_edge(&mut self, from: &str, target: &str) -> Result<(), SchedulerError> {
        self.wbs.remove_edge(from, target)?;
        if let Some(snapshot) = self.wbs.get_task(from).cloned() {
            self.state_machine.sync_task(&snapshot, self.wbs);
        }
        Ok(())
    }

    pub fn get_task(&self, task_id: &str) -> Option<&WbsTask> {
        self.wbs.get_task(task_id)
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
