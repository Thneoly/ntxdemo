// Component bindings for scheduler-core
#[cfg(target_arch = "wasm32")]
wit_bindgen::generate!({
    world: "scheduler-core",
    path: "wit",
});

#[cfg(target_arch = "wasm32")]
struct SchedulerCoreImpl;

#[cfg(target_arch = "wasm32")]
impl exports::scheduler::core_libs::parser::Guest for SchedulerCoreImpl {
    fn parse_scenario(
        yaml: String,
    ) -> Result<exports::scheduler::core_libs::types::Scenario, String> {
        use crate::dsl::Scenario;

        let scenario = Scenario::from_yaml_str(&yaml).map_err(|e| format!("parse error: {}", e))?;

        Ok(exports::scheduler::core_libs::types::Scenario {
            version: scenario.version.clone(),
            name: scenario.name.clone(),
            resources: vec![],
            actions: vec![],
            nodes: vec![],
        })
    }

    fn validate_scenario(
        _scenario: exports::scheduler::core_libs::types::Scenario,
    ) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
impl exports::scheduler::core_libs::socket::Guest for SchedulerCoreImpl {
    fn create_socket(
        family: exports::scheduler::core_libs::socket::AddressFamily,
        protocol: exports::scheduler::core_libs::socket::SocketProtocol,
    ) -> Result<u32, exports::scheduler::core_libs::socket::SocketError> {
        use crate::socket;

        let native_family = match family {
            exports::scheduler::core_libs::socket::AddressFamily::Ipv4 => {
                socket::AddressFamily::Ipv4
            }
            exports::scheduler::core_libs::socket::AddressFamily::Ipv6 => {
                socket::AddressFamily::Ipv6
            }
        };

        let native_protocol = match protocol {
            exports::scheduler::core_libs::socket::SocketProtocol::Tcp => {
                socket::SocketProtocol::Tcp
            }
            exports::scheduler::core_libs::socket::SocketProtocol::Udp => {
                socket::SocketProtocol::Udp
            }
        };

        socket::create_socket(native_family, native_protocol).map_err(convert_socket_error)
    }

    fn connect(
        socket: u32,
        address: exports::scheduler::core_libs::socket::SocketAddress,
    ) -> Result<(), exports::scheduler::core_libs::socket::SocketError> {
        use crate::socket;

        let native_addr = socket::SocketAddress::new(address.host, address.port);
        socket::connect(socket, native_addr).map_err(convert_socket_error)
    }

    fn bind(
        socket: u32,
        address: exports::scheduler::core_libs::socket::SocketAddress,
    ) -> Result<(), exports::scheduler::core_libs::socket::SocketError> {
        use crate::socket;

        let native_addr = socket::SocketAddress::new(address.host, address.port);
        socket::bind(socket, native_addr).map_err(convert_socket_error)
    }

    fn listen(
        socket: u32,
        backlog: u32,
    ) -> Result<(), exports::scheduler::core_libs::socket::SocketError> {
        use crate::socket;

        socket::listen(socket, backlog).map_err(convert_socket_error)
    }

    fn accept(socket: u32) -> Result<u32, exports::scheduler::core_libs::socket::SocketError> {
        use crate::socket;

        socket::accept(socket).map_err(convert_socket_error)
    }

    fn send(
        socket: u32,
        data: Vec<u8>,
    ) -> Result<u64, exports::scheduler::core_libs::socket::SocketError> {
        use crate::socket;

        socket::send(socket, &data).map_err(convert_socket_error)
    }

    fn receive(
        socket: u32,
        max_len: u64,
    ) -> Result<Vec<u8>, exports::scheduler::core_libs::socket::SocketError> {
        use crate::socket;

        socket::receive(socket, max_len).map_err(convert_socket_error)
    }

    fn send_to(
        socket: u32,
        data: Vec<u8>,
        address: exports::scheduler::core_libs::socket::SocketAddress,
    ) -> Result<u64, exports::scheduler::core_libs::socket::SocketError> {
        use crate::socket;

        let native_addr = socket::SocketAddress::new(address.host, address.port);
        socket::send_to(socket, &data, native_addr).map_err(convert_socket_error)
    }

    fn receive_from(
        socket: u32,
        max_len: u64,
    ) -> Result<
        (
            Vec<u8>,
            exports::scheduler::core_libs::socket::SocketAddress,
        ),
        exports::scheduler::core_libs::socket::SocketError,
    > {
        use crate::socket;

        let (data, addr) = socket::receive_from(socket, max_len).map_err(convert_socket_error)?;

        Ok((
            data,
            exports::scheduler::core_libs::socket::SocketAddress {
                host: addr.host,
                port: addr.port,
            },
        ))
    }

    fn close(socket: u32) -> Result<(), exports::scheduler::core_libs::socket::SocketError> {
        use crate::socket;

        socket::close(socket).map_err(convert_socket_error)
    }

    fn set_read_timeout(
        socket: u32,
        timeout_ms: Option<u64>,
    ) -> Result<(), exports::scheduler::core_libs::socket::SocketError> {
        use crate::socket;

        socket::set_read_timeout(socket, timeout_ms).map_err(convert_socket_error)
    }

    fn set_write_timeout(
        socket: u32,
        timeout_ms: Option<u64>,
    ) -> Result<(), exports::scheduler::core_libs::socket::SocketError> {
        use crate::socket;

        socket::set_write_timeout(socket, timeout_ms).map_err(convert_socket_error)
    }

    fn set_reuse_address(
        socket: u32,
        reuse: bool,
    ) -> Result<(), exports::scheduler::core_libs::socket::SocketError> {
        use crate::socket;

        socket::set_reuse_address(socket, reuse).map_err(convert_socket_error)
    }

    fn get_local_address(
        socket: u32,
    ) -> Result<
        exports::scheduler::core_libs::socket::SocketAddress,
        exports::scheduler::core_libs::socket::SocketError,
    > {
        use crate::socket;

        let addr = socket::get_local_address(socket).map_err(convert_socket_error)?;

        Ok(exports::scheduler::core_libs::socket::SocketAddress {
            host: addr.host,
            port: addr.port,
        })
    }

    fn get_peer_address(
        socket: u32,
    ) -> Result<
        exports::scheduler::core_libs::socket::SocketAddress,
        exports::scheduler::core_libs::socket::SocketError,
    > {
        use crate::socket;

        let addr = socket::get_peer_address(socket).map_err(convert_socket_error)?;

        Ok(exports::scheduler::core_libs::socket::SocketAddress {
            host: addr.host,
            port: addr.port,
        })
    }
}

#[cfg(target_arch = "wasm32")]
fn convert_socket_error(
    err: crate::socket::SocketError,
) -> exports::scheduler::core_libs::socket::SocketError {
    use crate::socket::SocketError as Native;
    use exports::scheduler::core_libs::socket::SocketError as Export;

    match err {
        Native::ConnectionRefused => Export::ConnectionRefused,
        Native::ConnectionReset => Export::ConnectionReset,
        Native::ConnectionAborted => Export::ConnectionAborted,
        Native::NetworkUnreachable => Export::NetworkUnreachable,
        Native::AddressInUse => Export::AddressInUse,
        Native::AddressNotAvailable => Export::AddressNotAvailable,
        Native::Timeout => Export::Timeout,
        Native::WouldBlock => Export::WouldBlock,
        Native::InvalidInput => Export::InvalidInput,
        Native::Other => Export::Other,
    }
}

#[cfg(target_arch = "wasm32")]
export!(SchedulerCoreImpl);
