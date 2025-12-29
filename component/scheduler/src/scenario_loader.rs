//! Scenario loading/parsing/validation.
//!
//! This module exists to keep `lib.rs` smaller. It owns all the file IO and
//! serde parsing of scenario YAML/JSON, plus basic validation.

use crate::scenario_types::{Actions, Load, NodeKind, Scenario, UserResources, Workbook, Workflow};
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;

#[derive(Clone, Debug)]
pub(crate) struct ScenarioConfig {
    pub(crate) config_dir: String,
    pub(crate) workflow_raw: Option<String>,
    pub(crate) workbook_raw: Option<String>,
    pub(crate) actions_raw: Option<String>,
    pub(crate) load_raw: Option<String>,
    pub(crate) parsed: Option<Scenario>,
}

pub(crate) fn load_scenario_config(config_dir: &str) -> Result<ScenarioConfig, String> {
    let meta =
        fs::metadata(config_dir).map_err(|e| format!("check config dir {config_dir}: {e}"))?;
    if !meta.is_dir() {
        return Err(format!("config dir is not a directory: {config_dir}"));
    }

    let workflow_raw = read_optional_file(config_dir, "workflow.yaml")
        .or_else(|_| read_optional_file(config_dir, "workflow.json"))
        .ok();
    let workbook_raw = read_optional_file(config_dir, "workbook.yaml")
        .or_else(|_| read_optional_file(config_dir, "workbook.json"))
        .ok();
    let actions_raw = read_optional_file(config_dir, "actions.yaml")
        .or_else(|_| read_optional_file(config_dir, "actions.json"))
        .ok();
    let load_raw = read_optional_file(config_dir, "load.yaml")
        .or_else(|_| read_optional_file(config_dir, "load.json"))
        .ok();

    let mut cfg = ScenarioConfig {
        config_dir: config_dir.to_string(),
        workflow_raw,
        workbook_raw,
        actions_raw,
        load_raw,
        parsed: None,
    };

    cfg.parsed = parse_scenario(config_dir, &cfg)?;
    Ok(cfg)
}

pub(crate) fn log_config_summary(cfg: &ScenarioConfig) -> Result<(), String> {
    let mut buf = String::new();
    writeln!(
        &mut buf,
        "[scheduler] config summary dir={} workflow={} workbook={} actions={} load={}",
        cfg.config_dir,
        cfg.workflow_raw.as_ref().map(|s| s.len()).unwrap_or(0),
        cfg.workbook_raw.as_ref().map(|s| s.len()).unwrap_or(0),
        cfg.actions_raw.as_ref().map(|s| s.len()).unwrap_or(0),
        cfg.load_raw.as_ref().map(|s| s.len()).unwrap_or(0)
    )
    .map_err(|e| format!("format summary: {e}"))?;
    print!("{buf}");
    Ok(())
}

fn read_optional_file(dir: &str, name: &str) -> Result<String, String> {
    let path = format!("{}/{}", dir, name);
    let content = fs::read_to_string(&path).map_err(|e| format!("read {path}: {e}"))?;
    println!("[scheduler] loaded config file: {path}");
    Ok(content)
}

