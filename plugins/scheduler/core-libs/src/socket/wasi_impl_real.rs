// Real WASI socket implementation
//
// This module implements the socket API by calling WASI Preview 2 socket imports.

use crate::component::wasi::io::streams::{self, InputStream, OutputStream};
use crate::component::wasi::sockets::instance_network::instance_network;
use crate::component::wasi::sockets::network::{
    ErrorCode as WasiErrorCode, IpAddressFamily, IpSocketAddress, Ipv4SocketAddress,
    Ipv6SocketAddress,
};
use crate::component::wasi::sockets::tcp::TcpSocket;
use crate::component::wasi::sockets::tcp_create_socket::create_tcp_socket;
use crate::component::wasi::sockets::udp::UdpSocket;
use crate::component::wasi::sockets::udp_create_socket::create_udp_socket;
use crate::socket::{AddressFamily, SocketError, SocketProtocol};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Mutex;

const MAX_ASYNC_SPINS: usize = 2048;

struct TcpState {
    socket: TcpSocket,
    input: Option<InputStream>,
    output: Option<OutputStream>,
}

struct UdpState {
    socket: UdpSocket,
}

enum SocketHandle {
    Tcp(TcpState),
    Udp(UdpState),
}

struct SocketRegistry {
    sockets: HashMap<u32, SocketHandle>,
    next_id: u32,
}

impl SocketRegistry {
    fn new() -> Self {
        Self {
            sockets: HashMap::new(),
            next_id: 1,
        }
    }

    fn register(&mut self, handle: SocketHandle) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.sockets.insert(id, handle);
        id
    }

    fn get_mut(&mut self, id: u32) -> Option<&mut SocketHandle> {
        self.sockets.get_mut(&id)
    }

    fn get(&self, id: u32) -> Option<&SocketHandle> {
        self.sockets.get(&id)
    }

    fn remove(&mut self, id: u32) -> Option<SocketHandle> {
        self.sockets.remove(&id)
    }
}

static REGISTRY: Lazy<Mutex<SocketRegistry>> = Lazy::new(|| Mutex::new(SocketRegistry::new()));

fn convert_error(error: WasiErrorCode) -> SocketError {
    match error {
        WasiErrorCode::ConnectionRefused => SocketError::ConnectionRefused,
        WasiErrorCode::ConnectionReset => SocketError::ConnectionReset,
        WasiErrorCode::ConnectionAborted => SocketError::ConnectionAborted,
        WasiErrorCode::RemoteUnreachable => SocketError::NetworkUnreachable,
        WasiErrorCode::AddressInUse => SocketError::AddressInUse,
        WasiErrorCode::AddressNotBindable => SocketError::AddressNotAvailable,
        WasiErrorCode::Timeout => SocketError::Timeout,
        WasiErrorCode::WouldBlock => SocketError::WouldBlock,
        WasiErrorCode::InvalidArgument => SocketError::InvalidInput,
        _ => SocketError::Other,
    }
}

fn convert_stream_error(err: streams::StreamError) -> SocketError {
    match err {
        streams::StreamError::Closed => SocketError::ConnectionReset,
        streams::StreamError::LastOperationFailed(_) => SocketError::Other,
    }
}

fn parse_socket_address(host: &str, port: u16) -> Result<IpSocketAddress, SocketError> {
    if let Ok(addr) = host.parse::<Ipv4Addr>() {
        let octets = addr.octets();
        Ok(IpSocketAddress::Ipv4(Ipv4SocketAddress {
            port,
            address: (octets[0], octets[1], octets[2], octets[3]),
        }))
    } else if let Ok(addr) = host.parse::<Ipv6Addr>() {
        let segments = addr.segments();
        Ok(IpSocketAddress::Ipv6(Ipv6SocketAddress {
            port,
            flow_info: 0,
            address: (
                segments[0],
                segments[1],
                segments[2],
                segments[3],
                segments[4],
                segments[5],
                segments[6],
                segments[7],
            ),
            scope_id: 0,
        }))
    } else {
        Err(SocketError::InvalidInput)
    }
}

