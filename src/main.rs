use std::{env, fs, time::Duration};

use anyhow::{Context, Result, bail};

mod component_utils;
mod echo;
mod guest_packet_val;
mod network;
use component_utils::{find_iface_parent, find_top_level_func, get_func_from_iface};
use echo::{
    run_echo_client_local, run_echo_client_wasm, run_echo_server_local, run_echo_server_wasm,
};
use network::stack::{PacketContext, build_udp_reply};
use network::{MacAddr, Nic};

use wasmtime::{
    Config, Engine, Store,
    component::{Component, Instance, Linker, ResourceTable, Val},
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView, p2::add_to_linker_sync};

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
            _ => {}
        }
    }

    opt
}

fn main() -> Result<()> {
    let opt = parse_args();

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
        Mode::EchoServer => run_echo_server_wasm(&mut store, &instance, &opt),
        Mode::EchoClient => run_echo_client_wasm(&mut store, &instance, &opt),
    }
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
    let mut ctx = PacketContext::with_capacity(opt.snaplen);

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

        ctx.set_frame(&buf[..n]);
        let decoded = match ctx.decode() {
            Ok(d) => d,
            Err(_) => continue,
        };

        if debug && (rx <= 10 || rx % 200 == 0) {
            if let Some(pt) = nic.last_pkttype() {
                eprintln!("debug: rx={} pkttype={}", rx, pt);
            }
            if decoded.ip.is_none() {
                eprintln!(
                    "debug: rx={} decoded: non-ipv4 ethertype=0x{:04x}",
                    rx, decoded.eth.ethertype
                );
            } else if decoded.udp.is_none() {
                eprintln!(
                    "debug: rx={} decoded: ipv4 protocol={} (not udp)",
                    rx,
                    decoded.ip.unwrap().protocol
                );
            }
        }

        let Some(ip) = decoded.ip else {
            continue;
        };
        let Some(udp) = decoded.udp else {
            continue;
        };

        if udp.dst_port != opt.port {
            continue;
        }

        let meta_val = Val::Record(vec![
            (
                "src-mac".to_string(),
                Val::List(decoded.eth.src.0.into_iter().map(Val::U8).collect()),
            ),
            (
                "dst-mac".to_string(),
                Val::List(decoded.eth.dst.0.into_iter().map(Val::U8).collect()),
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

        let payload_val = Val::List(decoded.payload.iter().copied().map(Val::U8).collect());
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
        // Reuse build_udp_reply by temporarily swapping ctx payload view.
        let reply = {
            // build_udp_reply uses decoded.payload, so we create a small shim packet with payload pointing to new_payload
            let shim = network::stack::DecodedPacket {
                eth: decoded.eth,
                ip: decoded.ip,
                udp: decoded.udp,
                payload: &new_payload,
            };
            build_udp_reply(&shim, MacAddr(iface_mac))?
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
