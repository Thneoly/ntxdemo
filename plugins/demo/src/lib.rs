use wit_bindgen::generate;
generate!({
    world: "demo",
    path: ["../wit",],
    generate_all,
});

use component::tcp_client::protocol::ProtocolTcpClient;
struct Demo;

impl Guest for Demo {
    fn start() {
        let client = ProtocolTcpClient::new("DemoTcp");
        let _ = client.register(None, None);
        client.init();
        client.do_action();
        client.release();
        client.un_register();
    }
}

export!(Demo);
