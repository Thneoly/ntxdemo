// Real WASI socket implementation
//
// This module implements the socket API by calling WASI socket imports

// All WASI implementation is only available for WASM targets
#[cfg(target_arch = "wasm32")]
mod wasm_impl {
    use crate::component::scheduler::core_libs::wasi_network::{
        ErrorCode as WasiErrorCode, IpAddress as WasiIpAddress, IpAddressFamily, Network,
    };
    use crate::component::scheduler::core_libs::wasi_tcp::TcpSocket;
    use crate::component::scheduler::core_libs::wasi_udp::UdpSocket;

    use crate::socket::{AddressFamily, SocketError, SocketProtocol};
    use once_cell::sync::Lazy;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Socket handle tracking
    enum SocketHandle {
        Tcp(TcpSocket),
        Udp(UdpSocket),
    }

    /// Registry for tracking socket resources
    struct SocketRegistry {
        sockets: HashMap<u32, SocketHandle>,
        next_id: u32,
        network: Option<Network>,
    }

    impl SocketRegistry {
        fn new() -> Self {
            Self {
                sockets: HashMap::new(),
                next_id: 1,
                network: None,
            }
        }

        fn ensure_network(&mut self) -> Result<(), SocketError> {
            if self.network.is_none() {
                // Get the network instance from WASI
                #[cfg(target_arch = "wasm32")]
                {
                    let net = crate::component::scheduler::core_libs::wasi_network::get_network();
                    self.network = Some(net);
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    return Err(SocketError::Other);
                }
            }
            Ok(())
        }

        fn network(&self) -> Option<&Network> {
            self.network.as_ref()
        }

        fn register(&mut self, handle: SocketHandle) -> u32 {
            let id = self.next_id;
            self.next_id += 1;
            self.sockets.insert(id, handle);
            id
        }

        fn get(&self, id: u32) -> Option<&SocketHandle> {
            self.sockets.get(&id)
        }

        fn remove(&mut self, id: u32) -> Option<SocketHandle> {
            self.sockets.remove(&id)
        }
    }

    static REGISTRY: Lazy<Mutex<SocketRegistry>> = Lazy::new(|| Mutex::new(SocketRegistry::new()));

    /// Convert WASI error to socket error
    fn convert_error(error: WasiErrorCode) -> SocketError {
        match error {
            WasiErrorCode::ConnectionRefused => SocketError::ConnectionRefused,
            WasiErrorCode::ConnectionReset => SocketError::ConnectionReset,
            WasiErrorCode::ConnectionAborted => SocketError::ConnectionAborted,
            WasiErrorCode::NetworkUnreachable => SocketError::NetworkUnreachable,
            WasiErrorCode::AddressInUse => SocketError::AddressInUse,
            WasiErrorCode::AddressNotAvailable => SocketError::AddressNotAvailable,
            WasiErrorCode::Timeout => SocketError::Timeout,
            WasiErrorCode::WouldBlock => SocketError::WouldBlock,
            WasiErrorCode::InvalidArgument => SocketError::InvalidInput,
            _ => SocketError::Other,
        }
    }

    /// Parse IP address string
    fn parse_ip_address(addr_str: &str) -> Result<WasiIpAddress, SocketError> {
        if addr_str.contains(':') {
            // IPv6
            parse_ipv6(addr_str)
        } else {
            // IPv4
            parse_ipv4(addr_str)
        }
    }

    fn parse_ipv4(addr: &str) -> Result<WasiIpAddress, SocketError> {
        let parts: Vec<&str> = addr.split('.').collect();
        if parts.len() != 4 {
            return Err(SocketError::InvalidInput);
        }

        let mut octets = [0u8; 4];
        for (i, part) in parts.iter().enumerate() {
            octets[i] = part.parse().map_err(|_| SocketError::InvalidInput)?;
        }

        Ok(WasiIpAddress::Ipv4((
            octets[0], octets[1], octets[2], octets[3],
        )))
    }

