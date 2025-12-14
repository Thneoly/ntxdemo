use crate::LocalActionDef;
use crate::scheduler::event_bus::event_bus::{
    self as wit_event_bus, SchedulerEvent, SchedulerEventKind, WbsEdge, WbsTask, WbsTaskKind,
};
use serde_yaml::Value;

const MAX_DYNAMIC_TASKS: usize = 8;

pub fn emit_for_action(action: &LocalActionDef, success: bool) -> Result<(), String> {
    if !success {
        return Ok(());
    }

    let with = &action.with;
    let Some(Value::Sequence(dynamic)) = with.get("dynamic_tasks") else {
        return Ok(());
    };

    if dynamic.is_empty() {
        return Ok(());
    }

    let tasks = dynamic
        .iter()
        .take(MAX_DYNAMIC_TASKS)
        .filter_map(|value| value.as_mapping())
        .filter_map(|map| build_task(map).transpose())
        .collect::<Result<Vec<_>, _>>()?;

    if tasks.is_empty() {
        return Ok(());
    }

    for task in tasks {
        let event = SchedulerEvent {
            kind: SchedulerEventKind::AddTask,
            action: None,
            task: Some(task),
            task_id: None,
            edge: None,
            from_id: None,
            target: None,
        };
        wit_event_bus::enqueue(&event)
            .map_err(|e| format!("enqueue scheduler event failed: {e}"))?;
    }

    Ok(())
}

fn build_task(map: &serde_yaml::Mapping) -> Result<Option<WbsTask>, String> {
    let task_id = map
        .get(&Value::String("id".into()))
        .and_then(Value::as_str)
        .ok_or_else(|| "dynamic task missing id".to_string())?
        .to_string();

    let action_id = map
        .get(&Value::String("action_id".into()))
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    let kind = match map
        .get(&Value::String("kind".into()))
        .and_then(Value::as_str)
        .unwrap_or("action")
    {
        "action" => WbsTaskKind::Action,
        "end" => WbsTaskKind::End,
        other => {
            return Err(format!("unsupported task kind `{}`", other));
        }
    };

    let outgoing = map
        .get(&Value::String("outgoing".into()))
        .and_then(Value::as_sequence)
        .map(|seq| {
            seq.iter()
                .filter_map(|edge| edge.as_mapping())
                .filter_map(|edge| build_edge(edge).transpose())
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();

    Ok(Some(WbsTask {
        id: task_id,
        action_id,
        kind,
        outgoing,
    }))
}

fn build_edge(map: &serde_yaml::Mapping) -> Result<Option<WbsEdge>, String> {
    let target = map
        .get(&Value::String("target".into()))
        .and_then(Value::as_str)
        .ok_or_else(|| "dynamic edge missing target".to_string())?
        .to_string();

    let condition = map
        .get(&Value::String("condition".into()))
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    let label = map
        .get(&Value::String("label".into()))
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    Ok(Some(WbsEdge {
        target,
        condition,
        label,
    }))
}