/// Parse scenario (prefer `scenario.yaml/json`; otherwise merge split files).
fn parse_scenario(config_dir: &str, raw: &ScenarioConfig) -> Result<Option<Scenario>, String> {
    let scenario_file_yaml = format!("{}/scenario.yaml", config_dir);
    let scenario_file_json = format!("{}/scenario.json", config_dir);
    if let Ok(content) = fs::read_to_string(&scenario_file_yaml) {
        let sc: Scenario = serde_yaml::from_str(&content)
            .or_else(|_| serde_json::from_str(&content))
            .map_err(|e| format!("parse scenario.yaml: {e}"))?;
        validate_scenario(&sc)?;
        return Ok(Some(sc));
    }
    if let Ok(content) = fs::read_to_string(&scenario_file_json) {
        let sc: Scenario =
            serde_json::from_str(&content).map_err(|e| format!("parse scenario.json: {e}"))?;
        validate_scenario(&sc)?;
        return Ok(Some(sc));
    }

    if raw.workflow_raw.is_none() && raw.workbook_raw.is_none() && raw.actions_raw.is_none() {
        return Ok(None);
    }

    let workflow: Workflow = parse_piece(raw.workflow_raw.as_ref(), "workflow")?;
    let workbook: Workbook = parse_piece(raw.workbook_raw.as_ref(), "workbook")?;
    let actions: Actions = parse_piece(raw.actions_raw.as_ref(), "actions")?;
    let load: Load = parse_piece(raw.load_raw.as_ref(), "load").unwrap_or_default();
    let user_resources: UserResources = parse_piece(None, "user_resources").unwrap_or_default();

    let sc = Scenario {
        workbook,
        actions,
        workflows: workflow,
        load,
        user_resources,
    };
    validate_scenario(&sc)?;
    Ok(Some(sc))
}

fn parse_piece<T: for<'de> Deserialize<'de> + Default>(
    raw: Option<&String>,
    name: &str,
) -> Result<T, String> {
    if let Some(text) = raw {
        serde_yaml::from_str::<T>(text)
            .or_else(|_| serde_json::from_str::<T>(text))
            .map_err(|e| format!("parse {name}: {e}"))
    } else {
        Ok(T::default())
    }
}

pub(crate) fn validate_scenario(sc: &Scenario) -> Result<(), String> {
    let mut action_ids = HashMap::new();
    for a in &sc.actions.actions {
        if action_ids.insert(&a.id, ()).is_some() {
            return Err(format!("duplicate action id: {}", a.id));
        }
    }

    let mut resource_ids = HashMap::new();
    for r in &sc.workbook.resources {
        if resource_ids.insert(&r.id, ()).is_some() {
            return Err(format!("duplicate resource id: {}", r.id));
        }
    }

    for n in &sc.workflows.nodes {
        if action_ids.is_empty() && n.action.is_some() {
            // allow empty
        }
    }

    let mut node_ids = HashMap::new();
    for n in &sc.workflows.nodes {
        if node_ids.insert(&n.id, ()).is_some() {
            return Err(format!("duplicate workflow node id: {}", n.id));
        }
        if n.kind == NodeKind::Action {
            let has_steps = n.steps.as_ref().map(|v| !v.is_empty()).unwrap_or(false);
            if has_steps {
                for st in n.steps.as_ref().unwrap() {
                    if !action_ids.contains_key(&st.action) {
                        return Err(format!(
                            "workflow node {} references unknown action {}",
                            n.id, st.action
                        ));
                    }
                }
            } else {
                let has_actions = n.actions.as_ref().map(|v| !v.is_empty()).unwrap_or(false);
                if has_actions {
                    for aid in n.actions.as_ref().unwrap() {
                        if !action_ids.contains_key(aid) {
                            return Err(format!(
                                "workflow node {} references unknown action {}",
                                n.id, aid
                            ));
                        }
                    }
                } else if let Some(action_id) = &n.action {
                    if !action_ids.contains_key(action_id) {
                        return Err(format!(
                            "workflow node {} references unknown action {}",
                            n.id, action_id
                        ));
                    }
                } else {
                    return Err(format!(
                        "workflow node {} is type=action but missing steps/actions/action",
                        n.id
                    ));
                }
            }
        } else if let Some(action_id) = &n.action {
            // allow legacy/extra fields, but validate if provided
            if !action_ids.contains_key(action_id) {
                return Err(format!(
                    "workflow node {} references unknown action {}",
                    n.id, action_id
                ));
            }
        }
    }
    for n in &sc.workflows.nodes {
        for e in &n.edges {
            if !node_ids.contains_key(&e.to) {
                return Err(format!(
                    "workflow edge from {} to missing node {}",
                    n.id, e.to
                ));
            }
        }
    }

    // Note: ip_binding.pool_id is a host-side resource pool *name* (e.g. "default"),
    // not a workbook resource id. We only do best-effort validation here.
    Ok(())
}