    fn parse_ipv6(addr: &str) -> Result<WasiIpAddress, SocketError> {
        // Simple IPv6 parser (does not handle :: compression)
        let parts: Vec<&str> = addr.split(':').collect();
        if parts.len() != 8 {
            return Err(SocketError::InvalidInput);
        }

        let mut segments = [0u16; 8];
        for (i, part) in parts.iter().enumerate() {
            segments[i] = u16::from_str_radix(part, 16).map_err(|_| SocketError::InvalidInput)?;
        }

        Ok(WasiIpAddress::Ipv6((
            segments[0],
            segments[1],
            segments[2],
            segments[3],
            segments[4],
            segments[5],
            segments[6],
            segments[7],
        )))
    }

    /// Format IP address for display
    fn format_ip_address(addr: &WasiIpAddress) -> String {
        match addr {
            WasiIpAddress::Ipv4((a, b, c, d)) => format!("{}.{}.{}.{}", a, b, c, d),
            WasiIpAddress::Ipv6((a, b, c, d, e, f, g, h)) => {
                format!(
                    "{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}",
                    a, b, c, d, e, f, g, h
                )
            }
        }
    }

    /// Create a new socket
    pub fn create_socket(
        family: AddressFamily,
        protocol: SocketProtocol,
    ) -> Result<u32, SocketError> {
        let addr_family = match family {
            AddressFamily::Ipv4 => IpAddressFamily::Ipv4,
            AddressFamily::Ipv6 => IpAddressFamily::Ipv6,
        };

        let mut registry = REGISTRY.lock().unwrap();

        let handle = match protocol {
            SocketProtocol::Tcp => {
                let tcp = TcpSocket::new(addr_family).map_err(convert_error)?;
                SocketHandle::Tcp(tcp)
            }
            SocketProtocol::Udp => {
                let udp = UdpSocket::new(addr_family).map_err(convert_error)?;
                SocketHandle::Udp(udp)
            }
        };

        Ok(registry.register(handle))
    }

    /// Connect TCP socket
    pub fn connect(socket_id: u32, host: &str, port: u16) -> Result<(), SocketError> {
        let ip_addr = parse_ip_address(host)?;

        // First, ensure network is initialized
        {
            let mut registry = REGISTRY.lock().unwrap();
            registry.ensure_network()?;
        }

        // Then use network with socket in a new lock scope
        let registry = REGISTRY.lock().unwrap();
        let network = registry.network().ok_or(SocketError::Other)?;
        let socket = registry.get(socket_id).ok_or(SocketError::InvalidInput)?;

        let SocketHandle::Tcp(tcp) = socket else {
            return Err(SocketError::InvalidInput);
        };

        tcp.connect(network, ip_addr, port).map_err(convert_error)?;

        Ok(())
    }

    /// Bind socket
    pub fn bind(socket_id: u32, host: &str, port: u16) -> Result<(), SocketError> {
        let ip_addr = parse_ip_address(host)?;

        // First, ensure network is initialized
        {
            let mut registry = REGISTRY.lock().unwrap();
            registry.ensure_network()?;
        }

        // Then use network with socket in a new lock scope
        let registry = REGISTRY.lock().unwrap();
        let network = registry.network().ok_or(SocketError::Other)?;
        let socket = registry.get(socket_id).ok_or(SocketError::InvalidInput)?;

        match socket {
            SocketHandle::Tcp(tcp) => {
                tcp.bind(network, ip_addr, port).map_err(convert_error)?;
            }
            SocketHandle::Udp(udp) => {
                udp.bind(network, ip_addr, port).map_err(convert_error)?;
            }
        }

        Ok(())
    }

    /// Listen for connections (TCP only)
    pub fn listen(socket_id: u32, backlog: u32) -> Result<(), SocketError> {
        let registry = REGISTRY.lock().unwrap();
        let socket = registry.get(socket_id).ok_or(SocketError::InvalidInput)?;

        let SocketHandle::Tcp(tcp) = socket else {
            return Err(SocketError::InvalidInput);
        };

        tcp.listen(backlog).map_err(convert_error)?;

        Ok(())
    }

