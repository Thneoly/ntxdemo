use indexmap::IndexMap;
use serde_yaml::Value;

use crate::dsl::{ResourceDef, Scenario};

#[derive(Debug, Clone)]
pub struct Workbook {
    pub resources: IndexMap<String, WorkbookResource>,
    pub metrics: Vec<WorkbookMetric>,
}

impl Workbook {
    pub fn from_scenario(scenario: &Scenario) -> Self {
        let resources = scenario
            .workbook
            .resources
            .iter()
            .cloned()
            .map(|resource| (resource.id.clone(), WorkbookResource { spec: resource }))
            .collect();

        let metrics = scenario
            .actions
            .actions
            .iter()
            .flat_map(|action| {
                action.export.iter().map(|export| WorkbookMetric {
                    action_id: action.id.clone(),
                    name: export.name.clone(),
                    export_type: export.export_type.clone(),
                    scope: export.scope.clone(),
                    default: export.default.clone(),
                })
            })
            .collect();

        Self { resources, metrics }
    }

    pub fn resource_count(&self) -> usize {
        self.resources.len()
    }

    pub fn metric_count(&self) -> usize {
        self.metrics.len()
    }
}

#[derive(Debug, Clone)]
pub struct WorkbookResource {
    pub spec: ResourceDef,
}

#[derive(Debug, Clone)]
pub struct WorkbookMetric {
    pub action_id: String,
    pub name: String,
    pub export_type: String,
    pub scope: Option<String>,
    pub default: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::{
        ActionDef, ActionsSection, ExportDef, ResourceDef, Scenario, WorkbookSection,
        WorkflowSection,
    };
    use indexmap::IndexMap;

    #[test]
    fn builds_resource_and_metric_indexes() {
        let scenario = Scenario {
            version: "1.0".into(),
            name: "workbook_test".into(),
            workbook: WorkbookSection {
                resources: vec![ResourceDef {
                    id: "resource".into(),
                    resource_type: "http_endpoint".into(),
                    properties: IndexMap::new(),
                }],
            },
            actions: ActionsSection {
                actions: vec![
                    ActionDef {
                        id: "probe-get".into(),
                        call: "get".into(),
                        with: IndexMap::new(),
                        export: vec![
                            ExportDef {
                                export_type: "content".into(),
                                name: "ip".into(),
                                scope: Some("workbook".into()),
                                default: Some(Value::from("127.0.0.1")),
                            },
                            ExportDef {
                                export_type: "content".into(),
                                name: "status".into(),
                                scope: None,
                                default: Some(Value::from(200)),
                            },
                        ],
                    },
                    ActionDef {
                        id: "push-post".into(),
                        call: "post".into(),
                        with: IndexMap::new(),
                        export: vec![],
                    },
                ],
            },
            workflows: WorkflowSection { nodes: vec![] },
        };

        let workbook = Workbook::from_scenario(&scenario);

        assert_eq!(workbook.resource_count(), 1);
        assert!(workbook.resources.contains_key("resource"));

        assert_eq!(workbook.metric_count(), 2);
        let names: Vec<_> = workbook
            .metrics
            .iter()
            .map(|metric| metric.name.clone())
            .collect();
        assert_eq!(names, vec!["ip".to_string(), "status".to_string()]);

        let ip_metric = &workbook.metrics[0];
        assert_eq!(ip_metric.scope.as_deref(), Some("workbook"));
        assert_eq!(
            ip_metric.default.as_ref().and_then(Value::as_str),
            Some("127.0.0.1")
        );
    }
}
