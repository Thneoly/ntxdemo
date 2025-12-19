wit_bindgen::generate!({
    world: "actions-executor-server",
    path: "wit",
    generate_all,
    debug: true,
});

pub struct Server;

impl exports::scheduler::actions_executor::server::Guest for Server {
    fn on_packet_received(payload: Vec<u8>) -> Result<Vec<u8>, String> {
        // Echo Server: 简单地返回接收到的数据包作为响应
        if payload.is_empty() {
            return Err("Payload is empty".to_string());
        }

        // 直接回显：返回相同的 payload
        Ok(payload)
    }
}

export!(Server);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exports::scheduler::actions_executor::server::Guest;

    #[test]
    fn test_echo() {
        let payload = vec![1, 2, 3, 4];
        let result = Server::on_packet_received(payload.clone()).unwrap();
        assert_eq!(result, payload);
    }
}
