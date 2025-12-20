use std::{env, fs, time::Duration};

use anyhow::{Context, Result, bail};

mod component_utils;
mod echo;
mod guest_packet_val;
mod network;
use component_utils::{find_iface_parent, find_top_level_func, get_func_from_iface};
use echo::{run_echo_client_local, run_echo_server_local};
use network::stack::PacketContext;
use network::traffic::udp_echo::build_udp_reply;
use network::{MacAddr, Nic};

use wasmtime::{
    Config, Engine, Store,
    component::{Component, Instance, Linker, ResourceTable, Val},
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView, p2::add_to_linker_sync};

#[cfg(feature = "scheduler-component")]
wasmtime::component::bindgen!({
    world: "scheduler:main/scheduler-component",
    path: [
        "plugins/scheduler/wit/core",
        "plugins/scheduler/wit/eventbus",
        "plugins/scheduler/wit/protocol",
        "plugins/scheduler/wit/net",
        "plugins/scheduler/wit/scheduler",
    ],
});
struct State {
    wasi: WasiCtx,
    table: ResourceTable,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Net,
    Scenario,
    EchoServer,
    EchoClient,
    TcpClient,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    AfPacket,
    AfPacketDgram,
    TpacketV3,
}

#[derive(Debug, Clone)]
struct Opt {
    mode: Mode,
    iface: String,
    backend: Backend,
    /// Only process UDP packets whose dst_port matches this port.
    port: u16,
    /// recv() buffer size
    snaplen: usize,
    /// Scheduler composed component path
    component_path: String,
    /// Scenario yaml path (scenario mode)
    scenario_path: String,
    /// Server IP for client mode
    server_ip: String,
    /// Server port for client mode
    server_port: u16,
    /// Number of requests for client mode
    client_count: u32,
    /// Packets per second for client mode
    pps: u32,

    /// Local TCP source port (tcp-client mode)
    tcp_local_port: u16,
    /// Remote TCP destination port (tcp-client mode)
    tcp_remote_port: u16,
    /// Initial sequence number (tcp-client mode)
    tcp_isn: u32,
    /// Optional payload string to send after handshake (tcp-client mode)
    tcp_payload: String,
}

fn parse_args() -> Opt {
    let default_scenario = "plugins/scheduler/res/simple_scenario.yaml";
    let default_component = "plugins/scheduler/wac/scheduler-composed.wasm";

    let mut opt = Opt {
        mode: Mode::Net,
        iface: "eno1".to_string(),
        backend: Backend::AfPacket,
        port: 10001,
        snaplen: 2048,
        component_path: env::var("SCHEDULER_COMPONENT")
            .unwrap_or_else(|_| default_component.into()),
        scenario_path: default_scenario.to_string(),
        server_ip: "10.0.0.1".to_string(),
        server_port: 10001,
        client_count: 100,
        pps: 50,

        tcp_local_port: 40000,
        tcp_remote_port: 80,
        tcp_isn: 100,
        tcp_payload: "hello".to_string(),
    };

    let mut it = env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                eprintln!(
                    "Usage: ntx [--mode net|scenario|server|client] [--iface IFACE] [--backend afpacket|afpacket-dgram|tpacketv3] [--port PORT] [--snaplen N] [--component PATH] [--scenario PATH] [--server-ip IP] [--server-port PORT] [--count N] [--pps N]\n\nDefault: --mode net --iface eno1 --backend afpacket --port 10001 --snaplen 2048"
                );
                std::process::exit(0);
            }
            "--mode" => {
                if let Some(v) = it.next() {
                    opt.mode = match v.as_str() {
                        "net" => Mode::Net,
                        "scenario" => Mode::Scenario,
                        "server" => Mode::EchoServer,
                        "client" => Mode::EchoClient,
                        "tcp-client" | "tcp_client" | "tcp" => Mode::TcpClient,
                        _ => {
                            eprintln!("invalid --mode: {v} (expected: net|scenario|server|client)");
                            std::process::exit(2);
                        }
                    }
                }
            }
            "--iface" => {
                if let Some(v) = it.next() {
                    opt.iface = v;
                }
            }
            "--backend" => {
                if let Some(v) = it.next() {
                    opt.backend = match v.as_str() {
                        "afpacket" => Backend::AfPacket,
                        "afpacket-dgram" | "afpacket_dgram" | "cooked" => Backend::AfPacketDgram,
                        "tpacketv3" | "tpacket_v3" | "tpv3" => Backend::TpacketV3,
                        _ => {
                            eprintln!(
                                "invalid --backend: {v} (expected: afpacket|afpacket-dgram|tpacketv3)"
                            );
                            std::process::exit(2);
                        }
                    }
                }
            }
            "--port" => {
                if let Some(v) = it.next() {
                    if let Ok(p) = v.parse::<u16>() {
                        opt.port = p;
                    }
                }
            }
            "--snaplen" => {
                if let Some(v) = it.next() {
                    if let Ok(n) = v.parse::<usize>() {
                        opt.snaplen = n;
                    }
                }
            }
            "--component" => {
                if let Some(v) = it.next() {
                    opt.component_path = v;
                }
            }
            "--scenario" => {
                if let Some(v) = it.next() {
                    opt.scenario_path = v;
                }
            }
            "--server-ip" => {
                if let Some(v) = it.next() {
                    opt.server_ip = v;
                }
            }
            "--server-port" => {
                if let Some(v) = it.next() {
                    if let Ok(p) = v.parse::<u16>() {
                        opt.server_port = p;
                    }
                }
            }
            "--count" => {
                if let Some(v) = it.next() {
                    if let Ok(c) = v.parse::<u32>() {
                        opt.client_count = c;
                    }
                }
            }
            "--pps" => {
                if let Some(v) = it.next() {
                    if let Ok(p) = v.parse::<u32>() {
                        opt.pps = p;
                    }
                }
            }
            "--tcp-local-port" => {
                if let Some(v) = it.next() {
                    if let Ok(p) = v.parse::<u16>() {
                        opt.tcp_local_port = p;
                    }
                }
            }
            "--tcp-remote-port" => {
                if let Some(v) = it.next() {
                    if let Ok(p) = v.parse::<u16>() {
                        opt.tcp_remote_port = p;
                    }
                }
            }
            "--tcp-isn" => {
                if let Some(v) = it.next() {
                    if let Ok(n) = v.parse::<u32>() {
                        opt.tcp_isn = n;
                    }
                }
            }
            "--tcp-payload" => {
                if let Some(v) = it.next() {
                    opt.tcp_payload = v;
                }
            }
            _ => {}
        }
    }

    opt
}

