use wit_bindgen::generate;
generate!({
    world: "core",
    path: ["../wit",],
    generate_all,
});
use exports::component::http_client::config::{Guest as ConfigGuest, GuestConfig};
use exports::component::http_client::flow::Workbook;
use exports::component::http_client::flow::{Guest as FlowGuest, GuestRes};
use exports::component::http_client::logging::{Guest as LoggingGuest, Level};
use exports::component::http_client::respool::{Guest as RespoolGuest, GuestScript};
use exports::component::http_client::sock::{Guest as SockGuest, GuestSock};

struct Core;
struct ConfigConfig;
struct RespoolScript;
struct FlowRes;
struct SockSock;

impl GuestScript for RespoolScript {
    fn new() -> Self {
        RespoolScript {}
    }
}
impl GuestSock for SockSock {
    fn new() -> Self {
        SockSock {}
    }
    fn newsock(&self, sock_type: u32) -> u32 {
        sock_type
    }
    fn bind(&self, _id: u32, _ip: String, _port: u16) {}
    fn send(&self, _id: u32, _data: Vec<u8>) {}
    fn recv(&self, _id: u32) -> Vec<u8> {
        vec![]
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
impl GuestRes for FlowRes {
    fn new() -> Self {
        FlowRes {}
    }
    fn execute(&self, wb: Workbook) -> Workbook {
        wb
    }
}

impl RespoolGuest for Core {
    type Script = RespoolScript;
}
impl FlowGuest for Core {
    type Res = FlowRes;
}
impl LoggingGuest for Core {
    fn log(_level: Level, _message: String) {}
}
impl SockGuest for Core {
    type Sock = SockSock;
}
impl ConfigGuest for Core {
    type Config = ConfigConfig;
}

impl Guest for Core {
    fn start() {}
}
export!(Core);
