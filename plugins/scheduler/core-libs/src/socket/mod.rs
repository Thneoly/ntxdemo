#![cfg_attr(
    not(target_arch = "wasm32"),
    allow(dead_code, unused_variables, unused_imports)
)]
mod api;
/// WASM socket implementation using WASI Preview 2
///
/// This module provides socket functionality by calling WASI socket imports.
#[cfg(target_arch = "wasm32")]
mod wasi_impl;

#[cfg(not(target_arch = "wasm32"))]
mod wasi_stub;

pub use api::Socket;

#[cfg(not(target_arch = "wasm32"))]
use once_cell::sync::Lazy;
#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Mutex;

/// Socket handle type
pub type SocketHandle = u32;

/// Address family
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressFamily {
    Ipv4,
    Ipv6,
}

/// Socket protocol
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketProtocol {
    Tcp,
    Udp,
}

/// Socket address
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketAddress {
    pub host: String,
    pub port: u16,
}

impl SocketAddress {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }
}

/// Socket error types
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocketError {
    ConnectionRefused,
    ConnectionReset,
    ConnectionAborted,
    NetworkUnreachable,
    AddressInUse,
    AddressNotAvailable,
    Timeout,
    WouldBlock,
    InvalidInput,
    Other,
}

impl std::fmt::Display for SocketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SocketError::ConnectionRefused => write!(f, "Connection refused"),
            SocketError::ConnectionReset => write!(f, "Connection reset"),
            SocketError::ConnectionAborted => write!(f, "Connection aborted"),
            SocketError::NetworkUnreachable => write!(f, "Network unreachable"),
            SocketError::AddressInUse => write!(f, "Address already in use"),
            SocketError::AddressNotAvailable => write!(f, "Address not available"),
            SocketError::Timeout => write!(f, "Operation timed out"),
            SocketError::WouldBlock => write!(f, "Operation would block"),
            SocketError::InvalidInput => write!(f, "Invalid input"),
            SocketError::Other => write!(f, "Other socket error"),
        }
    }
}

impl std::error::Error for SocketError {}

/// Internal socket state for WASM
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
struct SocketInfo {
    family: AddressFamily,
    protocol: SocketProtocol,
    // In WASM environment, we'll use WASI socket handles when available
    connected: bool,
    bound: bool,
    listening: bool,
}

/// Global socket registry
#[cfg(not(target_arch = "wasm32"))]
static SOCKET_REGISTRY: Lazy<Mutex<SocketRegistry>> =
    Lazy::new(|| Mutex::new(SocketRegistry::new()));

#[cfg(not(target_arch = "wasm32"))]
struct SocketRegistry {
    next_handle: SocketHandle,
    sockets: HashMap<SocketHandle, SocketInfo>,
}

#[cfg(not(target_arch = "wasm32"))]
impl SocketRegistry {
    fn new() -> Self {
        Self {
            next_handle: 1,
            sockets: HashMap::new(),
        }
    }

    fn register(&mut self, info: SocketInfo) -> SocketHandle {
        let handle = self.next_handle;
        self.next_handle += 1;
        self.sockets.insert(handle, info);
        handle
    }

    fn get_mut(&mut self, handle: SocketHandle) -> Option<&mut SocketInfo> {
        self.sockets.get_mut(&handle)
    }

    fn remove(&mut self, handle: SocketHandle) -> Option<SocketInfo> {
        self.sockets.remove(&handle)
    }
}

/// Create a new socket
pub fn create_socket(
    family: AddressFamily,
    protocol: SocketProtocol,
) -> Result<SocketHandle, SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        wasi_impl::create_socket(family, protocol)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Fallback stub for non-WASM
        let mut registry = SOCKET_REGISTRY.lock().unwrap();
        let info = SocketInfo {
            family,
            protocol,
            connected: false,
            bound: false,
            listening: false,
        };
        Ok(registry.register(info))
    }
}

/// Connect to a remote address (TCP)
pub fn connect(handle: SocketHandle, address: SocketAddress) -> Result<(), SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        wasi_impl::connect(handle, &address.host, address.port)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut registry = SOCKET_REGISTRY.lock().unwrap();
        let info = registry.get_mut(handle).ok_or(SocketError::InvalidInput)?;

        if info.protocol != SocketProtocol::Tcp {
            return Err(SocketError::InvalidInput);
        }

        // TODO: Integrate with WASI sockets
        info.connected = true;
        Ok(())
    }
}

/// Bind socket to local address
pub fn bind(handle: SocketHandle, address: SocketAddress) -> Result<(), SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        wasi_impl::bind(handle, &address.host, address.port)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut registry = SOCKET_REGISTRY.lock().unwrap();
        let info = registry.get_mut(handle).ok_or(SocketError::InvalidInput)?;
        info.bound = true;
        Ok(())
    }
}

/// Listen for incoming connections (TCP)
pub fn listen(handle: SocketHandle, backlog: u32) -> Result<(), SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        wasi_impl::listen(handle, backlog)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut registry = SOCKET_REGISTRY.lock().unwrap();
        let info = registry.get_mut(handle).ok_or(SocketError::InvalidInput)?;

        if info.protocol != SocketProtocol::Tcp {
            return Err(SocketError::InvalidInput);
        }

        if !info.bound {
            return Err(SocketError::InvalidInput);
        }

        info.listening = true;
        Ok(())
    }
}