fn main() -> Result<()> {
    let opt = parse_args();

    // tcp-client is a pure host-mode runner; no WASM component needed.
    if opt.mode == Mode::TcpClient {
        #[cfg(feature = "tcp-client")]
        {
            return run_tcp_client_mode(&opt);
        }
        #[cfg(not(feature = "tcp-client"))]
        {
            bail!("tcp-client mode is disabled (build with --features tcp-client)");
        }
    }

    // 设置 WASM 配置
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.async_support(false);

    let engine = Engine::new(&config)?;

    // 根据模式选择 WASM 组件路径
    let component_path = match opt.mode {
        Mode::EchoServer => "plugins/scheduler/wac/echo-server.wasm".to_string(),
        Mode::EchoClient => "plugins/scheduler/wac/echo-client.wasm".to_string(),
        _ => opt.component_path.clone(),
    };

    let mut store = Store::new(
        &engine,
        State {
            wasi: WasiCtxBuilder::new()
                .inherit_stdio()
                .inherit_network()
                .build(),
            table: ResourceTable::default(),
        },
    );
    let mut linker: Linker<State> = Linker::new(&engine);
    add_to_linker_sync(&mut linker)?;

    let component_path_display = component_path.clone();

    // Echo 模式特殊处理：如果 WASM 加载失败，回退到本地实现
    let load_result = Component::from_file(&engine, &component_path);

    let instance = match load_result {
        Ok(component) => match linker.instantiate(&mut store, &component) {
            Ok(inst) => inst,
            Err(e) => {
                eprintln!("[WASM] instantiate failed: {}, falling back to native", e);
                match opt.mode {
                    Mode::EchoServer => return run_echo_server_local(&opt),
                    Mode::EchoClient => return run_echo_client_local(&opt),
                    _ => bail!("failed to instantiate component: {}", e),
                }
            }
        },
        Err(e) => {
            eprintln!("[WASM] load failed: {}, falling back to native", e);
            match opt.mode {
                Mode::EchoServer => return run_echo_server_local(&opt),
                Mode::EchoClient => return run_echo_client_local(&opt),
                _ => bail!("载入组件失败: {}", component_path_display),
            }
        }
    };

    match opt.mode {
        Mode::Scenario => run_scenario_mode(&mut store, &instance, &opt),
        Mode::Net => run_net_mode(&mut store, &instance, &opt),
        Mode::EchoServer => {
            #[cfg(feature = "scheduler-component")]
            {
                return echo::run_echo_server_wasm(&mut store, &instance, &opt);
            }
            #[cfg(not(feature = "scheduler-component"))]
            {
                return run_echo_server_local(&opt);
            }
        }
        Mode::EchoClient => {
            #[cfg(feature = "scheduler-component")]
            {
                return echo::run_echo_client_wasm(&mut store, &instance, &opt);
            }
            #[cfg(not(feature = "scheduler-component"))]
            {
                return run_echo_client_local(&opt);
            }
        }
        Mode::TcpClient => unreachable!(),
    }
}