fn ip_socket_to_parts(addr: IpSocketAddress) -> (String, u16) {
    match addr {
        IpSocketAddress::Ipv4(v4) => {
            let (a, b, c, d) = v4.address;
            (Ipv4Addr::new(a, b, c, d).to_string(), v4.port)
        }
        IpSocketAddress::Ipv6(v6) => {
            let (a, b, c, d, e, f, g, h) = v6.address;
            (Ipv6Addr::new(a, b, c, d, e, f, g, h).to_string(), v6.port)
        }
    }
}

fn spin_wait<FStart, FFinish, R>(start: FStart, mut finish: FFinish) -> Result<R, SocketError>
where
    FStart: FnOnce() -> Result<(), WasiErrorCode>,
    FFinish: FnMut() -> Result<R, WasiErrorCode>,
{
    start().map_err(convert_error)?;
    let mut spins = 0usize;
    loop {
        match finish() {
            Ok(result) => return Ok(result),
            Err(WasiErrorCode::WouldBlock) => {
                spins += 1;
                if spins > MAX_ASYNC_SPINS {
                    return Err(SocketError::Timeout);
                }
                core::hint::spin_loop();
            }
            Err(err) => return Err(convert_error(err)),
        }
    }
}

pub fn create_socket(family: AddressFamily, protocol: SocketProtocol) -> Result<u32, SocketError> {
    let addr_family = match family {
        AddressFamily::Ipv4 => IpAddressFamily::Ipv4,
        AddressFamily::Ipv6 => IpAddressFamily::Ipv6,
    };

    let mut registry = REGISTRY.lock().unwrap();
    let handle = match protocol {
        SocketProtocol::Tcp => {
            let socket = create_tcp_socket(addr_family).map_err(convert_error)?;
            SocketHandle::Tcp(TcpState {
                socket,
                input: None,
                output: None,
            })
        }
        SocketProtocol::Udp => {
            let socket = create_udp_socket(addr_family).map_err(convert_error)?;
            SocketHandle::Udp(UdpState { socket })
        }
    };

    Ok(registry.register(handle))
}

pub fn connect(socket_id: u32, host: &str, port: u16) -> Result<(), SocketError> {
    let remote = parse_socket_address(host, port)?;
    let network = instance_network();
    let mut registry = REGISTRY.lock().unwrap();
    let handle = registry
        .get_mut(socket_id)
        .ok_or(SocketError::InvalidInput)?;

    match handle {
        SocketHandle::Tcp(state) => {
            let (input, output) = spin_wait(
                || state.socket.start_connect(&network, remote.clone()),
                || state.socket.finish_connect(),
            )?;
            state.input = Some(input);
            state.output = Some(output);
            Ok(())
        }
        SocketHandle::Udp(_) => Err(SocketError::InvalidInput),
    }
}

pub fn bind(socket_id: u32, host: &str, port: u16) -> Result<(), SocketError> {
    let local = parse_socket_address(host, port)?;
    let network = instance_network();
    let mut registry = REGISTRY.lock().unwrap();
    let handle = registry
        .get_mut(socket_id)
        .ok_or(SocketError::InvalidInput)?;

    match handle {
        SocketHandle::Tcp(state) => spin_wait(
            || state.socket.start_bind(&network, local.clone()),
            || state.socket.finish_bind(),
        ),
        SocketHandle::Udp(state) => spin_wait(
            || state.socket.start_bind(&network, local.clone()),
            || state.socket.finish_bind(),
        ),
    }
}

pub fn listen(socket_id: u32, backlog: u32) -> Result<(), SocketError> {
    let mut registry = REGISTRY.lock().unwrap();
    let handle = registry
        .get_mut(socket_id)
        .ok_or(SocketError::InvalidInput)?;

    match handle {
        SocketHandle::Tcp(state) => {
            if backlog > 0 {
                let _ = state
                    .socket
                    .set_listen_backlog_size(backlog as u64)
                    .map_err(convert_error);
            }
            spin_wait(
                || state.socket.start_listen(),
                || state.socket.finish_listen(),
            )
        }
        SocketHandle::Udp(_) => Err(SocketError::InvalidInput),
    }
}