/// Accept an incoming connection (TCP)
pub fn accept(handle: SocketHandle) -> Result<SocketHandle, SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        wasi_impl::accept(handle).map(|(id, _, _)| id)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _registry = SOCKET_REGISTRY.lock().unwrap();
        Err(SocketError::WouldBlock)
    }
}

/// Send data through socket
pub fn send(handle: SocketHandle, data: &[u8]) -> Result<u64, SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        wasi_impl::send(handle, data)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _registry = SOCKET_REGISTRY.lock().unwrap();
        Ok(data.len() as u64)
    }
}

/// Receive data from socket
pub fn receive(handle: SocketHandle, max_len: u64) -> Result<Vec<u8>, SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        wasi_impl::receive(handle, max_len)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _registry = SOCKET_REGISTRY.lock().unwrap();
        Ok(Vec::new())
    }
}

/// Send data to specific address (UDP)
pub fn send_to(
    handle: SocketHandle,
    data: &[u8],
    address: SocketAddress,
) -> Result<u64, SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        wasi_impl::send_to(handle, data, &address.host, address.port)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _registry = SOCKET_REGISTRY.lock().unwrap();
        Ok(data.len() as u64)
    }
}

/// Receive data with sender address (UDP)
pub fn receive_from(
    handle: SocketHandle,
    max_len: u64,
) -> Result<(Vec<u8>, SocketAddress), SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        wasi_impl::receive_from(handle, max_len)
            .map(|(data, host, port)| (data, SocketAddress::new(host, port)))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _registry = SOCKET_REGISTRY.lock().unwrap();
        Ok((Vec::new(), SocketAddress::new("0.0.0.0", 0)))
    }
}

/// Close socket
pub fn close(handle: SocketHandle) -> Result<(), SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        wasi_impl::close(handle)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut registry = SOCKET_REGISTRY.lock().unwrap();
        registry.remove(handle).ok_or(SocketError::InvalidInput)?;
        Ok(())
    }
}

/// Set socket option: read timeout
pub fn set_read_timeout(handle: SocketHandle, timeout_ms: Option<u64>) -> Result<(), SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        wasi_impl::set_read_timeout(handle, timeout_ms)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _registry = SOCKET_REGISTRY.lock().unwrap();
        Ok(())
    }
}

/// Set socket option: write timeout
pub fn set_write_timeout(handle: SocketHandle, timeout_ms: Option<u64>) -> Result<(), SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        wasi_impl::set_write_timeout(handle, timeout_ms)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _registry = SOCKET_REGISTRY.lock().unwrap();
        Ok(())
    }
}

/// Set socket option: reuse address
pub fn set_reuse_address(handle: SocketHandle, reuse: bool) -> Result<(), SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        wasi_impl::set_reuse_address(handle, reuse)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _registry = SOCKET_REGISTRY.lock().unwrap();
        Ok(())
    }
}

/// Get local address of socket
pub fn get_local_address(handle: SocketHandle) -> Result<SocketAddress, SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        wasi_impl::get_local_address(handle).map(|(host, port)| SocketAddress::new(host, port))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _registry = SOCKET_REGISTRY.lock().unwrap();
        Ok(SocketAddress::new("0.0.0.0", 0))
    }
}

/// Get peer address of socket
pub fn get_peer_address(handle: SocketHandle) -> Result<SocketAddress, SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        wasi_impl::get_peer_address(handle).map(|(host, port)| SocketAddress::new(host, port))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _registry = SOCKET_REGISTRY.lock().unwrap();
        Ok(SocketAddress::new("0.0.0.0", 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tcp_socket_creation() {
        let result = create_socket(AddressFamily::Ipv4, SocketProtocol::Tcp);
        assert!(result.is_ok());
        let handle = result.unwrap();
        assert!(close(handle).is_ok());
    }

    #[test]
    fn test_udp_socket_creation() {
        let result = create_socket(AddressFamily::Ipv4, SocketProtocol::Udp);
        assert!(result.is_ok());
        let handle = result.unwrap();
        assert!(close(handle).is_ok());
    }

    #[test]
    fn test_tcp_connect() {
        let handle = create_socket(AddressFamily::Ipv4, SocketProtocol::Tcp).unwrap();
        let addr = SocketAddress::new("127.0.0.1", 8080);
        assert!(connect(handle, addr).is_ok());
        assert!(close(handle).is_ok());
    }

    #[test]
    fn test_tcp_bind_listen() {
        let handle = create_socket(AddressFamily::Ipv4, SocketProtocol::Tcp).unwrap();
        let addr = SocketAddress::new("0.0.0.0", 8080);
        assert!(bind(handle, addr).is_ok());
        assert!(listen(handle, 128).is_ok());
        assert!(close(handle).is_ok());
    }

    #[test]
    fn test_udp_send_receive() {
        let handle = create_socket(AddressFamily::Ipv4, SocketProtocol::Udp).unwrap();
        let addr = SocketAddress::new("127.0.0.1", 5000);
        let data = b"test data";
        assert!(send_to(handle, data, addr).is_ok());
        assert!(close(handle).is_ok());
    }
}
