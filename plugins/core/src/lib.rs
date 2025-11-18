use std::cell::RefCell;
use wit_bindgen::generate;
generate!({
    world: "core",
    path: ["../wit",],
    generate_all,
});
use exports::component::tcp_client::config::{Guest as ConfigGuest, GuestConfig};
// use exports::component::tcp_client::flow::Workbook;
// use exports::component::tcp_client::flow::{Guest as FlowGuest, GuestRes};
use exports::component::tcp_client::logging::{Guest as LoggingGuest, Level};
use exports::component::tcp_client::respool::{Guest as RespoolGuest, GuestScript};
use exports::component::tcp_client::sock::{Guest as SockGuest, GuestSock};

use crate::wasi::io::streams;
use wasi::sockets::instance_network::instance_network;
use wasi::sockets::network::{ErrorCode, Ipv4SocketAddress, Network};
use wasi::sockets::tcp::IpSocketAddress;
use wasi::sockets::tcp_create_socket::{IpAddressFamily, TcpSocket, create_tcp_socket};

struct Core;
struct ConfigConfig;
struct RespoolScript;
// struct FlowRes;
struct SockSock {
    id: RefCell<u32>,
    sock_type: RefCell<u32>,
    ip: RefCell<String>,
    port: RefCell<u16>,
    sock: RefCell<Option<TcpSocket>>,
    input: RefCell<Option<streams::InputStream>>,
    output: RefCell<Option<streams::OutputStream>>,
}

impl GuestScript for RespoolScript {
    fn new() -> Self {
        RespoolScript {}
    }
}
impl GuestSock for SockSock {
    fn new(name: String) -> Self {
        println!("new sock {}", name);
        SockSock {
            sock_type: RefCell::new(0),
            id: RefCell::new(0),
            ip: RefCell::new(String::new()),
            port: RefCell::new(0),
            sock: RefCell::new(None),
            input: RefCell::new(None),
            output: RefCell::new(None),
        }
    }
    fn newsock(&self, sock_type: u32) -> u32 {
        self.sock_type.replace(sock_type);
        sock_type
    }
    fn bind(&self, id: u32, _ip: String, port: u16) {
        if id != *self.id.borrow() {
            println!("Invalid sock id: {}", id);
            return;
        }
        if *self.sock_type.borrow() != 1 {
            println!(
                "Invalid sock type: {}, expected: {}",
                *self.sock_type.borrow(),
                1
            );
            return;
        }
        let remote = IpSocketAddress::Ipv4(Ipv4SocketAddress {
            address: (127, 0, 0, 1),
            port: 8080,
        });
        self.ip.replace("127.0.0.1".to_string());
        self.port.replace(port);
        println!(
            "Binding sock id: {} to {}:{}",
            id,
            self.ip.borrow(),
            self.port.borrow()
        );
        let network: Network = instance_network();
        let sock: TcpSocket = match create_tcp_socket(IpAddressFamily::Ipv4) {
            Ok(sock) => sock,
            Err(error) => panic!("Failed to create socket: {}", error),
        };
        match sock.start_connect(&network, remote) {
            Ok(()) => println!("Started connecting..."),
            Err(error) => {
                println!("Error starting connect: {}", error);
                panic!("Failed to start connect {}", error)
            }
        }
        let (input, output) = loop {
            match sock.finish_connect() {
                Ok(streams) => {
                    println!("Connected successfully!");
                    break streams; // Exit the loop with the streams
                }
                Err(ErrorCode::WouldBlock) => {
                    // Connection still in progress, can retry
                }
                Err(e) => panic!("Failed to finish connect: {:?}", e),
            }
        };
        self.sock.replace(Some(sock));
        self.input.replace(Some(input));
        self.output.replace(Some(output));
    }
    fn send(&self, _id: u32, data: Vec<u8>) {
        match *self.output.borrow_mut() {
            Some(ref mut output) => match output.write(&data) {
                Ok(_) => {
                    println!("Sent bytes");
                }
                Err(e) => {
                    println!("Failed to send data: {}", e);
                }
            },
            None => {
                println!("No output stream available");
            }
        }
    }
    fn recv(&self, _id: u32, len: u32) -> Vec<u8> {
        match *self.input.borrow_mut() {
            Some(ref mut input) => match input.read(len.into()) {
                Ok(rcv_bytes) => {
                    println!("Received {:#?} bytes", rcv_bytes);
                    rcv_bytes
                }
                Err(e) => {
                    println!("Failed to receive data: {}", e);
                    vec![]
                }
            },
            None => {
                println!("No input stream available");
                vec![]
            }
        }
    }
    fn close(&self, _id: u32) {
        drop(self.input.borrow_mut().take());
        drop(self.output.borrow_mut().take());
        drop(self.sock.borrow_mut().take());

        println!("Closing");
    }
}

impl GuestConfig for ConfigConfig {
    fn new() -> Self {
        ConfigConfig {}
    }
    fn get_config(&self) -> String {
        String::from("core_config")
    }
}
// impl GuestRes for FlowRes {
//     fn new() -> Self {
//         FlowRes {}
//     }
//     fn execute(&self, wb: Workbook) -> Workbook {
//         wb
//     }
// }

impl RespoolGuest for Core {
    type Script = RespoolScript;
}
// impl FlowGuest for Core {
//     type Res = FlowRes;
// }
impl LoggingGuest for Core {
    fn log(level: Level, message: String) {
        println!("{:#?} -- {}", level, message);
    }
}
impl SockGuest for Core {
    type Sock = SockSock;
}
impl ConfigGuest for Core {
    type Config = ConfigConfig;
}

export!(Core);
