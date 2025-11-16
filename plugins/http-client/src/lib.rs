use wit_bindgen::generate;
generate!({
    world: "http-client",
    path: ["../wit/http-client.wit",],
});

use crate::exports::component::http_client::protocol::{Guest, GuestProtocolHttpClient};
use component::http_client::config::Config;
use component::http_client::flow::Res;

use component::http_client::logging::{Level, log};
struct Protocol;
struct HttpClient;
impl GuestProtocolHttpClient for HttpClient {
    fn new(_name: String) -> Self {
        HttpClient
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
    type ProtocolHttpClient = HttpClient;
}

export!(Protocol);