    /// Accept connection (TCP only)
    pub fn accept(socket_id: u32) -> Result<(u32, String, u16), SocketError> {
        let mut registry = REGISTRY.lock().unwrap();
        let socket = registry.get(socket_id).ok_or(SocketError::InvalidInput)?;

        let SocketHandle::Tcp(tcp) = socket else {
            return Err(SocketError::InvalidInput);
        };

        let (client_socket, remote_addr, remote_port) = tcp.accept().map_err(convert_error)?;

        let addr_str = format_ip_address(&remote_addr);
        let client_id = registry.register(SocketHandle::Tcp(client_socket));

        Ok((client_id, addr_str, remote_port))
    }

    /// Send data (TCP)
    pub fn send(socket_id: u32, data: &[u8]) -> Result<u64, SocketError> {
        let registry = REGISTRY.lock().unwrap();
        let socket = registry.get(socket_id).ok_or(SocketError::InvalidInput)?;

        let SocketHandle::Tcp(tcp) = socket else {
            return Err(SocketError::InvalidInput);
        };

        tcp.send(data).map_err(convert_error)
    }

    /// Receive data (TCP)
    pub fn receive(socket_id: u32, max_len: u64) -> Result<Vec<u8>, SocketError> {
        let registry = REGISTRY.lock().unwrap();
        let socket = registry.get(socket_id).ok_or(SocketError::InvalidInput)?;

        let SocketHandle::Tcp(tcp) = socket else {
            return Err(SocketError::InvalidInput);
        };

        tcp.receive(max_len).map_err(convert_error)
    }

    /// Send datagram (UDP)
    pub fn send_to(socket_id: u32, data: &[u8], host: &str, port: u16) -> Result<u64, SocketError> {
        let ip_addr = parse_ip_address(host)?;

        // First, ensure network is initialized
        {
            let mut registry = REGISTRY.lock().unwrap();
            registry.ensure_network()?;
        }

        // Then use network with socket in a new lock scope
        let registry = REGISTRY.lock().unwrap();
        let network = registry.network().ok_or(SocketError::Other)?;
        let socket = registry.get(socket_id).ok_or(SocketError::InvalidInput)?;

        let SocketHandle::Udp(udp) = socket else {
            return Err(SocketError::InvalidInput);
        };

        udp.send_to(data, network, ip_addr, port)
            .map_err(convert_error)
    }

    /// Receive datagram (UDP)
    pub fn receive_from(
        socket_id: u32,
        max_len: u64,
    ) -> Result<(Vec<u8>, String, u16), SocketError> {
        let registry = REGISTRY.lock().unwrap();
        let socket = registry.get(socket_id).ok_or(SocketError::InvalidInput)?;

        let SocketHandle::Udp(udp) = socket else {
            return Err(SocketError::InvalidInput);
        };

        let (data, remote_addr, remote_port) = udp.receive_from(max_len).map_err(convert_error)?;

        let addr_str = format_ip_address(&remote_addr);
        Ok((data, addr_str, remote_port))
    }

    /// Close socket
    pub fn close(socket_id: u32) -> Result<(), SocketError> {
        let mut registry = REGISTRY.lock().unwrap();
        registry
            .remove(socket_id)
            .ok_or(SocketError::InvalidInput)?;
        Ok(())
    }

    /// Set reuse address option
    pub fn set_reuse_address(socket_id: u32, reuse: bool) -> Result<(), SocketError> {
        let registry = REGISTRY.lock().unwrap();
        let socket = registry.get(socket_id).ok_or(SocketError::InvalidInput)?;

        match socket {
            SocketHandle::Tcp(tcp) => {
                tcp.set_reuse_address(reuse).map_err(convert_error)?;
            }
            SocketHandle::Udp(udp) => {
                udp.set_reuse_address(reuse).map_err(convert_error)?;
            }
        }

        Ok(())
    }

