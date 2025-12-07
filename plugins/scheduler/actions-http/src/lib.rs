pub mod http_client;

wit_bindgen::generate!({
    world: "scheduler:actions-http/http-action-component@0.1.0",
    path: ["../wit/core", "../wit/protocol"],
    generate_all,
    debug: true,
});

use crate::http_client::{HttpRequest, HttpResponse};
use indexmap::IndexMap;
use serde_json::Value as JsonValue;
use serde_yaml::Value;
use std::{net::IpAddr, time::Duration};

use crate::exports::scheduler::actions_http::http_component::Guest;
use crate::scheduler::core_libs::types::{ActionOutcome, ActionStatus};
use scheduler::core_libs::socket::{
    self as core_socket, AddressFamily, SocketAddress, SocketError, SocketProtocol,
};
use scheduler::core_libs::types::ActionDef;

struct HttpActionComponentImpl;

pub struct HttpActionComponent {
    initialized: bool,
}

impl HttpActionComponent {
    pub fn new() -> Self {
        Self { initialized: false }
    }

    pub fn init_component(&mut self) -> Result<(), String> {
        if !self.initialized {
            HttpActionComponentImpl::init_component()?;
            self.initialized = true;
        }
        Ok(())
    }

    pub fn do_http_action(&mut self, action: ActionDef) -> Result<ActionOutcome, String> {
        self.init_component()?;
        HttpActionComponentImpl::do_http_action(action)
    }

    pub fn release_component(&mut self) -> Result<(), String> {
        if self.initialized {
            HttpActionComponentImpl::release_component()?;
            self.initialized = false;
        }
        Ok(())
    }
}

impl Default for HttpActionComponent {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
struct LocalActionDef {
    id: String,
    call: String,
    with: IndexMap<String, Value>,
}

/// Lightweight TCP socket wrapper backed by scheduler-core WIT imports
struct WasiSocket(u32);

impl WasiSocket {
    fn tcp_v4() -> Result<Self, String> {
        let handle = core_socket::create_socket(AddressFamily::Ipv4, SocketProtocol::Tcp)
            .map_err(|e| socket_err("create_socket", e))?;
        Ok(Self(handle))
    }

    fn bind_ip(&self, ip: IpAddr, port: u16) -> Result<(), SocketError> {
        let addr = SocketAddress {
            host: ip.to_string(),
            port,
        };
        core_socket::bind(self.0, &addr)
    }

    fn connect(&self, host: &str, port: u16) -> Result<(), String> {
        let addr = SocketAddress {
            host: host.to_string(),
            port,
        };
        core_socket::connect(self.0, &addr).map_err(|e| socket_err("connect", e))
    }

    fn send(&self, data: &[u8]) -> Result<(), String> {
        core_socket::send(self.0, data)
            .map_err(|e| socket_err("send", e))
            .map(|_| ())
    }

    fn recv(&self, max_len: u64) -> Result<Vec<u8>, String> {
        core_socket::receive(self.0, max_len).map_err(|e| socket_err("receive", e))
    }

