use anyhow::{Context, Result};
use serde_json as json;
use serde_yaml::Value;

use crate::component::scheduler::core_libs::types::{
    ActionDef as WitActionDef, ActionOutcome as WitActionOutcome, ActionStatus as WitActionStatus,
    ExportDef as WitExportDef,
};
use crate::core::dsl::{ActionDef, ExportDef};
use crate::{ActionOutcome, ActionStatus};
use indexmap::IndexMap;

/// Helpers for converting between scheduler Rust types and WIT component types.
///
/// Despite some call sites being HTTP-based today, this module is intentionally transport-agnostic:
/// it only deals with (de)serializing the data shapes that cross the component boundary.

pub fn to_wit_action_def(action: &ActionDef) -> Result<WitActionDef> {
    let with_params = json::to_string(&action.with)
        .context("failed to encode action.with as JSON when calling actions-executor")?;

    let exports = action
        .export
        .iter()
        .map(to_wit_export_def)
        .collect::<Result<Vec<_>>>()?;

    Ok(WitActionDef {
        id: action.id.clone(),
        call: action.call.clone(),
        with_params,
        exports,
    })
}

pub fn from_wit_outcome(outcome: WitActionOutcome) -> ActionOutcome {
    let status = match outcome.status {
        WitActionStatus::Success => ActionStatus::Success,
        WitActionStatus::Failed => ActionStatus::Failed,
    };

    ActionOutcome {
        status,
        detail: outcome.detail,
    }
}

fn to_wit_export_def(export: &ExportDef) -> Result<WitExportDef> {
    let default_value = export
        .default
        .as_ref()
        .map(yaml_value_to_string)
        .transpose()?;

    Ok(WitExportDef {
        export_type: export.export_type.clone(),
        name: export.name.clone(),
        scope: export.scope.clone(),
        default_value,
    })
}

fn yaml_value_to_string(value: &Value) -> Result<String> {
    if let Some(s) = value.as_str() {
        Ok(s.to_string())
    } else {
        json::to_string(value)
            .context("failed to encode export.default as JSON when calling actions-executor")
    }
}

pub fn from_wit_action_def(action: &WitActionDef) -> Result<ActionDef> {
    let with = if action.with_params.trim().is_empty() {
        IndexMap::new()
    } else {
        json::from_str(&action.with_params)
            .context("failed to decode ActionDef.with from event bus message")?
    };

    let export = action
        .exports
        .iter()
        .map(from_wit_export_def)
        .collect::<Result<Vec<_>>>()?;

    Ok(ActionDef {
        id: action.id.clone(),
        call: action.call.clone(),
        with,
        export,
    })
}

fn from_wit_export_def(export: &WitExportDef) -> Result<ExportDef> {
    let default = match export.default_value.as_ref() {
        Some(raw) => Some(
            json::from_str(raw)
                .context("failed to decode ExportDef.default-value from event bus message")?,
        ),
        None => None,
    };

    Ok(ExportDef {
        export_type: export.export_type.clone(),
        name: export.name.clone(),
        scope: export.scope.clone(),
        default,
    })
}
