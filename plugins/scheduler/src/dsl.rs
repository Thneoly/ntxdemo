use std::collections::HashSet;

use indexmap::IndexMap;
use serde::Deserialize;
use serde_yaml::Value;

use crate::error::SchedulerError;

pub type NodeId = String;
pub type ResourceId = String;

#[derive(Debug, Clone, Deserialize)]
pub struct Scenario {
    pub version: String,
    pub name: String,
    #[serde(default)]
    pub workbook: WorkbookSection,
    #[serde(default)]
    pub actions: ActionsSection,
    #[serde(default)]
    pub workflows: WorkflowSection,
}

impl Scenario {
    pub fn from_yaml_str(input: &str) -> Result<Self, SchedulerError> {
        let scenario: Scenario = serde_yaml::from_str(input)?;
        Ok(scenario)
    }

    pub fn validate(&self) -> Result<(), SchedulerError> {
        let action_ids: HashSet<&str> = self
            .actions
            .actions
            .iter()
            .map(|action| action.id.as_str())
            .collect();
        let node_ids: HashSet<&str> = self
            .workflows
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect();

        for node in &self.workflows.nodes {
            if let Some(action_id) = &node.action {
                if !action_ids.contains(action_id.as_str()) {
                    return Err(SchedulerError::UnknownAction {
                        action: action_id.clone(),
                        node: node.id.clone(),
                    });
                }
            }

            for edge in &node.edges {
                if !node_ids.contains(edge.to.as_str()) {
                    return Err(SchedulerError::UnknownNode(edge.to.clone()));
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct WorkbookSection {
    #[serde(default)]
    pub resources: Vec<ResourceDef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResourceDef {
    pub id: ResourceId,
    #[serde(rename = "type")]
    pub resource_type: String,
    #[serde(default)]
    pub properties: IndexMap<String, Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ActionsSection {
    #[serde(default)]
    pub actions: Vec<ActionDef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActionDef {
    pub id: String,
    pub call: String,
    #[serde(default)]
    pub with: IndexMap<String, Value>,
    #[serde(default)]
    pub export: Vec<ExportDef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExportDef {
    #[serde(rename = "type")]
    pub export_type: String,
    pub name: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub default: Option<Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct WorkflowSection {
    #[serde(default)]
    pub nodes: Vec<WorkflowNode>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowNode {
    pub id: NodeId,
    #[serde(rename = "type")]
    pub node_type: WorkflowNodeType,
    pub action: Option<String>,
    #[serde(default)]
    pub edges: Vec<WorkflowEdge>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowEdge {
    pub to: NodeId,
    #[serde(default)]
    pub trigger: Option<TriggerDef>,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TriggerDef {
    #[serde(default)]
    pub condition: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WorkflowNodeType {
    Action,
    End,
}

impl Default for WorkflowNodeType {
    fn default() -> Self {
        WorkflowNodeType::Action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;

    const SAMPLE: &str = include_str!("../res/http_scenario.yaml");

    #[test]
    fn parses_sample_scenario() {
        let scenario = Scenario::from_yaml_str(SAMPLE).expect("should parse sample");
        assert_eq!(scenario.name, "http_tri_phase_demo");
        assert!(!scenario.workbook.resources.is_empty());
        assert!(!scenario.actions.actions.is_empty());
        assert!(!scenario.workflows.nodes.is_empty());
        scenario.validate().expect("scenario should be valid");
    }

    #[test]
    fn validate_rejects_unknown_action() {
        let scenario = Scenario {
            version: "1.0".into(),
            name: "invalid_action".into(),
            workbook: WorkbookSection::default(),
            actions: ActionsSection {
                actions: vec![ActionDef {
                    id: "probe-get".into(),
                    call: "get".into(),
                    with: IndexMap::new(),
                    export: vec![],
                }],
            },
            workflows: WorkflowSection {
                nodes: vec![WorkflowNode {
                    id: "start".into(),
                    node_type: WorkflowNodeType::Action,
                    action: Some("missing".into()),
                    edges: vec![],
                }],
            },
        };
        let err = scenario
            .validate()
            .expect_err("should report missing action");
        match err {
            SchedulerError::UnknownAction { action, node } => {
                assert_eq!(action, "missing");
                assert_eq!(node, "start");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_unknown_node() {
        let scenario = Scenario {
            version: "1.0".into(),
            name: "invalid_edge".into(),
            workbook: WorkbookSection::default(),
            actions: ActionsSection {
                actions: vec![ActionDef {
                    id: "ping".into(),
                    call: "get".into(),
                    with: IndexMap::new(),
                    export: vec![],
                }],
            },
            workflows: WorkflowSection {
                nodes: vec![WorkflowNode {
                    id: "start".into(),
                    node_type: WorkflowNodeType::Action,
                    action: Some("ping".into()),
                    edges: vec![WorkflowEdge {
                        to: "unknown".into(),
                        trigger: None,
                        label: None,
                    }],
                }],
            },
        };
        let err = scenario.validate().expect_err("should report missing node");
        assert!(matches!(err, SchedulerError::UnknownNode(target) if target == "unknown"));
    }
}
