/// Advanced Socket API with IP binding support
///
/// This module provides a higher-level socket API that integrates with the IP pool management.
/// It follows the standard socket programming model: socket() -> bind() -> listen()/connect() -> send()/recv()
use crate::ip::IpBinding;
use std::net::IpAddr;

use super::{
    AddressFamily, SocketAddress, SocketError, SocketHandle, SocketProtocol, accept as raw_accept,
    bind as raw_bind, close as raw_close, connect as raw_connect,
    create_socket as raw_create_socket, listen as raw_listen, receive as raw_receive,
    receive_from as raw_receive_from, send as raw_send, send_to as raw_send_to,
};

/// High-level Socket structure with IP binding support
#[derive(Debug)]
pub struct Socket {
    /// Underlying socket handle
    handle: SocketHandle,
    /// Socket protocol (TCP/UDP)
    protocol: SocketProtocol,
    /// Address family (IPv4/IPv6)
    family: AddressFamily,
    /// Bound IP address (if any)
    bound_ip: Option<IpAddr>,
    /// Bound port (if any)
    bound_port: Option<u16>,
    /// Connected remote address (TCP only)
    remote_addr: Option<SocketAddress>,
    /// Socket state
    state: SocketState,
}

/// Socket state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SocketState {
    Created,
    Bound,
    Listening,
    Connected,
    Closed,
}

impl Socket {
    /// Create a new TCP socket
    pub fn new_tcp(family: AddressFamily) -> Result<Self, SocketError> {
        let handle = raw_create_socket(family, SocketProtocol::Tcp)?;
        Ok(Self {
            handle,
            protocol: SocketProtocol::Tcp,
            family,
            bound_ip: None,
            bound_port: None,
            remote_addr: None,
            state: SocketState::Created,
        })
    }

    /// Create a new UDP socket
    pub fn new_udp(family: AddressFamily) -> Result<Self, SocketError> {
        let handle = raw_create_socket(family, SocketProtocol::Udp)?;
        Ok(Self {
            handle,
            protocol: SocketProtocol::Udp,
            family,
            bound_ip: None,
            bound_port: None,
            remote_addr: None,
            state: SocketState::Created,
        })
    }

    /// Create a new TCP IPv4 socket
    pub fn tcp_v4() -> Result<Self, SocketError> {
        Self::new_tcp(AddressFamily::Ipv4)
    }

    /// Create a new TCP IPv6 socket
    pub fn tcp_v6() -> Result<Self, SocketError> {
        Self::new_tcp(AddressFamily::Ipv6)
    }

    /// Create a new UDP IPv4 socket
    pub fn udp_v4() -> Result<Self, SocketError> {
        Self::new_udp(AddressFamily::Ipv4)
    }

    /// Create a new UDP IPv6 socket
    pub fn udp_v6() -> Result<Self, SocketError> {
        Self::new_udp(AddressFamily::Ipv6)
    }

    /// Bind socket to an IP address from the IP pool
    ///
    /// This associates the socket with a specific IP address and port.
    /// The IP should be allocated from an IpPool before calling this method.
    ///
    /// # Example
    /// ```no_run
    /// # use scheduler_core::{Socket, IpPool, ResourceType};
    /// # use std::net::IpAddr;
    /// let mut pool = IpPool::new("my-pool");
    /// pool.add_cidr_range("192.168.1.0/24").unwrap();
    ///
    /// let ip = pool.allocate("instance", "socket1", ResourceType::Custom("socket".into())).unwrap();
    /// let mut sock = Socket::tcp_v4().unwrap();
    /// sock.bind_to_ip(ip, 8080).unwrap();
    /// ```
    pub fn bind_to_ip(&mut self, ip: IpAddr, port: u16) -> Result<(), SocketError> {
        if self.state != SocketState::Created {
            return Err(SocketError::InvalidInput);
        }

        let addr = SocketAddress::new(ip.to_string(), port);
        raw_bind(self.handle, addr)?;

        self.bound_ip = Some(ip);
        self.bound_port = Some(port);
        self.state = SocketState::Bound;
        Ok(())
    }

