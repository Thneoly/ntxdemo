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

        socket::create_socket(native_family, native_protocol)
            .map_err(|_| exports::scheduler::core_libs::socket::SocketError::Other)
    }

    fn connect(
        socket: u32,
        address: exports::scheduler::core_libs::socket::SocketAddress,
    ) -> Result<(), exports::scheduler::core_libs::socket::SocketError> {
        use crate::socket;

        let native_addr = socket::SocketAddress::new(address.host, address.port);
        socket::connect(socket, native_addr)
            .map_err(|_| exports::scheduler::core_libs::socket::SocketError::Other)
    }

    fn bind(
        socket: u32,
        address: exports::scheduler::core_libs::socket::SocketAddress,
    ) -> Result<(), exports::scheduler::core_libs::socket::SocketError> {
        use crate::socket;

        let native_addr = socket::SocketAddress::new(address.host, address.port);
        socket::bind(socket, native_addr)
            .map_err(|_| exports::scheduler::core_libs::socket::SocketError::Other)
    }

    fn send(
        socket: u32,
        data: Vec<u8>,
    ) -> Result<u64, exports::scheduler::core_libs::socket::SocketError> {
        use crate::socket;

        socket::send(socket, &data)
            .map_err(|_| exports::scheduler::core_libs::socket::SocketError::Other)
    }

    fn receive(
        socket: u32,
        max_length: u64,
    ) -> Result<Vec<u8>, exports::scheduler::core_libs::socket::SocketError> {
        use crate::socket;

        socket::receive(socket, max_length)
            .map_err(|_| exports::scheduler::core_libs::socket::SocketError::Other)
    }

    fn close(socket: u32) -> Result<(), exports::scheduler::core_libs::socket::SocketError> {
        use crate::socket;

        socket::close(socket).map_err(|_| exports::scheduler::core_libs::socket::SocketError::Other)
    }
}

#[cfg(target_arch = "wasm32")]
export!(SchedulerCoreImpl);
