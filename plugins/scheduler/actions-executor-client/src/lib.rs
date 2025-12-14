wit_bindgen::generate!({
    world: "actions-executor-client",
    path: "wit",
    generate_all,
    debug: true,
});

pub struct Client;

impl exports::scheduler::actions_executor::client::Guest for Client {
    fn generate(count: u32, pps: u32) -> Result<u32, String> {
        // Echo Client: 生成指定数量的请求
        if count == 0 {
            return Err("Count must be greater than 0".to_string());
        }
        if pps == 0 {
            return Err("PPS must be greater than 0".to_string());
        }

        // 返回实际发送的包数（这里简化为 count）
        Ok(count)
    }

    fn build_payload(seq: u32) -> Result<Vec<u8>, String> {
        let mut payload = Vec::with_capacity(4 + 16);
        payload.extend_from_slice(&seq.to_be_bytes());
        payload.extend_from_slice(b"Echo request data");
        Ok(payload)
    }

    fn validate_reply(seq: u32, payload: Vec<u8>) -> Result<bool, String> {
        if payload.len() < 4 {
            return Ok(false);
        }
        let seq_bytes = [payload[0], payload[1], payload[2], payload[3]];
        let seq_recv = u32::from_be_bytes(seq_bytes);
        Ok(seq_recv == seq)
    }
}

export!(Client);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate() {
        let result = Client::generate(10, 5).unwrap();
        assert_eq!(result, 10);
    }

    #[test]
    fn test_generate_invalid() {
        assert!(Client::generate(0, 5).is_err());
        assert!(Client::generate(10, 0).is_err());
    }
}
