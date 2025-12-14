use crate::component::scheduler::core_libs::types::ActionDef as WitActionDef;
use crate::component::scheduler::event_bus::event_bus::{
    self as wit_event_bus, SchedulerEvent as WitSchedulerEvent,
    SchedulerEventKind as WitSchedulerEventKind, WbsEdge as WitWbsEdge, WbsTask as WitWbsTask,
    WbsTaskKind as WitWbsTaskKind,
};
use crate::core::dsl::ActionDef;
use crate::core::error::SchedulerError;
use crate::core::wbs::{WbsEdge, WbsTask, WbsTaskKind};
use crate::executor::SchedulerEvent;
use crate::http_bridge::from_wit_action_def;

pub(crate) fn drain_scheduler_events(limit: u32) -> Result<Vec<SchedulerEvent>, SchedulerError> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let wit_events = wit_event_bus::drain(limit).map_err(|e| {
        SchedulerError::InvalidConfiguration(format!("event bus drain failed: {e}"))
    })?;

    wit_events.into_iter().map(convert_event).collect()
}

fn convert_event(event: WitSchedulerEvent) -> Result<SchedulerEvent, SchedulerError> {
    match event.kind {
        WitSchedulerEventKind::RegisterAction => {
            let action = event.action.ok_or_else(|| {
                SchedulerError::InvalidConfiguration(
                    "register-action event missing action payload".into(),
                )
            })?;
            let action = convert_action(action)?;
            Ok(SchedulerEvent::RegisterAction(action))
        }
        WitSchedulerEventKind::AddTask => {
            let task = event.task.ok_or_else(|| {
                SchedulerError::InvalidConfiguration("add-task event missing task payload".into())
            })?;
            Ok(SchedulerEvent::AddTask(convert_task(task)))
        }
        WitSchedulerEventKind::RemoveTask => {
            let task_id = event.task_id.ok_or_else(|| {
                SchedulerError::InvalidConfiguration("remove-task event missing task-id".into())
            })?;
            Ok(SchedulerEvent::RemoveTask { task_id })
        }
        WitSchedulerEventKind::UpdateTask => {
            let task = event.task.ok_or_else(|| {
                SchedulerError::InvalidConfiguration(
                    "update-task event missing task payload".into(),
                )
            })?;
            Ok(SchedulerEvent::UpdateTask(convert_task(task)))
        }
        WitSchedulerEventKind::AddEdge => {
            let from_id = event.from_id.ok_or_else(|| {
                SchedulerError::InvalidConfiguration("add-edge event missing from-id".into())
            })?;
            let edge = event.edge.ok_or_else(|| {
                SchedulerError::InvalidConfiguration("add-edge event missing edge payload".into())
            })?;
            Ok(SchedulerEvent::AddEdge {
                from_id,
                edge: convert_edge(edge),
            })
        }
        WitSchedulerEventKind::RemoveEdge => {
            let from_id = event.from_id.ok_or_else(|| {
                SchedulerError::InvalidConfiguration("remove-edge event missing from-id".into())
            })?;
            let target = event.target.ok_or_else(|| {
                SchedulerError::InvalidConfiguration("remove-edge event missing target".into())
            })?;
            Ok(SchedulerEvent::RemoveEdge { from_id, target })
        }
    }
}

fn convert_action(action: WitActionDef) -> Result<ActionDef, SchedulerError> {
    from_wit_action_def(&action).map_err(|e| {
        SchedulerError::InvalidConfiguration(format!("invalid action payload from event bus: {e}"))
    })
}

fn convert_task(task: WitWbsTask) -> WbsTask {
    WbsTask {
        id: task.id,
        action_id: task.action_id,
        kind: convert_task_kind(task.kind),
        outgoing: task.outgoing.into_iter().map(convert_edge).collect(),
    }
}

fn convert_edge(edge: WitWbsEdge) -> WbsEdge {
    WbsEdge {
        target: edge.target,
        condition: edge.condition,
        label: edge.label,
    }
}

fn convert_task_kind(kind: WitWbsTaskKind) -> WbsTaskKind {
    match kind {
        WitWbsTaskKind::Action => WbsTaskKind::Action,
        WitWbsTaskKind::End => WbsTaskKind::End,
    }
}