    fn close(&self) {
        let _ = core_socket::close(self.0);
    }
}

impl Drop for WasiSocket {
    fn drop(&mut self) {
        self.close();
    }
}

fn socket_err(op: &str, err: SocketError) -> String {
    format!("{} failed: {:?}", op, err)
}

fn parse_action_def(action: ActionDef) -> Result<LocalActionDef, String> {
    let with: IndexMap<String, Value> = serde_json::from_str(&action.with_params)
        .map_err(|e| format!("failed to parse with-params: {}", e))?;

    Ok(LocalActionDef {
        id: action.id,
        call: action.call,
        with,
    })
}

fn extract_url(action: &LocalActionDef) -> Result<String, String> {
    action
        .with
        .get("url")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .ok_or_else(|| format!("action `{}` missing `with.url`", action.id))
}

fn extract_headers(action: &LocalActionDef) -> Vec<(String, String)> {
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

fn extract_body(action: &LocalActionDef) -> Result<Option<String>, String> {
    let Some(body) = action.with.get("body") else {
        return Ok(None);
    };

    if let Some(raw) = body.as_str() {
        return Ok(Some(raw.to_string()));
    }

    let json_value: JsonValue =
        serde_yaml::from_value(body.clone()).map_err(|e| format!("body to json: {}", e))?;
    serde_json::to_string(&json_value)
        .map(Some)
        .map_err(|e| format!("json to string: {}", e))
}

fn extract_bind_ip(action: &LocalActionDef) -> Option<IpAddr> {
    action
        .with
        .get("bind_ip")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<IpAddr>().ok())
}

/// Execute HTTP request using scheduler-core socket imports
fn execute_http_request(
    request: &HttpRequest,
    bind_ip: Option<IpAddr>,
) -> Result<HttpResponse, String> {
    let (host, port, _, is_https) = request
        .parse_url()
        .map_err(|e| format!("Failed to parse URL: {}", e))?;

    if is_https {
        return Err("HTTPS not yet supported (TLS required)".to_string());
    }

    let socket = WasiSocket::tcp_v4()?;

    if let Some(ip) = bind_ip {
        match socket.bind_ip(ip, 0) {
            Ok(()) => {
                println!("[HttpAction] Bound source IP {}", ip);
            }
            Err(err) => {
                println!(
                    "[HttpAction] WARN: failed to bind {} (error={:?}). Continuing without explicit source IP.",
                    ip, err
                );
            }
        }
    }

    socket.connect(&host, port)?;

    let request_bytes = request
        .build_request_bytes()
        .map_err(|e| format!("Failed to build request: {}", e))?;
    socket.send(&request_bytes)?;

    let mut response_data = Vec::new();
    let mut header_end: Option<usize> = None;
    let mut expected_len: Option<usize> = None;

    const EMPTY_READ_RETRIES: usize = 200;
    let mut empty_reads = 0usize;

    loop {
        let chunk = socket.recv(8192)?;
        if chunk.is_empty() {
            if response_data.is_empty() && empty_reads < EMPTY_READ_RETRIES {
                empty_reads += 1;
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            break;
        }

        empty_reads = 0;
        response_data.extend_from_slice(&chunk);

        if header_end.is_none() {
            if let Some(idx) = find_header_end(&response_data) {
                header_end = Some(idx + 4);
                expected_len = content_length(&response_data[..idx]).map(|len| idx + 4 + len);
            }
        }

        if let Some(total) = expected_len {
            if response_data.len() >= total {
                break;
            }
        }
    }

    socket.close();

    if response_data.is_empty() {
        return Err("No response data received".to_string());
    }

    HttpResponse::parse(&response_data).map_err(|e| format!("Failed to parse response: {}", e))
}

fn find_header_end(data: &[u8]) -> Option<usize> {
    if data.len() < 4 {
        return None;
    }
    data.windows(4).position(|w| w == b"\r\n\r\n")
}

fn content_length(header_bytes: &[u8]) -> Option<usize> {
    let header = String::from_utf8_lossy(header_bytes);
    for line in header.lines() {
        if let Some(rest) = line
            .strip_prefix("Content-Length:")
            .or_else(|| line.strip_prefix("content-length:"))
        {
            if let Ok(value) = rest.trim().parse::<usize>() {
                return Some(value);
            }
        }
    }
    None
}

impl exports::scheduler::actions_http::http_component::Guest for HttpActionComponentImpl {
    fn init_component() -> Result<(), String> {
        Ok(())
    }

    fn do_http_action(action: ActionDef) -> Result<ActionOutcome, String> {
        println!("[HttpAction] Received action: {:?}", action);
        let action = parse_action_def(action)?;

        let url = extract_url(&action)?;
        println!(
            "[HttpAction] Executing action `{}`: {} {}",
            action.id, action.call, url
        );
        if url.contains("{{") {
            return Ok(ActionOutcome {
                status: ActionStatus::Success,
                detail: Some(format!("skip unresolved template url={}", url)),
            });
        }

        let bind_ip = extract_bind_ip(&action);
        let method = action.call.to_uppercase();
        let mut request = HttpRequest::new(&method, &url);

        for (key, value) in extract_headers(&action) {
            request = request.header(key, value);
        }

        if let Some(body) = extract_body(&action)? {
            request = request.body(body.into_bytes());
        }

        match execute_http_request(&request, bind_ip) {
            Ok(response) => {
                let detail = if response.is_success() {
                    let bind_info = bind_ip
                        .map(|ip| format!(" from_ip={}", ip))
                        .unwrap_or_default();
                    format!(
                        "{} {} status={} body_len={}{}",
                        method,
                        url,
                        response.status_code,
                        response.body.len(),
                        bind_info
                    )
                } else {
                    let body_preview = response
                        .body_string()
                        .unwrap_or_else(|_| format!("<binary {} bytes>", response.body.len()));
                    let truncated = if body_preview.len() > 200 {
                        format!("{}...", &body_preview[..200])
                    } else {
                        body_preview
                    };
                    format!(
                        "{} {} status={} body={}",
                        method, url, response.status_code, truncated
                    )
                };

                let status = if response.is_success() {
                    ActionStatus::Success
                } else {
                    ActionStatus::Failed
                };

                Ok(ActionOutcome {
                    status,
                    detail: Some(detail),
                })
            }
            Err(err) => Ok(ActionOutcome {
                status: ActionStatus::Failed,
                detail: Some(format!("HTTP request failed: {}", err)),
            }),
        }
    }

    fn release_component() -> Result<(), String> {
        Ok(())
    }
}

export!(HttpActionComponentImpl);
