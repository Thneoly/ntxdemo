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
