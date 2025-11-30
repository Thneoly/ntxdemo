use indexmap::IndexMap;
use scheduler_core::{dsl::ActionDef, workbook::Workbook};
use serde_yaml::{Mapping, Value};

/// Stores template key/value pairs and can render YAML values by replacing
/// `{{var}}` placeholders with their resolved string equivalents.
#[derive(Debug, Clone, Default)]
pub struct TemplateContext {
    vars: IndexMap<String, String>,
}

impl TemplateContext {
    pub fn new() -> Self {
        Self {
            vars: IndexMap::new(),
        }
    }

    pub fn from_workbook(workbook: &Workbook) -> Self {
        let mut ctx = TemplateContext::new();
        for (resource_id, resource) in &workbook.resources {
            for (prop, value) in &resource.spec.properties {
                if let Some(rendered) = value_to_string(value) {
                    ctx.vars
                        .insert(format!("{}.{}", resource_id, prop), rendered);
                }
            }
        }
        ctx
    }

    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.vars.insert(key.into(), value.into());
    }

    pub fn extend(&mut self, data: &IndexMap<String, String>) {
        for (k, v) in data {
            self.vars.insert(k.clone(), v.clone());
        }
    }

    pub fn merged(&self, overrides: &IndexMap<String, String>) -> Self {
        let mut merged = self.clone();
        merged.extend(overrides);
        merged
    }

    pub fn vars(&self) -> &IndexMap<String, String> {
        &self.vars
    }

    pub fn render_action(&self, action: &ActionDef) -> ActionDef {
        let mut cloned = action.clone();
        for value in cloned.with.values_mut() {
            *value = self.render_value(value);
        }
        cloned
    }

    pub fn render_value(&self, value: &Value) -> Value {
        match value {
            Value::String(raw) => Value::String(self.render_str(raw)),
            Value::Sequence(seq) => {
                Value::Sequence(seq.iter().map(|v| self.render_value(v)).collect())
            }
            Value::Mapping(map) => {
                let mut rendered = Mapping::new();
                for (k, v) in map {
                    let rendered_key = match k {
                        Value::String(raw) => Value::String(self.render_str(raw)),
                        other => self.render_value(other),
                    };
                    rendered.insert(rendered_key, self.render_value(v));
                }
                Value::Mapping(rendered)
            }
            _ => value.clone(),
        }
    }

    pub fn render_str(&self, input: &str) -> String {
        let mut rendered = input.to_string();
        for (key, value) in &self.vars {
            let needle = format!("{{{{{}}}}}", key);
            if rendered.contains(&needle) {
                rendered = rendered.replace(&needle, value);
            }
        }
        rendered
    }
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(s.clone()),
        Value::Sequence(_) | Value::Mapping(_) | Value::Tagged(_) => serde_yaml::to_string(value)
            .ok()
            .map(|s| s.trim().to_string()),
    }
}
