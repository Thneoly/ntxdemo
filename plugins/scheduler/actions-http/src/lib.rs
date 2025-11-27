use anyhow::{Context, Result, anyhow};
use scheduler_core::dsl::ActionDef;
use scheduler_executor::{ActionComponent, ActionContext, ActionOutcome};
use serde_json::Value as JsonValue;
use serde_yaml::Value;
use ureq::Agent;

/// 真实业务组件：根据 DSL action 描述发起 HTTP 请求。
pub struct HttpActionComponent {
    agent: Agent,
}

impl Default for HttpActionComponent {
    fn default() -> Self {
        let agent = Agent::new();
        Self { agent }
    }
}

impl HttpActionComponent {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ActionComponent for HttpActionComponent {
    fn init(&mut self) -> Result<()> {
        Ok(())
    }

    fn do_action(
        &mut self,
        action: &ActionDef,
        _ctx: &mut ActionContext<'_>,
    ) -> Result<ActionOutcome> {
        let method = action.call.to_uppercase();
        let url = extract_url(action)?;

        if url.contains("{{") {
            return Ok(
                ActionOutcome::success().with_detail(format!("skip unresolved template url={url}"))
            );
        }

        let mut request = self.agent.request(&method, &url);
        for (key, value) in extract_headers(action) {
            request = request.set(&key, &value);
        }

        let response = match extract_body(action)? {
            Some(body) => request.send_string(&body),
            None => request.call(),
        };

        match response {
            Ok(resp) => {
                let status = resp.status();
                let detail = format!("{} {} status={} ", method, url, status);
                Ok(ActionOutcome::success().with_detail(detail))
            }
            Err(ureq::Error::Status(code, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                Ok(ActionOutcome::failure()
                    .with_detail(format!("{} {} status={} body={}", method, url, code, body)))
            }
            Err(err) => Err(anyhow!(err)),
        }
    }

    fn release(&mut self) -> Result<()> {
        Ok(())
    }
}

/// 简易日志组件，可用于测试或不希望发真实 HTTP 的场景。
#[derive(Default)]
pub struct LoggingActionComponent;

impl ActionComponent for LoggingActionComponent {
    fn init(&mut self) -> Result<()> {
        Ok(())
    }

    fn do_action(
        &mut self,
        action: &ActionDef,
        _ctx: &mut ActionContext<'_>,
    ) -> Result<ActionOutcome> {
        Ok(ActionOutcome::success().with_detail(format!("call={} (logging only)", action.call)))
    }

    fn release(&mut self) -> Result<()> {
        Ok(())
    }
}

fn extract_url(action: &ActionDef) -> Result<String> {
    action
        .with
        .get("url")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("action `{}` missing `with.url`", action.id))
}

fn extract_headers(action: &ActionDef) -> Vec<(String, String)> {
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

fn extract_body(action: &ActionDef) -> Result<Option<String>> {
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
