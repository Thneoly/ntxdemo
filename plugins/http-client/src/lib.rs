use wit_bindgen::generate;

generate!({
    world: "http-client",
    path: ["wit/world.wit",],
});
use crate::exports::component::http_client::protocol::Guest;
use component::http_client::logging::{Level, log};
struct Protocol;
impl Guest for Protocol {
    fn register() -> String {
        log(Level::Info, "Registering Protocol");
        "Protocol Registered".to_string()
    }
    fn init() -> String {
        log(Level::Info, "Initializing Protocol");
        "Protocol Initialized".to_string()
    }
    fn do_action() -> String {
        log(Level::Info, "Doing Protocol Action");
        "Protocol Action Done".to_string()
    }
    fn release() -> String {
        log(Level::Info, "Releasing Protocol");
        "Protocol Released".to_string()
    }
    fn un_register() -> String {
        log(Level::Info, "Unregistering Protocol");
        "Protocol Unregistered".to_string()
    }

}

export!(Protocol);