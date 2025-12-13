use std::time::Duration;

use ntx::network::Ipv4Addr;
use ntx::network::traffic::matcher::{FlowKey, Matcher};
use ntx::network::traffic::token::{TOKEN_LEN, Token, decode_token, encode_token};

#[test]
fn token_encode_decode_roundtrip() {
    let b = encode_token(42);
    assert_eq!(b.len(), TOKEN_LEN);
    let t = decode_token(&b).unwrap();
    assert_eq!(t, Token(42));
}

#[test]
fn matcher_insert_match_and_timeout() {
    let mut m = Matcher::new(Duration::from_millis(5));
    let key = FlowKey {
        dst_ip: Ipv4Addr([10, 0, 0, 1]),
        dst_port: 10001,
        src_port: 40000,
        token: Token(1),
    };

    m.insert(key);
    assert_eq!(m.outstanding(), 1);

    // Match should remove.
    m.on_reply(key);
    assert_eq!(m.outstanding(), 0);
    assert_eq!(m.stats.matched, 1);

    // Timeout path.
    let key2 = FlowKey {
        dst_ip: Ipv4Addr([10, 0, 0, 2]),
        dst_port: 10001,
        src_port: 40000,
        token: Token(2),
    };
    m.insert(key2);
    std::thread::sleep(Duration::from_millis(8));
    m.sweep_timeouts();
    assert_eq!(m.outstanding(), 0);
    assert_eq!(m.stats.timeouts, 1);
}
