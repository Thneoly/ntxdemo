/// WASI socket abstraction implementation
///
/// This module provides a high-level wrapper around WASI socket operations
/// for TCP and UDP network communication.
use std::collections::HashMap;
use std::sync::Mutex;

#[cfg(target_arch = "wasm32")]
use once_cell::sync::Lazy;

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

/// Internal socket state
#[derive(Debug)]
struct SocketState {
    family: AddressFamily,
    protocol: SocketProtocol,
    #[cfg(not(target_arch = "wasm32"))]
    inner: Option<std::net::TcpStream>, // Simplified for now
}

/// Socket manager to track open sockets
#[cfg(target_arch = "wasm32")]
static SOCKET_MANAGER: Lazy<Mutex<SocketManager>> = Lazy::new(|| Mutex::new(SocketManager::new()));

#[derive(Debug)]
struct SocketManager {
    next_handle: SocketHandle,
    sockets: HashMap<SocketHandle, SocketState>,
}

impl SocketManager {
    fn new() -> Self {
        Self {
            next_handle: 1,
            sockets: HashMap::new(),
        }
    }

    fn allocate_handle(&mut self) -> SocketHandle {
        let handle = self.next_handle;
        self.next_handle = self.next_handle.wrapping_add(1);
        handle
    }

    fn insert(&mut self, state: SocketState) -> SocketHandle {
        let handle = self.allocate_handle();
        self.sockets.insert(handle, state);
        handle
    }

    fn get(&self, handle: SocketHandle) -> Option<&SocketState> {
        self.sockets.get(&handle)
    }

    fn get_mut(&mut self, handle: SocketHandle) -> Option<&mut SocketState> {
        self.sockets.get_mut(&handle)
    }

    fn remove(&mut self, handle: SocketHandle) -> Option<SocketState> {
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
        let state = SocketState { family, protocol };

        let mut manager = SOCKET_MANAGER.lock().map_err(|_| SocketError::Other)?;
        Ok(manager.insert(state))
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        // For non-WASM, we could use std::net directly
        // This is a stub implementation
        Err(SocketError::Other)
    }
}

/// Connect to a remote address (TCP)
pub fn connect(socket: SocketHandle, _address: SocketAddress) -> Result<(), SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        let mut manager = SOCKET_MANAGER.lock().map_err(|_| SocketError::Other)?;

        let state = manager.get_mut(socket).ok_or(SocketError::InvalidInput)?;

        if state.protocol != SocketProtocol::Tcp {
            return Err(SocketError::InvalidInput);
        }

        // TODO: Implement actual WASI socket connection
        // For now, this is a stub
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(SocketError::Other)
    }
}

/// Bind socket to local address
pub fn bind(socket: SocketHandle, _address: SocketAddress) -> Result<(), SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        let mut manager = SOCKET_MANAGER.lock().map_err(|_| SocketError::Other)?;

        let _state = manager.get_mut(socket).ok_or(SocketError::InvalidInput)?;

        // TODO: Implement actual WASI socket bind
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(SocketError::Other)
    }
}

/// Listen for incoming connections (TCP)
pub fn listen(socket: SocketHandle, _backlog: u32) -> Result<(), SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        let mut manager = SOCKET_MANAGER.lock().map_err(|_| SocketError::Other)?;

        let state = manager.get_mut(socket).ok_or(SocketError::InvalidInput)?;

        if state.protocol != SocketProtocol::Tcp {
            return Err(SocketError::InvalidInput);
        }

        // TODO: Implement actual WASI socket listen
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(SocketError::Other)
    }
}

/// Accept an incoming connection (TCP)
pub fn accept(socket: SocketHandle) -> Result<SocketHandle, SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        let mut manager = SOCKET_MANAGER.lock().map_err(|_| SocketError::Other)?;

        let state = manager.get(socket).ok_or(SocketError::InvalidInput)?;

        if state.protocol != SocketProtocol::Tcp {
            return Err(SocketError::InvalidInput);
        }

        // TODO: Implement actual WASI socket accept
        // For now, return a dummy handle
        let new_state = SocketState {
            family: state.family,
            protocol: state.protocol,
        };
        Ok(manager.insert(new_state))
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(SocketError::Other)
    }
}

/// Send data through socket
pub fn send(socket: SocketHandle, data: &[u8]) -> Result<u64, SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        let manager = SOCKET_MANAGER.lock().map_err(|_| SocketError::Other)?;

        let _state = manager.get(socket).ok_or(SocketError::InvalidInput)?;

        // TODO: Implement actual WASI socket send
        Ok(data.len() as u64)
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(SocketError::Other)
    }
}