    /// Bind socket to an address (alternative to bind_to_ip)
    pub fn bind(&mut self, addr: SocketAddress) -> Result<(), SocketError> {
        if self.state != SocketState::Created {
            return Err(SocketError::InvalidInput);
        }

        // Try to parse the host as an IP address
        if let Ok(ip) = addr.host.parse::<IpAddr>() {
            self.bound_ip = Some(ip);
        }
        self.bound_port = Some(addr.port);

        raw_bind(self.handle, addr)?;
        self.state = SocketState::Bound;
        Ok(())
    }

    /// Bind socket using an IP binding from the pool
    ///
    /// This is a convenience method that extracts the IP from an IpBinding.
    pub fn bind_with_binding(&mut self, binding: &IpBinding, port: u16) -> Result<(), SocketError> {
        self.bind_to_ip(binding.ip, port)
    }

    /// Listen for incoming connections (TCP only)
    ///
    /// Must be called after bind(). The backlog parameter specifies the maximum
    /// number of pending connections.
    pub fn listen(&mut self, backlog: u32) -> Result<(), SocketError> {
        if self.protocol != SocketProtocol::Tcp {
            return Err(SocketError::InvalidInput);
        }
        if self.state != SocketState::Bound {
            return Err(SocketError::InvalidInput);
        }

        raw_listen(self.handle, backlog)?;
        self.state = SocketState::Listening;
        Ok(())
    }

    /// Connect to a remote address (TCP only)
    ///
    /// For unbound sockets, the system will automatically select a local IP and port.
    /// For bound sockets, the connection will use the bound IP as the source address.
    pub fn connect(&mut self, addr: SocketAddress) -> Result<(), SocketError> {
        if self.protocol != SocketProtocol::Tcp {
            return Err(SocketError::InvalidInput);
        }
        if self.state != SocketState::Created && self.state != SocketState::Bound {
            return Err(SocketError::InvalidInput);
        }

        raw_connect(self.handle, addr.clone())?;
        self.remote_addr = Some(addr);
        self.state = SocketState::Connected;
        Ok(())
    }

    /// Accept an incoming connection (TCP only)
    ///
    /// Returns a new Socket for the accepted connection.
    /// Must be called on a listening socket.
    pub fn accept(&mut self) -> Result<Socket, SocketError> {
        if self.protocol != SocketProtocol::Tcp {
            return Err(SocketError::InvalidInput);
        }
        if self.state != SocketState::Listening {
            return Err(SocketError::InvalidInput);
        }

        let new_handle = raw_accept(self.handle)?;

        // Create a new socket for the accepted connection
        Ok(Socket {
            handle: new_handle,
            protocol: self.protocol,
            family: self.family,
            bound_ip: self.bound_ip,
            bound_port: self.bound_port,
            remote_addr: None, // Will be set if we can get peer info
            state: SocketState::Connected,
        })
    }

    /// Send data through the socket
    ///
    /// For TCP: Must be connected first
    /// For UDP: Can be used with connect() or send_to()
    pub fn send(&mut self, data: &[u8]) -> Result<u64, SocketError> {
        match self.state {
            SocketState::Connected => raw_send(self.handle, data),
            SocketState::Bound if self.protocol == SocketProtocol::Udp => {
                // UDP can send even if not connected
                raw_send(self.handle, data)
            }
            _ => Err(SocketError::InvalidInput),
        }
    }

    /// Receive data from the socket
    ///
    /// Returns the number of bytes received and the data.
    pub fn recv(&mut self, max_len: u64) -> Result<Vec<u8>, SocketError> {
        match self.state {
            SocketState::Connected => raw_receive(self.handle, max_len),
            SocketState::Bound if self.protocol == SocketProtocol::Udp => {
                // UDP can receive even if not connected
                raw_receive(self.handle, max_len)
            }
            _ => Err(SocketError::InvalidInput),
        }
    }

