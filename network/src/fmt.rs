//! Tiny formatting helpers/macros.
//!
//! These exist mostly to keep examples readable.

/// Format an IPv4 address as dotted quad.
///
/// Usage:
/// ```ignore
/// eprintln!("dst={}", fmt_ipv4!(ip));
/// ```
#[macro_export]
macro_rules! fmt_ipv4 {
    ($ip:expr) => {{
        let o = ($ip).0;
        format!("{}.{}.{}.{}", o[0], o[1], o[2], o[3])
    }};
}

/// Format a MAC address as hex with colons.
///
/// Usage:
/// ```ignore
/// eprintln!("mac={}", fmt_mac!(mac));
/// ```
#[macro_export]
macro_rules! fmt_mac {
    ($mac:expr) => {{
        let o = ($mac).0;
        format!(
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            o[0], o[1], o[2], o[3], o[4], o[5]
        )
    }};
}

/// Format an ARP header in a human-friendly single line.
///
/// Usage:
/// ```ignore
/// eprintln!("{}", fmt_arp!(arp));
/// ```
#[macro_export]
macro_rules! fmt_arp {
    ($arp:expr) => {{
        format!(
            "oper={} sha={} spa={} tha={} tpa={}",
            ($arp).oper,
            $crate::fmt_mac!(($arp).sha),
            $crate::fmt_ipv4!(($arp).spa),
            $crate::fmt_mac!(($arp).tha),
            $crate::fmt_ipv4!(($arp).tpa)
        )
    }};
}

/// Format a combined Ether + ARP line.
///
/// Usage:
/// ```ignore
/// if let Some(eth) = eth_l2 {
///     eprintln!("[arp rx] {}", fmt_ether_arp!(eth, arp));
/// } else {
///     eprintln!("[arp rx] {}", fmt_arp!(arp));
/// }
/// ```
#[macro_export]
macro_rules! fmt_ether_arp {
    ($eth:expr, $arp:expr) => {{
        format!(
            "l2 {} -> {}  {}",
            $crate::fmt_mac!(($eth).src),
            $crate::fmt_mac!(($eth).dst),
            $crate::fmt_arp!($arp)
        )
    }};
}