pub fn accept(socket_id: u32) -> Result<(u32, String, u16), SocketError> {
    let mut registry = REGISTRY.lock().unwrap();
    let handle = registry
        .get_mut(socket_id)
        .ok_or(SocketError::InvalidInput)?;

    let SocketHandle::Tcp(state) = handle else {
        return Err(SocketError::InvalidInput);
    };

    let mut spins = 0usize;
    let (client_socket, input, output) = loop {
        match state.socket.accept() {
            Ok(tuple) => break tuple,
            Err(WasiErrorCode::WouldBlock) => {
                spins += 1;
                if spins > MAX_ASYNC_SPINS {
                    return Err(SocketError::Timeout);
                }
                core::hint::spin_loop();
            }
            Err(err) => return Err(convert_error(err)),
        }
    };

    let (peer_host, peer_port) = client_socket
        .remote_address()
        .map(ip_socket_to_parts)
        .map_err(convert_error)?;

    let client_id = registry.register(SocketHandle::Tcp(TcpState {
        socket: client_socket,
        input: Some(input),
        output: Some(output),
    }));

    Ok((client_id, peer_host, peer_port))
}

pub fn send(socket_id: u32, data: &[u8]) -> Result<u64, SocketError> {
    let mut registry = REGISTRY.lock().unwrap();
    let handle = registry
        .get_mut(socket_id)
        .ok_or(SocketError::InvalidInput)?;

    let SocketHandle::Tcp(state) = handle else {
        return Err(SocketError::InvalidInput);
    };

    let output = state.output.as_mut().ok_or(SocketError::InvalidInput)?;
    output.write(data).map_err(convert_stream_error)?;
    Ok(data.len() as u64)
}

pub fn receive(socket_id: u32, max_len: u64) -> Result<Vec<u8>, SocketError> {
    let mut registry = REGISTRY.lock().unwrap();
    let handle = registry
        .get_mut(socket_id)
        .ok_or(SocketError::InvalidInput)?;

    let SocketHandle::Tcp(state) = handle else {
        return Err(SocketError::InvalidInput);
    };

    let input = state.input.as_mut().ok_or(SocketError::InvalidInput)?;
    match input.read(max_len) {
        Ok(bytes) => Ok(bytes),
        Err(streams::StreamError::Closed) => Ok(Vec::new()),
        Err(other) => Err(convert_stream_error(other)),
    }
}

pub fn send_to(_socket_id: u32, _data: &[u8], _host: &str, _port: u16) -> Result<u64, SocketError> {
    Err(SocketError::Other)
}

pub fn receive_from(_socket_id: u32, _max_len: u64) -> Result<(Vec<u8>, String, u16), SocketError> {
    Err(SocketError::Other)
}

pub fn close(socket_id: u32) -> Result<(), SocketError> {
    let mut registry = REGISTRY.lock().unwrap();
    let handle = registry
        .remove(socket_id)
        .ok_or(SocketError::InvalidInput)?;

    if let SocketHandle::Tcp(mut state) = handle {
        // Drop streams explicitly before socket so WASI doesn't complain about live children
        state.input.take();
        state.output.take();
    }

    Ok(())
}

pub fn set_reuse_address(_socket_id: u32, _reuse: bool) -> Result<(), SocketError> {
    Ok(())
}

pub fn get_local_address(socket_id: u32) -> Result<(String, u16), SocketError> {
    let registry = REGISTRY.lock().unwrap();
    let handle = registry.get(socket_id).ok_or(SocketError::InvalidInput)?;

    match handle {
        SocketHandle::Tcp(state) => state
            .socket
            .local_address()
            .map(ip_socket_to_parts)
            .map_err(convert_error),
        SocketHandle::Udp(state) => state
            .socket
            .local_address()
            .map(ip_socket_to_parts)
            .map_err(convert_error),
    }
}

pub fn get_peer_address(socket_id: u32) -> Result<(String, u16), SocketError> {
    let registry = REGISTRY.lock().unwrap();
    let handle = registry.get(socket_id).ok_or(SocketError::InvalidInput)?;

    let SocketHandle::Tcp(state) = handle else {
        return Err(SocketError::InvalidInput);
    };

    state
        .socket
        .remote_address()
        .map(ip_socket_to_parts)
        .map_err(convert_error)
}

pub fn set_read_timeout(_socket_id: u32, _timeout_ms: Option<u64>) -> Result<(), SocketError> {
    Ok(())
}

pub fn set_write_timeout(_socket_id: u32, _timeout_ms: Option<u64>) -> Result<(), SocketError> {
    Ok(())
}
