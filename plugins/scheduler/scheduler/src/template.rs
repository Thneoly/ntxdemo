use crate::core::{dsl::ActionDef, workbook::Workbook};
use indexmap::IndexMap;
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

        // First pass: collect all variables from workbook
        for (resource_id, resource) in &workbook.resources {
            for (prop, value) in &resource.spec.properties {
                if let Some(rendered) = value_to_string(value) {
                    ctx.vars
                        .insert(format!("{}.{}", resource_id, prop), rendered);
                }
            }
        }

        // Second pass: resolve variables recursively
        ctx.resolve_recursively();

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

    /// Recursively resolve all template variables in the context
    ///
    /// This method iterates through all variables and resolves any template
    /// references ({{var}}) within their values. It continues until no more
    /// substitutions can be made or a maximum iteration limit is reached to
    /// prevent infinite loops.
    ///
    /// Example:
    /// ```
    /// vars = {
    ///   "base.url": "{{base.protocol}}://{{base.host}}",
    ///   "base.protocol": "https",
    ///   "base.host": "example.com"
    /// }
    /// After resolution:
    /// vars = {
    ///   "base.url": "https://example.com",
    ///   "base.protocol": "https",
    ///   "base.host": "example.com"
    /// }
    /// ```
    fn resolve_recursively(&mut self) {
        const MAX_ITERATIONS: usize = 10;

        for _ in 0..MAX_ITERATIONS {
            let mut any_changes = false;
            let mut resolved = IndexMap::new();

            // Try to resolve each variable
            for (key, value) in &self.vars {
                let new_value = self.render_str(value);
                if &new_value != value {
                    any_changes = true;
                }
                resolved.insert(key.clone(), new_value);
            }

            // Update the variables with resolved values
            self.vars = resolved;

            // If no changes were made, we're done
            if !any_changes {
                break;
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_variable_substitution() {
        let mut ctx = TemplateContext::new();
        ctx.insert("name", "world");

        let result = ctx.render_str("Hello {{name}}!");
        assert_eq!(result, "Hello world!");
    }

    #[test]
    fn test_recursive_variable_resolution() {
        let mut ctx = TemplateContext::new();
        ctx.insert("protocol", "https");
        ctx.insert("host", "example.com");
        ctx.insert("port", "443");
        ctx.insert("base_url", "{{protocol}}://{{host}}:{{port}}");
        ctx.insert("endpoint", "{{base_url}}/api/v1");

        ctx.resolve_recursively();

        assert_eq!(ctx.vars.get("protocol"), Some(&"https".to_string()));
        assert_eq!(ctx.vars.get("host"), Some(&"example.com".to_string()));
        assert_eq!(ctx.vars.get("port"), Some(&"443".to_string()));
        assert_eq!(
            ctx.vars.get("base_url"),
            Some(&"https://example.com:443".to_string())
        );
        assert_eq!(
            ctx.vars.get("endpoint"),
            Some(&"https://example.com:443/api/v1".to_string())
        );
    }

    #[test]
    fn test_circular_reference_handling() {
        let mut ctx = TemplateContext::new();
        ctx.insert("var_a", "{{var_b}}");
        ctx.insert("var_b", "{{var_a}}");

        // Should not panic or loop infinitely
        ctx.resolve_recursively();

        // Variables should still contain references (not resolved)
        // This is expected behavior - we stop after MAX_ITERATIONS
        let var_a = ctx.vars.get("var_a").unwrap();
        let var_b = ctx.vars.get("var_b").unwrap();

        // After max iterations, they will still reference each other
        assert!(var_a.contains("{{") || var_a.contains("var"));
        assert!(var_b.contains("{{") || var_b.contains("var"));
    }

    #[test]
    fn test_multiple_references_in_one_string() {
        let mut ctx = TemplateContext::new();
        ctx.insert("first", "John");
        ctx.insert("last", "Doe");
        ctx.insert("full_name", "{{first}} {{last}}");

        ctx.resolve_recursively();

        assert_eq!(ctx.vars.get("full_name"), Some(&"John Doe".to_string()));
    }

    #[test]
    fn test_nested_recursive_resolution() {
        let mut ctx = TemplateContext::new();
        ctx.insert("a", "value_a");
        ctx.insert("b", "{{a}}_b");
        ctx.insert("c", "{{b}}_c");
        ctx.insert("d", "{{c}}_d");

        ctx.resolve_recursively();

        assert_eq!(ctx.vars.get("a"), Some(&"value_a".to_string()));
        assert_eq!(ctx.vars.get("b"), Some(&"value_a_b".to_string()));
        assert_eq!(ctx.vars.get("c"), Some(&"value_a_b_c".to_string()));
        assert_eq!(ctx.vars.get("d"), Some(&"value_a_b_c_d".to_string()));
    }
}
