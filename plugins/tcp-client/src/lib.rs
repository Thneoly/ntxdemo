use wit_bindgen::generate;
generate!({
    world: "tcp-client",
    path: ["../wit",],
});

use crate::exports::component::tcp_client::protocol::{Guest, GuestProtocolTcpClient};
use component::tcp_client::config::Config;
use component::tcp_client::flow::Res;

use component::tcp_client::logging::{Level, log};
struct Protocol;
struct TcpClient;

impl GuestProtocolTcpClient for TcpClient {
    fn new(_name: String) -> Self {
        TcpClient
    }
    fn register(&self, res: Res, config: Config) -> u32 {
        log(
            Level::Info,
            &format!(
                "Registering HTTP client with resource: {:?} and config: {:?}",
                res, config
            ),
        );
        42 // Placeholder handler ID
    }
    fn init(&self) {}
    fn do_action(&self) {}
    fn release(&self) {}
    fn un_register(&self) {}
}
impl Guest for Protocol {
    type ProtocolTcpClient = TcpClient;
}

export!(Protocol);
