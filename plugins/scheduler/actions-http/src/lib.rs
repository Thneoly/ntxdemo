use anyhow::{Context, Result, anyhow};
use scheduler_core::dsl::ActionDef;
use scheduler_core::socket::{self, AddressFamily, SocketAddress, SocketProtocol};
use scheduler_executor::{ActionComponent, ActionContext, ActionOutcome, ActionStatus};
use serde_json::Value as JsonValue;
use serde_yaml::Value;

// HTTP client using raw sockets
pub mod http_client;

#[cfg(target_arch = "wasm32")]
pub mod component;

/// HTTP Action 组件（基于 core-libs socket）
///
/// 使用 core-libs 的 socket API 执行 HTTP 请求
pub struct HttpActionComponent {
    // No state needed for socket-based implementation
}

impl HttpActionComponent {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for HttpActionComponent {
    fn default() -> Self {
        Self::new()
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
        // 提取请求参数
        let url = extract_url(action)?;
        let headers = extract_headers(action);
        let body = extract_body(action)?;
        let bind_ip = extract_bind_ip(action);

        // 构建 HTTP 请求
        let mut http_request = http_client::HttpRequest::new(&action.call, &url);

        // 添加请求头
        for (key, value) in headers {
            http_request = http_request.header(key, value);
        }

        // 添加请求体
        if let Some(body_str) = body {
            http_request = http_request.body(body_str.into_bytes());
        }

        // 如果指定了 bind_ip，在日志中显示
        if let Some(ip) = &bind_ip {
            println!(
                "[HTTP] {} {} (bind_ip: {})",
                action.call.to_uppercase(),
                url,
                ip
            );
        } else {
            println!("[HTTP] {} {}", action.call.to_uppercase(), url);
        }

        // 发送请求（使用 core-libs socket）
        let response = send_http_request(&http_request, bind_ip.as_deref())
            .with_context(|| format!("Failed to send {} request to {}", action.call, url))?;

        let status_code = response.status_code;
        let success = response.is_success();
        let response_body = response
            .body_string()
            .unwrap_or_else(|_| format!("<binary data: {} bytes>", response.body.len()));

        let status = if success {
            ActionStatus::Success
        } else {
            ActionStatus::Failed
        };

        let detail = format!(
            "{} {} -> {} ({} bytes)",
            action.call.to_uppercase(),
            url,
            status_code,
            response.body.len()
        );

        Ok(ActionOutcome {
            status,
            detail: Some(detail),
        })
    }

    fn release(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Send HTTP request using core-libs socket
fn send_http_request(
    request: &http_client::HttpRequest,
    bind_ip: Option<&str>,
) -> Result<http_client::HttpResponse> {
    // Parse URL
    let (host, port, _path, is_https) = request.parse_url()?;

    if is_https {
        return Err(anyhow!(
            "HTTPS not yet supported in socket-based implementation"
        ));
    }

    // Create TCP socket
    let socket = socket::create_socket(AddressFamily::Ipv4, SocketProtocol::Tcp)
        .map_err(|e| anyhow!("Failed to create socket: {}", e))?;

    // Bind to specific IP if requested
    if let Some(ip_str) = bind_ip {
        let bind_addr = SocketAddress::new(ip_str, 0);
        socket::bind(socket, bind_addr)
            .map_err(|e| anyhow!("Failed to bind to {}: {}", ip_str, e))?;
    }

    // Connect to remote host
    let remote_addr = SocketAddress::new(&host, port);
    socket::connect(socket, remote_addr)
        .map_err(|e| anyhow!("Failed to connect to {}:{}: {}", host, port, e))?;

    // Send HTTP request
    let request_bytes = request.build_request_bytes()?;
    socket::send(socket, &request_bytes).map_err(|e| anyhow!("Failed to send request: {}", e))?;

    // Receive response
    let mut response_data = Vec::new();
    loop {
        match socket::receive(socket, 8192) {
            Ok(chunk) => {
                if chunk.is_empty() {
                    break;
                }
                response_data.extend_from_slice(&chunk);

                // Check if we have complete response
                if response_data.windows(4).any(|w| w == b"\r\n\r\n") {
                    // Simple check: if we have headers, we're done
                    // TODO: Properly handle Content-Length and chunked encoding
                    break;
                }
            }
            Err(e) => {
                if response_data.is_empty() {
                    return Err(anyhow!("Failed to receive response: {}", e));
                }
                break;
            }
        }
    }

    // Close socket
    let _ = socket::close(socket);

    // Parse HTTP response
    http_client::HttpResponse::parse(&response_data)
}

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

/// Extract bind_ip from action definition
pub fn extract_bind_ip(action: &ActionDef) -> Option<String> {
    action
        .with
        .get("bind_ip")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
}