#[cfg(feature = "tcp-client")]
fn run_tcp_client_mode(opt: &Opt) -> Result<()> {
    use std::time::{Duration, Instant};

    use crate::network::{MacAddr, Nic, TcpClient, TcpClientState, TcpFlags, TcpSegment};

    // Basic assumptions:
    // - L2 dst is broadcast (no ARP yet)
    // - src IP from env NTX_CLIENT_IP (same convention as echo client)
    // - dst IP from --server-ip
    // This is intentionally minimal and meant for netns lab setups.
    let mut nic: Box<dyn Nic> = match opt.backend {
        Backend::AfPacket => {
            Box::new(network::AfPacketNic::open(&opt.iface).context("open afpacket nic")?)
        }
        Backend::AfPacketDgram => Box::new(
            network::AfPacketDgramNic::open(&opt.iface).context("open afpacket-dgram nic")?,
        ),
        Backend::TpacketV3 => Box::new(
            network::TpacketV3Nic::open(&opt.iface, 1 << 20, 64, opt.snaplen as u32, 10)
                .context("open tpacketv3 nic")?,
        ),
    };

    let iface_mac = nic.iface_mac().context("failed to query iface mac")?;
    let src_ip_str = std::env::var("NTX_CLIENT_IP").unwrap_or_else(|_| "10.0.0.2".to_string());
    let src_ip = parse_ipv4_local(&src_ip_str).context("invalid NTX_CLIENT_IP/ default src ip")?;
    let dst_ip = parse_ipv4_local(&opt.server_ip).context("invalid --server-ip")?;

    // Minimal ARP resolve: ask the peer's MAC before sending SYN.
    // This avoids relying on L2 broadcast working for IPv4/TCP frames.
    let dst_mac = resolve_arp_minimal(
        &mut *nic,
        MacAddr(iface_mac),
        src_ip,
        dst_ip,
        opt.snaplen,
        Duration::from_secs(2),
    )
    .with_context(|| {
        format!(
            "ARP resolve failed for {}.{}.{}.{} (try ping/arp in netns or check iface)",
            dst_ip.0[0], dst_ip.0[1], dst_ip.0[2], dst_ip.0[3]
        )
    })?;

    eprintln!(
        "ntx(tcp-client) starting: iface={} backend={:?} {}:{} -> {}:{} isn={} payload_len={} dst-mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        opt.iface,
        opt.backend,
        src_ip.0[0],
        opt.tcp_local_port,
        opt.server_ip,
        opt.tcp_remote_port,
        opt.tcp_isn,
        opt.tcp_payload.as_bytes().len(),
        dst_mac.0[0],
        dst_mac.0[1],
        dst_mac.0[2],
        dst_mac.0[3],
        dst_mac.0[4],
        dst_mac.0[5]
    );

    let mut ctx = PacketContext::default();
    let mut buf = vec![0u8; opt.snaplen];

    let mut c = TcpClient::new(opt.tcp_local_port, opt.tcp_remote_port, opt.tcp_isn);

    // 1) Send SYN
    let syn = c.connect()?;
    send_tcp_segment(&mut *nic, MacAddr(iface_mac), dst_mac, src_ip, dst_ip, syn)?;

    let start = Instant::now();
    let mut sent_data = false;
    let mut sent_fin = false;

    loop {
        if start.elapsed() > Duration::from_secs(10) {
            bail!("tcp-client timeout");
        }

        let _ = nic.poll_readable(Some(Duration::from_millis(200)))?;

        if let Some(n) = nic.recv_nonblocking(&mut buf)? {
            ctx.set_frame(&buf[..n]);
            let decoded = match ctx.decode() {
                Ok(d) => d,
                Err(_) => continue,
            };

            let Some(ip) = decoded.ip else { continue };
            if ip.protocol != 6 {
                continue;
            }
            let Some(tcp) = decoded.tcp.as_ref() else {
                continue;
            };
            if ip.src != dst_ip || ip.dst != src_ip {
                continue;
            }
            if tcp.src_port != opt.tcp_remote_port || tcp.dst_port != opt.tcp_local_port {
                continue;
            }

            let inbound = TcpSegment {
                hdr: tcp.clone(),
                payload: decoded.payload.to_vec(),
            };
            if let Some(out) = c.on_segment(inbound)? {
                send_tcp_segment(&mut *nic, MacAddr(iface_mac), dst_mac, src_ip, dst_ip, out)?;
            }

            if c.state == TcpClientState::Established && !sent_data {
                let data = c.send_data(opt.tcp_payload.as_bytes())?;
                send_tcp_segment(&mut *nic, MacAddr(iface_mac), dst_mac, src_ip, dst_ip, data)?;
                sent_data = true;
            }

            if sent_data && !sent_fin {
                // Heuristic: once we see remote ACK for our data (or any remote data), initiate close.
                if tcp.flags.contains(TcpFlags::ACK) {
                    let fin = c.close()?;
                    send_tcp_segment(&mut *nic, MacAddr(iface_mac), dst_mac, src_ip, dst_ip, fin)?;
                    sent_fin = true;
                }
            }

            if c.state == TcpClientState::TimeWait {
                eprintln!("ntx(tcp-client) done: TIME_WAIT");
                return Ok(());
            }
        }
    }
}

