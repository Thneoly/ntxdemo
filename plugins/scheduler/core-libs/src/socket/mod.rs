mod api;

mod wasi_impl;

pub use api::Socket;

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
/// Create a new socket
pub fn create_socket(
    family: AddressFamily,
    protocol: SocketProtocol,
) -> Result<SocketHandle, SocketError> {
    wasi_impl::create_socket(family, protocol)
}

/// Connect to a remote address (TCP)
pub fn connect(handle: SocketHandle, address: SocketAddress) -> Result<(), SocketError> {
    wasi_impl::connect(handle, &address.host, address.port)
}

/// Bind socket to local address
pub fn bind(handle: SocketHandle, address: SocketAddress) -> Result<(), SocketError> {
    wasi_impl::bind(handle, &address.host, address.port)
}

/// Listen for incoming connections (TCP)
pub fn listen(handle: SocketHandle, backlog: u32) -> Result<(), SocketError> {
    wasi_impl::listen(handle, backlog)
}

/// Accept an incoming connection (TCP)
pub fn accept(handle: SocketHandle) -> Result<SocketHandle, SocketError> {
    wasi_impl::accept(handle).map(|(id, _, _)| id)
}

/// Send data through socket
pub fn send(handle: SocketHandle, data: &[u8]) -> Result<u64, SocketError> {
    wasi_impl::send(handle, data)
}

/// Receive data from socket
pub fn receive(handle: SocketHandle, max_len: u64) -> Result<Vec<u8>, SocketError> {
    wasi_impl::receive(handle, max_len)
}

/// Send data to specific address (UDP)
pub fn send_to(
    handle: SocketHandle,
    data: &[u8],
    address: SocketAddress,
) -> Result<u64, SocketError> {
    wasi_impl::send_to(handle, data, &address.host, address.port)
}

/// Receive data with sender address (UDP)
pub fn receive_from(
    handle: SocketHandle,
    max_len: u64,
) -> Result<(Vec<u8>, SocketAddress), SocketError> {
    wasi_impl::receive_from(handle, max_len)
        .map(|(data, host, port)| (data, SocketAddress::new(host, port)))
}

/// Close socket
pub fn close(handle: SocketHandle) -> Result<(), SocketError> {
    wasi_impl::close(handle)
}

/// Set socket option: read timeout
pub fn set_read_timeout(handle: SocketHandle, timeout_ms: Option<u64>) -> Result<(), SocketError> {
    wasi_impl::set_read_timeout(handle, timeout_ms)
}

/// Set socket option: write timeout
pub fn set_write_timeout(handle: SocketHandle, timeout_ms: Option<u64>) -> Result<(), SocketError> {
    wasi_impl::set_write_timeout(handle, timeout_ms)
}

/// Set socket option: reuse address
pub fn set_reuse_address(handle: SocketHandle, reuse: bool) -> Result<(), SocketError> {
    wasi_impl::set_reuse_address(handle, reuse)
}

/// Get local address of socket
pub fn get_local_address(handle: SocketHandle) -> Result<SocketAddress, SocketError> {
    wasi_impl::get_local_address(handle).map(|(host, port)| SocketAddress::new(host, port))
}

/// Get peer address of socket
pub fn get_peer_address(handle: SocketHandle) -> Result<SocketAddress, SocketError> {
    wasi_impl::get_peer_address(handle).map(|(host, port)| SocketAddress::new(host, port))
}