    /// Get local address
    pub fn get_local_address(socket_id: u32) -> Result<(String, u16), SocketError> {
        let registry = REGISTRY.lock().unwrap();
        let socket = registry.get(socket_id).ok_or(SocketError::InvalidInput)?;

        let (ip_addr, port) = match socket {
            SocketHandle::Tcp(tcp) => tcp.local_address().map_err(convert_error)?,
            SocketHandle::Udp(udp) => udp.local_address().map_err(convert_error)?,
        };

        let addr_str = format_ip_address(&ip_addr);
        Ok((addr_str, port))
    }

    /// Get peer address (TCP only)
    pub fn get_peer_address(socket_id: u32) -> Result<(String, u16), SocketError> {
        let registry = REGISTRY.lock().unwrap();
        let socket = registry.get(socket_id).ok_or(SocketError::InvalidInput)?;

        let SocketHandle::Tcp(tcp) = socket else {
            return Err(SocketError::InvalidInput);
        };

        let (ip_addr, port) = tcp.remote_address().map_err(convert_error)?;

        let addr_str = format_ip_address(&ip_addr);
        Ok((addr_str, port))
    }

    // Note: Timeouts are not directly supported in WASI sockets Preview 2
    // These functions are no-op stubs for compatibility
    pub fn set_read_timeout(_socket_id: u32, _timeout_ms: Option<u64>) -> Result<(), SocketError> {
        // WASI sockets don't have timeout options in the current spec
        Ok(())
    }

    pub fn set_write_timeout(_socket_id: u32, _timeout_ms: Option<u64>) -> Result<(), SocketError> {
        // WASI sockets don't have timeout options in the current spec
        Ok(())
    }
} // end of wasm_impl module

// Re-export all public functions for WASM targets
#[cfg(target_arch = "wasm32")]
pub use wasm_impl::*;

// Stub implementations for non-WASM targets (used in tests)
#[cfg(not(target_arch = "wasm32"))]
pub use crate::socket::{AddressFamily, SocketError, SocketProtocol};

#[cfg(not(target_arch = "wasm32"))]
pub fn create_socket(
    _family: AddressFamily,
    _protocol: SocketProtocol,
) -> Result<u32, SocketError> {
    Ok(1)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn connect(_socket_id: u32, _host: &str, _port: u16) -> Result<(), SocketError> {
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn bind(_socket_id: u32, _host: &str, _port: u16) -> Result<(), SocketError> {
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn listen(_socket_id: u32, _backlog: u32) -> Result<(), SocketError> {
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn accept(_socket_id: u32) -> Result<(u32, String, u16), SocketError> {
    Ok((2, "127.0.0.1".to_string(), 12345))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn send(_socket_id: u32, data: &[u8]) -> Result<u64, SocketError> {
    Ok(data.len() as u64)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn receive(_socket_id: u32, _max_len: u64) -> Result<Vec<u8>, SocketError> {
    Ok(Vec::new())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn send_to(_socket_id: u32, data: &[u8], _host: &str, _port: u16) -> Result<u64, SocketError> {
    Ok(data.len() as u64)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn receive_from(_socket_id: u32, _max_len: u64) -> Result<(Vec<u8>, String, u16), SocketError> {
    Ok((Vec::new(), "127.0.0.1".to_string(), 12345))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn close(_socket_id: u32) -> Result<(), SocketError> {
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn set_reuse_address(_socket_id: u32, _reuse: bool) -> Result<(), SocketError> {
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn get_local_address(_socket_id: u32) -> Result<(String, u16), SocketError> {
    Ok(("0.0.0.0".to_string(), 0))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn get_peer_address(_socket_id: u32) -> Result<(String, u16), SocketError> {
    Ok(("0.0.0.0".to_string(), 0))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn set_read_timeout(_socket_id: u32, _timeout_ms: Option<u64>) -> Result<(), SocketError> {
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn set_write_timeout(_socket_id: u32, _timeout_ms: Option<u64>) -> Result<(), SocketError> {
    Ok(())
}
