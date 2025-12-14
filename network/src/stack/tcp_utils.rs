use anyhow::Result;

use crate::{ETH_TYPE_IPV4, EthernetHeader, Ipv4Addr, Ipv4Header, MacAddr, TcpHeader};

use super::ReplyFrame;

/// Build a minimal IPv4/TCP frame.
///
/// Notes:
/// - IPv4 header IHL is fixed at 5 (no options).
/// - TCP options are supported via `TcpHeader.options` (must match `data_offset_words`).
/// - TCP checksum is computed in `TcpHeader::write`.
pub fn build_tcp_frame(
    src_mac: MacAddr,
    dst_mac: MacAddr,
    src_ip: Ipv4Addr,
    dst_ip: Ipv4Addr,
    tcp: &TcpHeader,
    payload: &[u8],
) -> Result<Vec<u8>> {
    let eth_len = EthernetHeader::LEN;
    let ip_len = Ipv4Header::MIN_LEN;
    let tcp_len = tcp.header_len();

    let mut bytes = vec![0u8; eth_len + ip_len + tcp_len + payload.len()];

    let eth = EthernetHeader {
        dst: dst_mac,
        src: src_mac,
        ethertype: ETH_TYPE_IPV4,
    };
    eth.write(&mut bytes[..eth_len])?;

    let ip = Ipv4Header {
        src: src_ip,
        dst: dst_ip,
        protocol: 6,
        ttl: 64,
        identification: 0,
        flags_fragment: 0,
    };
    ip.write(
        &mut bytes[eth_len..eth_len + ip_len],
        tcp_len + payload.len(),
        0,
    )?;

    let tcp_off = eth_len + ip_len;
    tcp.write(
        &mut bytes[tcp_off..tcp_off + tcp_len + payload.len()],
        payload,
        src_ip,
        dst_ip,
    )?;

    Ok(bytes)
}

/// Build a reply TCP frame by swapping MAC/IP/ports.
///
/// The caller provides the new TCP header (seq/ack/flags etc) and payload.
pub fn build_tcp_reply(
    rx_eth: &EthernetHeader,
    rx_ip: &Ipv4Header,
    iface_mac: MacAddr,
    tcp: &TcpHeader,
    payload: &[u8],
) -> Result<ReplyFrame> {
    let bytes = build_tcp_frame(iface_mac, rx_eth.src, rx_ip.dst, rx_ip.src, tcp, payload)?;
    Ok(ReplyFrame { bytes })
}