    /// Send data to a specific address (UDP only)
    ///
    /// The socket must be bound first.
    pub fn send_to(&mut self, data: &[u8], addr: SocketAddress) -> Result<u64, SocketError> {
        if self.protocol != SocketProtocol::Udp {
            return Err(SocketError::InvalidInput);
        }
        if self.state != SocketState::Bound {
            return Err(SocketError::InvalidInput);
        }

        raw_send_to(self.handle, data, addr)
    }

    /// Receive data from any address (UDP only)
    ///
    /// Returns the data and the sender's address.
    pub fn recv_from(&mut self, max_len: u64) -> Result<(Vec<u8>, SocketAddress), SocketError> {
        if self.protocol != SocketProtocol::Udp {
            return Err(SocketError::InvalidInput);
        }
        if self.state != SocketState::Bound {
            return Err(SocketError::InvalidInput);
        }

        raw_receive_from(self.handle, max_len)
    }

    /// Close the socket
    pub fn close(&mut self) -> Result<(), SocketError> {
        if self.state == SocketState::Closed {
            return Ok(());
        }

        raw_close(self.handle)?;
        self.state = SocketState::Closed;
        Ok(())
    }

    /// Get the socket handle
    pub fn handle(&self) -> SocketHandle {
        self.handle
    }

    /// Get the bound IP address
    pub fn local_ip(&self) -> Option<IpAddr> {
        self.bound_ip
    }

    /// Get the bound port
    pub fn local_port(&self) -> Option<u16> {
        self.bound_port
    }

    /// Get the remote address (for connected TCP sockets)
    pub fn remote_addr(&self) -> Option<&SocketAddress> {
        self.remote_addr.as_ref()
    }

    /// Get the socket protocol
    pub fn protocol(&self) -> SocketProtocol {
        self.protocol
    }

    /// Get the address family
    pub fn family(&self) -> AddressFamily {
        self.family
    }

    /// Check if socket is connected
    pub fn is_connected(&self) -> bool {
        self.state == SocketState::Connected
    }

    /// Check if socket is bound
    pub fn is_bound(&self) -> bool {
        matches!(
            self.state,
            SocketState::Bound | SocketState::Listening | SocketState::Connected
        )
    }

    /// Check if socket is listening
    pub fn is_listening(&self) -> bool {
        self.state == SocketState::Listening
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tcp_socket_creation() {
        let sock = Socket::tcp_v4();
        assert!(sock.is_ok());
        let sock = sock.unwrap();
        assert_eq!(sock.protocol(), SocketProtocol::Tcp);
        assert_eq!(sock.family(), AddressFamily::Ipv4);
    }

    #[test]
    fn test_udp_socket_creation() {
        let sock = Socket::udp_v4();
        assert!(sock.is_ok());
        let sock = sock.unwrap();
        assert_eq!(sock.protocol(), SocketProtocol::Udp);
    }

    #[test]
    fn test_socket_bind() {
        let mut sock = Socket::tcp_v4().unwrap();
        let addr = SocketAddress::new("127.0.0.1", 8080);
        let result = sock.bind(addr);
        assert!(result.is_ok());
        assert!(sock.is_bound());
    }

    #[test]
    fn test_socket_bind_to_ip() {
        let mut sock = Socket::tcp_v4().unwrap();
        let ip = "127.0.0.1".parse().unwrap();
        let result = sock.bind_to_ip(ip, 9000);
        assert!(result.is_ok());
        assert_eq!(sock.local_ip(), Some(ip));
        assert_eq!(sock.local_port(), Some(9000));
    }

    #[test]
    fn test_socket_lifecycle() {
        let mut sock = Socket::tcp_v4().unwrap();
        assert!(!sock.is_bound());
        assert!(!sock.is_connected());
        assert!(!sock.is_listening());

        // Bind
        let addr = SocketAddress::new("0.0.0.0", 8080);
        sock.bind(addr).unwrap();
        assert!(sock.is_bound());

        // Listen
        sock.listen(10).unwrap();
        assert!(sock.is_listening());
    }
}