#[cfg(not(feature = "tcp-client"))]
#[allow(dead_code)]
fn run_tcp_client_mode(_opt: &Opt) -> Result<()> {
    bail!("tcp-client mode is disabled (build with --features tcp-client)")
}

fn resolve_arp_minimal(
    nic: &mut dyn crate::network::Nic,
    src_mac: crate::network::MacAddr,
    src_ip: crate::network::Ipv4Addr,
    target_ip: crate::network::Ipv4Addr,
    snaplen: usize,
    timeout: Duration,
) -> Result<crate::network::MacAddr> {
    use crate::network::arp::{build_arp_request_frame, parse_arp_reply};

    // Best-effort: try a couple times within timeout.
    let start = std::time::Instant::now();
    let mut buf = vec![0u8; snaplen.max(64)];

    // Pre-send once, then re-send every 250ms.
    let mut last_send = std::time::Instant::now() - Duration::from_secs(3600);

    loop {
        if start.elapsed() > timeout {
            bail!("arp timeout");
        }

        if last_send.elapsed() >= Duration::from_millis(250) {
            let req = build_arp_request_frame(src_mac, src_ip, target_ip)?;
            nic.send(&req).context("send arp request")?;
            last_send = std::time::Instant::now();
        }

        // Wait a bit for readability.
        let _ = nic.poll_readable(Some(Duration::from_millis(200)))?;

        // Drain all available frames so we don't miss the ARP reply.
        while let Some(n) = nic.recv_nonblocking(&mut buf)? {
            if let Some((sip, smac)) = parse_arp_reply(&buf[..n])? {
                if sip == target_ip {
                    return Ok(smac);
                }
            }
        }
    }
}

fn parse_ipv4_local(s: &str) -> Result<crate::network::Ipv4Addr> {
    let parts: Vec<_> = s.split('.').collect();
    if parts.len() != 4 {
        bail!("invalid ipv4: {s}");
    }
    let mut octets = [0u8; 4];
    for (i, p) in parts.iter().enumerate() {
        octets[i] = p
            .parse::<u8>()
            .map_err(|_| anyhow::anyhow!("invalid ipv4: {s}"))?;
    }
    Ok(crate::network::Ipv4Addr(octets))
}

#[cfg(feature = "tcp-client")]
fn send_tcp_frame(
    nic: &mut dyn Nic,
    iface_mac: MacAddr,
    dst_mac: MacAddr,
    src_ip: crate::network::Ipv4Addr,
    dst_ip: crate::network::Ipv4Addr,
    seg: crate::network::TcpSegment,
) -> bool {
    let frame = crate::network::stack::build_tcp_frame(iface_mac, dst_mac, src_ip, dst_ip, seg);
    nic.send(&frame).is_ok()
}

