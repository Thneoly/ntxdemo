/// Template expansion utilities for Action params.
///
/// Supported format: `{{ path.to.value }}`
/// Lookup order: vars > resources > exports.

#[derive(Debug, Clone, Default)]
pub struct TemplateContext {
    pub vars: serde_json::Value,
    pub resources: serde_json::Value,
    pub exports: serde_json::Value,
}

pub fn render_value(
    value: &serde_json::Value,
    ctx: &TemplateContext,
) -> Result<serde_json::Value, String> {
    match value {
        serde_json::Value::String(s) => Ok(serde_json::Value::String(render_str(s, ctx)?)),
        serde_json::Value::Array(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            for v in arr {
                out.push(render_value(v, ctx)?);
            }
            Ok(serde_json::Value::Array(out))
        }
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                out.insert(k.clone(), render_value(v, ctx)?);
            }
            Ok(serde_json::Value::Object(out))
        }
        other => Ok(other.clone()),
    }
}

fn render_str(input: &str, ctx: &TemplateContext) -> Result<String, String> {
    // 简单占位符：{{ path.to.value }}，按 vars > resources > exports 查找
    let mut out = String::new();
    let mut rest = input;
    while let Some(start) = rest.find("{{") {
        let (prefix, rem) = rest.split_at(start);
        out.push_str(prefix);
        if let Some(end_rel) = rem.find("}}") {
            let (inside_with_brace, after) = rem.split_at(end_rel + 2);
            let key = inside_with_brace
                .trim_start_matches("{{")
                .trim_end_matches("}}")
                .trim();
            let val = lookup_path(key, ctx).unwrap_or_else(|| "".to_string());
            out.push_str(&val);
            rest = after;
        } else {
            out.push_str(rem);
            rest = "";
        }
    }
    out.push_str(rest);
    Ok(out)
}

fn lookup_path(path: &str, ctx: &TemplateContext) -> Option<String> {
    let parts: Vec<&str> = path.split('.').collect();
    for src in [&ctx.vars, &ctx.resources, &ctx.exports] {
        if let Some(val) = get_path(src, &parts) {
            return Some(val);
        }
    }
    None
}

fn get_path<'a>(val: &'a serde_json::Value, parts: &[&str]) -> Option<String> {
    let mut cur = val;
    for p in parts {
        cur = cur.get(*p)?;
    }
    match cur {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Null => Some("null".to_string()),
        other => serde_json::to_string(other).ok(),
    }
}
