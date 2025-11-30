// Component bindings for scheduler-actions-http
#[cfg(target_arch = "wasm32")]
mod bindings {
    use crate::extract_url;
    use crate::http_client::{HttpRequest, HttpResponse};
    use indexmap::IndexMap;
    use scheduler_core::dsl::ActionDef;
    use scheduler_core::socket::{Socket, SocketAddress};
    use std::net::IpAddr;

    wit_bindgen::generate!({
        world: "http-action-component",
        path: "wit",
    });

    struct HttpActionComponentImpl;

    /// Resolve hostname to IP address (simplified version)
    /// In production, this should use proper DNS resolution
    fn resolve_hostname(host: &str, port: u16) -> Result<SocketAddress, String> {
        // For now, try to parse as IP directly or use common names
        // In real implementation, would use DNS lookup or WASI name-lookup

        // Return SocketAddress with host and port
        Ok(SocketAddress::new(host, port))
    }

    /// Execute HTTP request using core-libs socket (via Rust API)
    /// Optionally binds to a specific source IP before connecting
    fn execute_http_request(
        request: &HttpRequest,
        bind_ip: Option<IpAddr>,
    ) -> Result<HttpResponse, String> {
        // Parse URL
        let (host, port, _, is_https) = request
            .parse_url()
            .map_err(|e| format!("Failed to parse URL: {}", e))?;

        if is_https {
            return Err("HTTPS not yet supported (TLS required)".to_string());
        }

        // Create socket address
        let addr = resolve_hostname(&host, port)?;

        // Create TCP socket using scheduler-core Socket API
        let mut socket =
            Socket::tcp_v4().map_err(|e| format!("Failed to create socket: {:?}", e))?;

        // Bind to specific source IP if provided
        if let Some(source_ip) = bind_ip {
            socket
                .bind_to_ip(source_ip, 0) // Port 0 = let system choose
                .map_err(|e| format!("Failed to bind to source IP {}: {:?}", source_ip, e))?;
        }

        // Connect to server
        socket
            .connect(addr)
            .map_err(|e| format!("Failed to connect to {}:{}: {:?}", host, port, e))?;

        // Build and send HTTP request
        let request_bytes = request
            .build_request_bytes()
            .map_err(|e| format!("Failed to build request: {}", e))?;

        socket
            .send(&request_bytes)
            .map_err(|e| format!("Failed to send request: {:?}", e))?;

        // Receive response (read in chunks until connection closes or we have full response)
        let mut response_data = Vec::new();
        let mut attempts = 0;
        const MAX_ATTEMPTS: usize = 100; // Prevent infinite loop

        loop {
            attempts += 1;
            if attempts > MAX_ATTEMPTS {
                break;
            }

            match socket.recv(4096) {
                Ok(chunk) => {
                    if chunk.is_empty() {
                        break; // Connection closed
                    }
                    response_data.extend_from_slice(&chunk);

                    // Check if we have complete response (simple check for \r\n\r\n + content-length)
                    if response_data.len() > 4 {
                        if let Some(header_end) =
                            response_data.windows(4).position(|w| w == b"\r\n\r\n")
                        {
                            // Parse headers to check Content-Length
                            let header_str = String::from_utf8_lossy(&response_data[..header_end]);
                            if let Some(cl_line) = header_str
                                .lines()
                                .find(|line| line.to_lowercase().starts_with("content-length:"))
                            {
                                if let Some(len_str) = cl_line.split(':').nth(1) {
                                    if let Ok(content_len) = len_str.trim().parse::<usize>() {
                                        let expected_total = header_end + 4 + content_len;
                                        if response_data.len() >= expected_total {
                                            break; // Complete response received
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    // Check if it's just EOF/connection closed
                    if response_data.is_empty() {
                        return Err(format!("Failed to receive response: {:?}", e));
                    }
                    break;
                }
            }
        }

        // Close socket
        let _ = socket.close();

        if response_data.is_empty() {
            return Err("No response data received".to_string());
        }

        // Parse HTTP response
        HttpResponse::parse(&response_data).map_err(|e| format!("Failed to parse response: {}", e))
    }

    impl exports::scheduler::actions_http::http_component::Guest for HttpActionComponentImpl {
        fn init_component() -> Result<(), String> {
            Ok(())
        }

        fn do_http_action(
            action: exports::scheduler::actions_http::types::ActionDef,
        ) -> Result<exports::scheduler::actions_http::types::ActionOutcome, String> {
            // Convert wit action to native ActionDef
            let with_params: IndexMap<String, serde_yaml::Value> =
                serde_json::from_str(&action.with_params)
                    .map_err(|e| format!("failed to parse with-params: {}", e))?;

            let native_action = ActionDef {
                id: action.id.clone(),
                call: action.call.clone(),
                with: with_params,
                export: action
                    .exports
                    .into_iter()
                    .map(|e| scheduler_core::dsl::ExportDef {
                        export_type: e.export_type,
                        name: e.name,
                        scope: e.scope,
                        default: e.default_value.and_then(|v| serde_json::from_str(&v).ok()),
                    })
                    .collect(),
            };

            // Extract URL and headers
            let url =
                extract_url(&native_action).map_err(|e| format!("failed to extract url: {}", e))?;

            // Skip unresolved templates
            if url.contains("{{") {
                return Ok(exports::scheduler::actions_http::types::ActionOutcome {
                    status: exports::scheduler::actions_http::types::ActionStatus::Success,
                    detail: Some(format!("skip unresolved template url={}", url)),
                });
            }

            // Extract optional bind_ip parameter
            let bind_ip = native_action
                .with
                .get("bind_ip")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<IpAddr>().ok());

            // Build HTTP request
            let method = native_action.call.to_uppercase();
            let mut request = HttpRequest::new(&method, &url);

            // Add headers
            let headers = crate::extract_headers(&native_action);
            for (key, value) in headers {
                request = request.header(key, value);
            }

            // Add body if present
            if let Ok(Some(body_str)) = crate::extract_body(&native_action) {
                request = request.body(body_str.into_bytes());
            }

            // Execute HTTP request using socket (with optional source IP binding)
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
                        exports::scheduler::actions_http::types::ActionStatus::Success
                    } else {
                        exports::scheduler::actions_http::types::ActionStatus::Failed
                    };

                    Ok(exports::scheduler::actions_http::types::ActionOutcome {
                        status,
                        detail: Some(detail),
                    })
                }
                Err(err) => Ok(exports::scheduler::actions_http::types::ActionOutcome {
                    status: exports::scheduler::actions_http::types::ActionStatus::Failed,
                    detail: Some(format!("HTTP request failed: {}", err)),
                }),
            }
        }

        fn release_component() -> Result<(), String> {
            Ok(())
        }
    }

    export!(HttpActionComponentImpl);
}