#[cfg(not(feature = "tcp-client"))]
#[allow(dead_code)]
fn send_tcp_frame(
    _nic: &mut dyn Nic,
    _iface_mac: MacAddr,
    _dst_mac: MacAddr,
    _src_ip: crate::network::Ipv4Addr,
    _dst_ip: crate::network::Ipv4Addr,
    _seg: (),
) -> bool {
    false
}

fn run_scenario_mode(store: &mut Store<State>, instance: &Instance, opt: &Opt) -> Result<()> {
    let scenario = fs::read_to_string(&opt.scenario_path)
        .with_context(|| format!("读取场景文件失败: {}", opt.scenario_path))?;

    println!(
        "开始执行 run-scenario，输入 YAML 长度 {} 字节",
        scenario.len()
    );

    let func = find_top_level_func(store, instance, &["run-scenario"])?;
    let typed = func
        .typed::<(&str,), (Result<String, String>,)>(&*store)
        .context("run-scenario 签名检查失败")?;

    match typed.call(store, (&scenario,))?.0 {
        Ok(summary) => println!("✅ 执行成功: {summary}"),
        Err(err) => println!("❌ 执行失败: {err}"),
    };
    Ok(())
}

fn run_net_mode(store: &mut Store<State>, instance: &Instance, opt: &Opt) -> Result<()> {
    use std::io::Write as _;

    let debug = std::env::var("NTX_DEBUG").ok().as_deref() == Some("1");

    // One-line banner for scripts/runbooks.
    // Print to stderr so stdout can remain clean if needed.
    eprintln!(
        "ntx(net) starting: iface={} backend={:?} port={} snaplen={} component={}",
        opt.iface, opt.backend, opt.port, opt.snaplen, opt.component_path
    );
    let _ = std::io::stderr().flush();

    let mut nic: Box<dyn Nic> = match opt.backend {
        Backend::AfPacket => {
            Box::new(network::AfPacketNic::open(&opt.iface).context("open afpacket nic")?)
        }
        Backend::AfPacketDgram => Box::new(
            network::AfPacketDgramNic::open(&opt.iface).context("open afpacket-dgram nic")?,
        ),
        Backend::TpacketV3 => Box::new(
            network::TpacketV3Nic::open(&opt.iface, 1 << 20, 64, opt.snaplen as u32, 10)
                .context("open tpacketv3 nic")?,
        ),
    };

    let iface_mac = nic
        .iface_mac()
        .context("failed to query iface mac (SIOCGIFHWADDR); are you running as root?")?;

    // Locate packet::on-udp, if exported.
    // Note: after WAC composition, the export is often the fully-qualified interface name
    // (e.g. `scheduler:net/packet@0.1.0`) instead of the short alias `packet`.
    let parent = find_iface_parent(
        store,
        instance,
        &[
            "packet",
            "scheduler:net/packet@0.1.0",
            "scheduler:net/packet",
        ],
    )
    .context("missing exported interface instance 'packet'")?;
    let on_udp = get_func_from_iface(store, instance, &parent, "on-udp")
        .ok_or_else(|| anyhow::anyhow!("missing func export packet/on-udp"))?;

    eprintln!(
        "ntx(net): iface={} ifindex={} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} port={} backend={:?}",
        nic.ifname(),
        nic.ifindex(),
        iface_mac[0],
        iface_mac[1],
        iface_mac[2],
        iface_mac[3],
        iface_mac[4],
        iface_mac[5],
        opt.port,
        opt.backend
    );

    let mut buf = vec![0u8; opt.snaplen];
    let mut ctx = PacketContext::default();

    let mut rx: u64 = 0;
    let mut decoded_udp: u64 = 0;
    let mut guest_ok: u64 = 0;
    let mut guest_err: u64 = 0;
    let mut sent: u64 = 0;

    let mut last_report = std::time::Instant::now();
    let report_iv = Duration::from_secs(1);

    loop {
        // For ring backends, poll avoids busy loop.
        let _ = nic.poll_readable(Some(Duration::from_millis(100)))?;

        let n = match nic.recv_nonblocking(&mut buf)? {
            Some(n) => n,
            None => {
                if last_report.elapsed() >= report_iv {
                    eprintln!(
                        "stats: rx={} udp={} guest_ok={} guest_err={} sent={}",
                        rx, decoded_udp, guest_ok, guest_err, sent
                    );
                    last_report = std::time::Instant::now();
                }
                continue;
            }
        };

        rx = rx.wrapping_add(1);

        // Parse using the stack parser. When ctx.abr is None, accept() is permissive.
        let (layers, payload) = match network::stack::parse_packet_with_ctx(
            &buf[..n],
            network::stack::LayerId::Ether,
            &network::stack::default_registry(),
            &ctx,
        ) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let Some(ip) = layers
            .iter()
            .find_map(|l| l.downcast_ref::<network::packet::layers::Ipv4>())
        else {
            continue;
        };
        let Some(udp) = layers
            .iter()
            .find_map(|l| l.downcast_ref::<network::packet::layers::Udp>())
        else {
            continue;
        };

        if debug && (rx <= 10 || rx % 200 == 0) {
            if let Some(pt) = nic.last_pkttype() {
                eprintln!("debug: rx={} pkttype={}", rx, pt);
            }
        }

        if udp.dst_port != opt.port {
            continue;
        }

        // Extract Ether layer for metadata.
        let Some(eth) = layers
            .iter()
            .find_map(|l| l.downcast_ref::<network::packet::layers::Ether>())
        else {
            continue;
        };

        let meta_val = Val::Record(vec![
            (
                "src-mac".to_string(),
                Val::List(eth.src.0.into_iter().map(Val::U8).collect()),
            ),
            (
                "dst-mac".to_string(),
                Val::List(eth.dst.0.into_iter().map(Val::U8).collect()),
            ),
            (
                "src-ip".to_string(),
                Val::Record(vec![
                    ("a".to_string(), Val::U8(ip.src.0[0])),
                    ("b".to_string(), Val::U8(ip.src.0[1])),
                    ("c".to_string(), Val::U8(ip.src.0[2])),
                    ("d".to_string(), Val::U8(ip.src.0[3])),
                ]),
            ),
            (
                "dst-ip".to_string(),
                Val::Record(vec![
                    ("a".to_string(), Val::U8(ip.dst.0[0])),
                    ("b".to_string(), Val::U8(ip.dst.0[1])),
                    ("c".to_string(), Val::U8(ip.dst.0[2])),
                    ("d".to_string(), Val::U8(ip.dst.0[3])),
                ]),
            ),
            ("src-port".to_string(), Val::U16(udp.src_port)),
            ("dst-port".to_string(), Val::U16(udp.dst_port)),
            ("rx-ifindex".to_string(), Val::U32(nic.ifindex() as u32)),
        ]);

        let payload_val = Val::List(payload.iter().copied().map(Val::U8).collect());
        let mut results = [Val::Bool(false)];
        if let Err(e) = on_udp.call(&mut *store, &[meta_val, payload_val], &mut results) {
            guest_err = guest_err.wrapping_add(1);
            if debug {
                eprintln!("guest call failed: {e:#}");
            }
            // If the call failed, we can't trust `results`; skip this packet.
            continue;
        }
        let _ = on_udp.post_return(&mut *store);

        decoded_udp = decoded_udp.wrapping_add(1);

        let maybe_payload: Option<Vec<u8>> = match guest_packet_val::parse_on_udp_result(&results) {
            Ok(v) => {
                guest_ok = guest_ok.wrapping_add(1);
                v
            }
            Err(msg) => {
                guest_err = guest_err.wrapping_add(1);
                if debug {
                    eprintln!("guest err: {msg}");
                }
                None
            }
        };

        let Some(new_payload) = maybe_payload else {
            if last_report.elapsed() >= report_iv {
                eprintln!(
                    "stats: rx={} udp={} guest_ok={} guest_err={} sent={}",
                    rx, decoded_udp, guest_ok, guest_err, sent
                );
                last_report = std::time::Instant::now();
            }
            continue;
        };

        // Build reply frame with our iface MAC and the new payload.
        // Reuse build_udp_reply by constructing a ParsedPacket view from layers.
        let reply = {
            let pkt = network::stack::ParsedPacket {
                layers,
                payload: &new_payload,
            };
            build_udp_reply(&pkt, MacAddr(iface_mac))?
        };

        if nic.send(&reply.bytes).is_ok() {
            sent = sent.wrapping_add(1);
        }

        if last_report.elapsed() >= report_iv {
            eprintln!(
                "stats: rx={} udp={} guest_ok={} guest_err={} sent={}",
                rx, decoded_udp, guest_ok, guest_err, sent
            );
            last_report = std::time::Instant::now();
        }
    }
}
