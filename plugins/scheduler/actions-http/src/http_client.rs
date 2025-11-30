/// HTTP client implementation using core-libs socket API
/// This module provides HTTP request functionality using raw TCP sockets
/// instead of wasi-http, allowing for IP binding and custom networking.
use anyhow::{Context, Result, anyhow};
use std::collections::HashMap;

/// Simple HTTP request builder
#[derive(Debug)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub body: Option<Vec<u8>>,
}

impl HttpRequest {
    pub fn new(method: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            method: method.into().to_uppercase(),
            url: url.into(),
            headers: HashMap::new(),
            body: None,
        }
    }

    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    pub fn body(mut self, data: Vec<u8>) -> Self {
        self.body = Some(data);
        self
    }

    /// Parse URL into (host, port, path, is_https)
    pub fn parse_url(&self) -> Result<(String, u16, String, bool)> {
        let url = &self.url;

        // Check scheme
        let (is_https, rest) = if url.starts_with("https://") {
            (true, &url[8..])
        } else if url.starts_with("http://") {
            (false, &url[7..])
        } else {
            return Err(anyhow!("URL must start with http:// or https://"));
        };

        // Split host and path
        let (host_port, path) = if let Some(pos) = rest.find('/') {
            (&rest[..pos], &rest[pos..])
        } else {
            (rest, "/")
        };

        // Parse host and port
        let (host, port) = if let Some(pos) = host_port.rfind(':') {
            let h = &host_port[..pos];
            let p = host_port[pos + 1..]
                .parse::<u16>()
                .context("invalid port number")?;
            (h.to_string(), p)
        } else {
            let default_port = if is_https { 443 } else { 80 };
            (host_port.to_string(), default_port)
        };

        Ok((host, port, path.to_string(), is_https))
    }

    /// Build HTTP request as bytes
    pub fn build_request_bytes(&self) -> Result<Vec<u8>> {
        let (host, _port, path, _) = self.parse_url()?;

        let mut request = String::new();

        // Request line
        request.push_str(&format!("{} {} HTTP/1.1\r\n", self.method, path));

        // Host header (required for HTTP/1.1)
        request.push_str(&format!("Host: {}\r\n", host));

        // User headers
        for (key, value) in &self.headers {
            request.push_str(&format!("{}: {}\r\n", key, value));
        }

        // Content-Length if body present
        if let Some(ref body) = self.body {
            request.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }

        // Connection: close (simplify for now)
        request.push_str("Connection: close\r\n");

        // End of headers
        request.push_str("\r\n");

        let mut bytes = request.into_bytes();

        // Append body if present
        if let Some(ref body) = self.body {
            bytes.extend_from_slice(body);
        }

        Ok(bytes)
    }
}

/// Simple HTTP response parser
#[derive(Debug)]
pub struct HttpResponse {
    pub status_code: u16,
    pub status_text: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// Parse HTTP response from bytes
    pub fn parse(data: &[u8]) -> Result<Self> {
        // Find end of headers
        let header_end = data
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .ok_or_else(|| anyhow!("incomplete HTTP response"))?;

        let header_bytes = &data[..header_end];
        let body = data[header_end + 4..].to_vec();

        let header_str = String::from_utf8_lossy(header_bytes);
        let mut lines = header_str.lines();

        // Parse status line
        let status_line = lines.next().ok_or_else(|| anyhow!("missing status line"))?;

        let parts: Vec<&str> = status_line.splitn(3, ' ').collect();
        if parts.len() < 2 {
            return Err(anyhow!("invalid status line"));
        }

        let status_code = parts[1].parse::<u16>().context("invalid status code")?;
        let status_text = parts.get(2).unwrap_or(&"").to_string();

        // Parse headers
        let mut headers = HashMap::new();
        for line in lines {
            if let Some(pos) = line.find(':') {
                let key = line[..pos].trim().to_lowercase();
                let value = line[pos + 1..].trim().to_string();
                headers.insert(key, value);
            }
        }

        Ok(Self {
            status_code,
            status_text,
            headers,
            body,
        })
    }

    /// Check if response is successful (2xx)
    pub fn is_success(&self) -> bool {
        self.status_code >= 200 && self.status_code < 300
    }

    /// Get body as string (if UTF-8)
    pub fn body_string(&self) -> Result<String> {
        String::from_utf8(self.body.clone()).context("response body is not valid UTF-8")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_url() {
        let req = HttpRequest::new("GET", "http://example.com/path");
        let (host, port, path, is_https) = req.parse_url().unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(port, 80);
        assert_eq!(path, "/path");
        assert!(!is_https);

        let req = HttpRequest::new("GET", "https://example.com:8443/api");
        let (host, port, path, is_https) = req.parse_url().unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(port, 8443);
        assert_eq!(path, "/api");
        assert!(is_https);
    }

    #[test]
    fn test_build_request() {
        let req =
            HttpRequest::new("GET", "http://example.com/test").header("User-Agent", "TestClient");

        let bytes = req.build_request_bytes().unwrap();
        let request_str = String::from_utf8(bytes).unwrap();

        assert!(request_str.contains("GET /test HTTP/1.1"));
        assert!(request_str.contains("Host: example.com"));
        assert!(request_str.contains("User-Agent: TestClient"));
    }

    #[test]
    fn test_parse_response() {
        let response_data = b"HTTP/1.1 200 OK\r\n\
                              Content-Type: text/plain\r\n\
                              Content-Length: 5\r\n\
                              \r\n\
                              Hello";

        let resp = HttpResponse::parse(response_data).unwrap();
        assert_eq!(resp.status_code, 200);
        assert_eq!(resp.status_text, "OK");
        assert_eq!(resp.headers.get("content-type").unwrap(), "text/plain");
        assert_eq!(resp.body, b"Hello");
    }
}
