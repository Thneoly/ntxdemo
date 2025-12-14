use wit_bindgen::generate;
generate!({
    world: "tcp-client",
    path: ["../wit",],
    generate_all,
});

use crate::exports::component::tcp_client::protocol::{Guest, GuestProtocolTcpClient};
// use component::tcp_client::config::Config;
// use component::tcp_client::flow::Res;

use component::tcp_client::logging::{Level, log};
use component::tcp_client::sock::Sock;
struct Protocol;
struct TcpClient {
    sock: Sock,
}

impl GuestProtocolTcpClient for TcpClient {
    fn new(_name: String) -> Self {
        TcpClient {
            sock: Sock::new("Tcp Client"),
        }
    }
    fn register(&self, res: Option<u32>, config: Option<u32>) -> u32 {
        log(
            Level::Info,
            &format!(
                "Registering TCP client with resource: {:?} and config: {:?}",
                res, config
            ),
        );
        42 // Placeholder handler ID
    }
    fn init(&self) {
        log(Level::Info, "Initializing TCP client");
    }
    fn do_action(&self) {
        self.sock.newsock(1); // 1 for TCP
        self.sock.bind(0, "127.0.0.1", 8080);
        log(Level::Info, "Connecting to server: 127.0.0.1:8080");
        self.sock.send(42, b"Hello, TCP Server!");
        let data = self.sock.recv(42, 1024);
        log(Level::Info, &format!("Received data: {:?}", data));
        log(Level::Info, "Doing TCP client action");
        self.sock.close(42);
    }
    fn release(&self) {
        log(Level::Info, "Releasing TCP client");
    }
    fn un_register(&self) {
        log(Level::Info, "Unregistering TCP client");
    }
}
impl Guest for Protocol {
    type ProtocolTcpClient = TcpClient;
}

export!(Protocol);