/// Receive data from socket
pub fn receive(socket: SocketHandle, _max_len: u64) -> Result<Vec<u8>, SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        let manager = SOCKET_MANAGER.lock().map_err(|_| SocketError::Other)?;

        let _state = manager.get(socket).ok_or(SocketError::InvalidInput)?;

        // TODO: Implement actual WASI socket receive
        Ok(Vec::new())
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(SocketError::Other)
    }
}

/// Send data to specific address (UDP)
pub fn send_to(
    socket: SocketHandle,
    data: &[u8],
    _address: SocketAddress,
) -> Result<u64, SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        let manager = SOCKET_MANAGER.lock().map_err(|_| SocketError::Other)?;

        let state = manager.get(socket).ok_or(SocketError::InvalidInput)?;

        if state.protocol != SocketProtocol::Udp {
            return Err(SocketError::InvalidInput);
        }

        // TODO: Implement actual WASI socket send_to
        Ok(data.len() as u64)
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(SocketError::Other)
    }
}

/// Receive data with sender address (UDP)
pub fn receive_from(
    socket: SocketHandle,
    _max_len: u64,
) -> Result<(Vec<u8>, SocketAddress), SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        let manager = SOCKET_MANAGER.lock().map_err(|_| SocketError::Other)?;

        let state = manager.get(socket).ok_or(SocketError::InvalidInput)?;

        if state.protocol != SocketProtocol::Udp {
            return Err(SocketError::InvalidInput);
        }

        // TODO: Implement actual WASI socket receive_from
        let dummy_addr = SocketAddress::new("0.0.0.0", 0);
        Ok((Vec::new(), dummy_addr))
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(SocketError::Other)
    }
}

/// Close socket
pub fn close(socket: SocketHandle) -> Result<(), SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        let mut manager = SOCKET_MANAGER.lock().map_err(|_| SocketError::Other)?;

        manager.remove(socket).ok_or(SocketError::InvalidInput)?;

        // TODO: Implement actual WASI socket close
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(SocketError::Other)
    }
}

/// Set read timeout
pub fn set_read_timeout(socket: SocketHandle, _timeout_ms: Option<u64>) -> Result<(), SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        let manager = SOCKET_MANAGER.lock().map_err(|_| SocketError::Other)?;

        let _state = manager.get(socket).ok_or(SocketError::InvalidInput)?;

        // TODO: Implement actual WASI socket set_read_timeout
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(SocketError::Other)
    }
}

/// Set write timeout
pub fn set_write_timeout(
    socket: SocketHandle,
    _timeout_ms: Option<u64>,
) -> Result<(), SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        let manager = SOCKET_MANAGER.lock().map_err(|_| SocketError::Other)?;

        let _state = manager.get(socket).ok_or(SocketError::InvalidInput)?;

        // TODO: Implement actual WASI socket set_write_timeout
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(SocketError::Other)
    }
}

/// Set reuse address option
pub fn set_reuse_address(socket: SocketHandle, _reuse: bool) -> Result<(), SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        let manager = SOCKET_MANAGER.lock().map_err(|_| SocketError::Other)?;

        let _state = manager.get(socket).ok_or(SocketError::InvalidInput)?;

        // TODO: Implement actual WASI socket set_reuse_address
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(SocketError::Other)
    }
}

/// Get local address of socket
pub fn get_local_address(socket: SocketHandle) -> Result<SocketAddress, SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        let manager = SOCKET_MANAGER.lock().map_err(|_| SocketError::Other)?;

        let _state = manager.get(socket).ok_or(SocketError::InvalidInput)?;

        // TODO: Implement actual WASI socket get_local_address
        Ok(SocketAddress::new("0.0.0.0", 0))
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(SocketError::Other)
    }
}

/// Get peer address of socket
pub fn get_peer_address(socket: SocketHandle) -> Result<SocketAddress, SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        let manager = SOCKET_MANAGER.lock().map_err(|_| SocketError::Other)?;

        let _state = manager.get(socket).ok_or(SocketError::InvalidInput)?;

        // TODO: Implement actual WASI socket get_peer_address
        Ok(SocketAddress::new("0.0.0.0", 0))
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(SocketError::Other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_socket_address() {
        let addr = SocketAddress::new("127.0.0.1", 8080);
        assert_eq!(addr.host, "127.0.0.1");
        assert_eq!(addr.port, 8080);
    }

    #[test]
    fn test_socket_error_display() {
        assert_eq!(
            SocketError::ConnectionRefused.to_string(),
            "Connection refused"
        );
        assert_eq!(SocketError::Timeout.to_string(), "Operation timed out");
    }
}
