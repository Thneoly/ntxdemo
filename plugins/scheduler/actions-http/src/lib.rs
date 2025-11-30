use anyhow::{Context, Result, anyhow};
use scheduler_core::dsl::ActionDef;
use serde_json::Value as JsonValue;
use serde_yaml::Value;

// HTTP client using raw sockets (for WASM component)
pub mod http_client;

#[cfg(target_arch = "wasm32")]
pub mod component;

/// Extract URL from action definition
pub fn extract_url(action: &ActionDef) -> Result<String> {
    action
        .with
        .get("url")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("action `{}` missing `with.url`", action.id))
}

/// Extract headers from action definition
pub fn extract_headers(action: &ActionDef) -> Vec<(String, String)> {
    action
        .with
        .get("headers")
        .and_then(Value::as_mapping)
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| {
                    let key = k.as_str()?.to_string();
                    let value = v.as_str()?.to_string();
                    Some((key, value))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Extract body from action definition
pub fn extract_body(action: &ActionDef) -> Result<Option<String>> {
    let Some(body) = action.with.get("body") else {
        return Ok(None);
    };

    if let Some(raw) = body.as_str() {
        return Ok(Some(raw.to_string()));
    }

    let json_value: JsonValue = serde_yaml::from_value(body.clone()).context("body to json")?;
    let body_str = serde_json::to_string(&json_value).context("json to string")?;
    Ok(Some(body_str))
}
