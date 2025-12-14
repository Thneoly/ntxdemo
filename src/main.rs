use std::{env, fs, time::Duration};

use anyhow::{Context, Result, bail};

mod guest_packet_val;
mod network;
use network::stack::{PacketContext, build_udp_reply};
use network::{EthernetHeader, Ipv4Addr, Ipv4Header, MacAddr, Nic, UdpHeader};

use wasmtime::{
    Config, Engine, Store,
    component::{
        Component, ComponentExportIndex, Func, Instance, Linker, ResourceTable, Val,
        types::ComponentItem,
    },
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

// 顶层找接口导出的"父索引"，用于进入接口命名空间
#[allow(unused)]
fn find_iface_parent(
    store: &mut Store<State>,
    inst: &Instance,
    candidates: &[&str],
) -> Result<ComponentExportIndex> {
    for name in candidates {
        if let Some((item, idx)) = inst.get_export(&mut *store, None, name) {
            if matches!(item, ComponentItem::ComponentInstance(_)) {
                return Ok(idx);
            } else {
                println!("找到非接口导出：{:#?}", item);
            }
        }
    }
    bail!(
        "找不到接口导出：候选 = {candidates:?}\n请用 `wasm-tools component wit demo.wasm` 查看实际导出名/版本，并在 WAC 顶层正确 `export`。"
    );
}

// 顶层函数查找：在顶层导出中按候选名查找 func
#[allow(unused)]
fn find_top_level_func(
    store: &mut Store<State>,
    inst: &Instance,
    candidates: &[&str],
) -> Result<Func> {
    for name in candidates {
        if let Some((item, idx)) = inst.get_export(&mut *store, None, name) {
            if matches!(item, ComponentItem::ComponentFunc(_)) {
                if let Some(f) = inst.get_func(&mut *store, idx) {
                    return Ok(f);
                }
            }
        }
    }
    bail!(
        "找不到顶层函数导出：候选 = {candidates:?}。请用 `wasm-tools component wit <你的 wasm>` 确认实际导出名。"
    );
}

// 从接口命名空间获取函数
#[allow(unused)]
fn get_func_from_iface(
    store: &mut Store<State>,
    inst: &Instance,
    parent: &ComponentExportIndex,
    func_name: &str,
) -> Option<Func> {
    let (_item, func_idx) = inst.get_export(&mut *store, Some(parent), func_name)?;
    inst.get_func(&mut *store, func_idx)
}

fn run_echo_server_wasm(store: &mut Store<State>, instance: &Instance, opt: &Opt) -> Result<()> {
    eprintln!(
        "ntx(echo-server-wasm) starting: iface={} port={} component={} wasm_path={}",
        opt.iface, opt.port, "echo-server.wasm", "plugins/scheduler/wac/echo-server.wasm"
    );

    // Echo server WASM v2组件按 WIT 导出的是一个接口实例 `server`，函数名 `on-packet-received`。
    // 所以这里要先进入接口命名空间，再取函数。
    let parent = match find_iface_parent(
        store,
        instance,
        &[
            "server",
            "scheduler:actions-executor/server@0.1.0",
            "scheduler:actions-executor/server",
        ],
    ) {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "[wasm] No exported interface instance 'server': {e}. Falling back to native"
            );
            return run_echo_server_native(store, instance, opt);
        }
    };
    let Some(on_packet_received) =
        get_func_from_iface(store, instance, &parent, "on-packet-received")
    else {
        eprintln!("[wasm] Missing func export server/on-packet-received. Falling back to native");
        return run_echo_server_native(store, instance, opt);
    };

    eprintln!("[wasm] Echo server export resolved: server/on-packet-received");

    // 运行网络收包循环，但使用 WASM 导出处理 payload，再由 host 负责构造 UDP 回复并发送。
    run_echo_server_wasm_loop(store, &on_packet_received, opt)
}

fn run_echo_server_wasm_loop(
    store: &mut Store<State>,
    on_packet_received: &Func,
    opt: &Opt,
) -> Result<()> {
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

    eprintln!(
        "[echo-server/wasm] iface={} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} port={} backend={:?}",
        nic.ifname(),
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
        let _ = nic.poll_readable(Some(Duration::from_millis(100)))?;

        let n = match nic.recv_nonblocking(&mut buf)? {
            Some(n) => n,
            None => {
                if last_report.elapsed() >= report_iv {
                    eprintln!(
                        "[echo-server/wasm] stats: rx={} udp={} guest_ok={} guest_err={} sent={}",
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

        let Some(_ip) = decoded.ip else {
            continue;
        };
        let Some(udp) = decoded.udp else {
            continue;
        };

        if udp.dst_port != opt.port {
            continue;
        }
        decoded_udp = decoded_udp.wrapping_add(1);

        let payload_val = Val::List(decoded.payload.iter().copied().map(Val::U8).collect());
        let mut results = [Val::Bool(false)];

        match on_packet_received.call(&mut *store, &[payload_val], &mut results) {
            Ok(()) => {
                let _ = on_packet_received.post_return(&mut *store);
            }
            Err(e) => {
                guest_err = guest_err.wrapping_add(1);
                // 如果 guest call 本身失败，别继续用 results
                eprintln!(
                    "[echo-server/wasm] guest call failed: {e:#}. Falling back to native loop"
                );
                return run_echo_server_native_impl(opt);
            }
        }

        // component func typed: (list<u8>) -> (result<list<u8>, string>,)
        let maybe_payload: Option<Vec<u8>> = match &results[0] {
            Val::Result(r) => match r {
                Ok(Some(okv)) => {
                    // wit result ok payload: list<u8>
                    if let Val::List(vs) = okv.as_ref() {
                        let mut out = Vec::with_capacity(vs.len());
                        for v in vs {
                            match v {
                                Val::U8(b) => out.push(*b),
                                _ => {
                                    guest_err = guest_err.wrapping_add(1);
                                    eprintln!(
                                        "[echo-server/wasm] unexpected ok payload element type"
                                    );
                                    out.clear();
                                    break;
                                }
                            }
                        }
                        if out.is_empty() {
                            None
                        } else {
                            guest_ok = guest_ok.wrapping_add(1);
                            Some(out)
                        }
                    } else {
                        guest_err = guest_err.wrapping_add(1);
                        eprintln!("[echo-server/wasm] unexpected ok type (expected list<u8>)");
                        None
                    }
                }
                Ok(None) => {
                    guest_err = guest_err.wrapping_add(1);
                    eprintln!("[echo-server/wasm] ok result has no payload");
                    None
                }
                Err(_errv) => {
                    guest_err = guest_err.wrapping_add(1);
                    None
                }
            },
            _ => {
                guest_err = guest_err.wrapping_add(1);
                eprintln!("[echo-server/wasm] unexpected return slot type (expected result)");
                None
            }
        };

        let Some(new_payload) = maybe_payload else {
            if last_report.elapsed() >= report_iv {
                eprintln!(
                    "[echo-server/wasm] stats: rx={} udp={} guest_ok={} guest_err={} sent={}",
                    rx, decoded_udp, guest_ok, guest_err, sent
                );
                last_report = std::time::Instant::now();
            }
            continue;
        };

        let reply = {
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
                "[echo-server/wasm] stats: rx={} udp={} guest_ok={} guest_err={} sent={}",
                rx, decoded_udp, guest_ok, guest_err, sent
            );
            last_report = std::time::Instant::now();
        }
    }
}

// Echo Server 本地实现的简单包装（用于回退）
fn run_echo_server_local(opt: &Opt) -> Result<()> {
    eprintln!("[echo-server] using native implementation (WASM load failed)");
    run_echo_server_native_impl(opt)
}

// Echo Client 本地实现的简单包装（用于回退）
fn run_echo_client_local(opt: &Opt) -> Result<()> {
    eprintln!("[echo-client] using native implementation (WASM load failed)");
    run_echo_client_native(opt)
}

// 核心 Echo Server 实现逻辑
fn run_echo_server_native_impl(opt: &Opt) -> Result<()> {
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

    eprintln!(
        "[echo-server] iface={} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} port={} backend={:?}",
        nic.ifname(),
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
    let mut processed: u64 = 0;
    let mut sent: u64 = 0;

    let mut last_report = std::time::Instant::now();
    let report_iv = Duration::from_secs(1);

    loop {
        let _ = nic.poll_readable(Some(Duration::from_millis(100)))?;

        let n = match nic.recv_nonblocking(&mut buf)? {
            Some(n) => n,
            None => {
                if last_report.elapsed() >= report_iv {
                    eprintln!(
                        "[echo-server] rx={} udp={} processed={} sent={}",
                        rx, decoded_udp, processed, sent
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

        let Some(_ip) = decoded.ip else {
            continue;
        };
        let Some(udp) = decoded.udp else {
            continue;
        };

        if udp.dst_port != opt.port {
            continue;
        }

        decoded_udp = decoded_udp.wrapping_add(1);

        // 直接 Echo：构造回复包
        let reply = build_udp_reply(&decoded, MacAddr(iface_mac))?;

        if nic.send(&reply.bytes).is_ok() {
            sent = sent.wrapping_add(1);
            processed = processed.wrapping_add(1);
        }

        if last_report.elapsed() >= report_iv {
            eprintln!(
                "[echo-server] rx={} udp={} processed={} sent={}",
                rx, decoded_udp, processed, sent
            );
            last_report = std::time::Instant::now();
        }
    }
}

fn run_echo_server_native(
    _store: &mut Store<State>,
    _instance: &Instance,
    opt: &Opt,
) -> Result<()> {
    run_echo_server_native_impl(opt)
}

fn run_echo_client_wasm(store: &mut Store<State>, instance: &Instance, opt: &Opt) -> Result<()> {
    eprintln!(
        "ntx(echo-client-wasm) starting: iface={} server={}:{} count={} pps={} wasm_path={}",
        opt.iface,
        opt.server_ip,
        opt.server_port,
        opt.client_count,
        opt.pps,
        "plugins/scheduler/wac/echo-client.wasm"
    );

    let parent = match find_iface_parent(
        store,
        instance,
        &[
            "client",
            "scheduler:actions-executor/client@0.1.0",
            "scheduler:actions-executor/client",
        ],
    ) {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "[wasm] No exported interface instance 'client': {e}. Falling back to native"
            );
            return run_echo_client_native(opt);
        }
    };
    let Some(generate) = get_func_from_iface(store, instance, &parent, "generate") else {
        eprintln!("[wasm] Missing func export client/generate. Falling back to native");
        return run_echo_client_native(opt);
    };
    let Some(build_payload) = get_func_from_iface(store, instance, &parent, "build-payload") else {
        eprintln!("[wasm] Missing func export client/build-payload. Falling back to native");
        return run_echo_client_native(opt);
    };
    let Some(validate_reply) = get_func_from_iface(store, instance, &parent, "validate-reply")
    else {
        eprintln!("[wasm] Missing func export client/validate-reply. Falling back to native");
        return run_echo_client_native(opt);
    };

    eprintln!("[wasm] Echo client export resolved: client/generate");

    // 调用 guest 的 generate(count, pps)，让 WASM 决定要发送多少。
    let args = [Val::U32(opt.client_count), Val::U32(opt.pps)];
    let mut results = [Val::Bool(false)];
    match generate.call(&mut *store, &args, &mut results) {
        Ok(()) => {
            let _ = generate.post_return(&mut *store);
        }
        Err(e) => {
            eprintln!("[echo-client/wasm] guest call failed: {e:#}. Falling back to native");
            return run_echo_client_native(opt);
        }
    }

    let guest_count: Option<u32> = match &results[0] {
        Val::Result(r) => match r {
            Ok(Some(okv)) => match okv.as_ref() {
                Val::U32(n) => Some(*n),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    };

    if let Some(n) = guest_count {
        eprintln!("[echo-client/wasm] WASM generate() returned count={n}");
    } else {
        eprintln!(
            "[echo-client/wasm] Unexpected return from WASM generate(); falling back to native"
        );
        return run_echo_client_native(opt);
    }

    run_echo_client_native_with_wasm(store, &build_payload, &validate_reply, opt)
}

fn run_echo_client_native(opt: &Opt) -> Result<()> {
    use std::time::{Duration, Instant};

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

    // Source IP: 优先环境变量 NTX_CLIENT_IP，其次默认 10.0.0.2（与 ntx1 脚本一致）。
    let src_ip_str = std::env::var("NTX_CLIENT_IP").unwrap_or_else(|_| "10.0.0.2".to_string());
    let src_ip = parse_ipv4(&src_ip_str).context("invalid NTX_CLIENT_IP/ default src ip")?;
    let dst_ip = parse_ipv4(&opt.server_ip).context("invalid --server-ip")?;
    let dst_mac = MacAddr::BROADCAST; // 简化：广播以避免 ARP 依赖
    let src_port: u16 = 40000; // 固定源端口，方便调试

    eprintln!("[echo-client] NIC initialized: {}", nic.ifname());

    // 计算时间间隔（基于 pps）
    let packet_interval = if opt.pps > 0 {
        Duration::from_millis(1000 / opt.pps as u64)
    } else {
        Duration::from_millis(20)
    };

    let mut sent: u32 = 0;
    let mut received: u32 = 0;
    let mut matched: u32 = 0;
    let timeouts: u32 = 0;
    let mut errors: u32 = 0;

    let start_time = Instant::now();
    let mut last_send = start_time;
    let mut last_report = start_time;
    let report_iv = Duration::from_secs(1);

    let mut buf = vec![0u8; opt.snaplen];
    let mut ctx = PacketContext::with_capacity(opt.snaplen);

    eprintln!(
        "[echo-client] Generating {} requests at {} pps",
        opt.client_count, opt.pps
    );

    loop {
        let now = Instant::now();

        // 发送请求（按 pps 限制）
        if sent < opt.client_count && now.duration_since(last_send) >= packet_interval {
            // 构造简单的 UDP 回显请求
            // 格式: [seq: 4 bytes big-endian] [payload: "Echo request"]
            let seq = sent;
            let mut payload = Vec::new();
            payload.extend_from_slice(&seq.to_be_bytes());
            payload.extend_from_slice(b"Echo request data");

            let frame = build_udp_frame(
                MacAddr(iface_mac),
                dst_mac,
                src_ip,
                dst_ip,
                src_port,
                opt.server_port,
                &payload,
            )?;

            if nic.send(&frame).is_ok() {
                sent = sent.wrapping_add(1);
                eprintln!("[echo-client] Sent seq={} (udp)", seq);
            } else {
                errors = errors.wrapping_add(1);
            }

            last_send = now;
        }

        // 接收回复（非阻塞）
        let _ = nic.poll_readable(Some(Duration::from_millis(10)))?;

        if let Some(n) = nic.recv_nonblocking(&mut buf)? {
            received = received.wrapping_add(1);

            // 解析接收到的包
            ctx.set_frame(&buf[..n]);
            if let Ok(decoded) = ctx.decode() {
                if decoded.udp.is_some() {
                    // 简单验证：检查 payload 中的序列号
                    if decoded.payload.len() >= 4 {
                        let seq_bytes = [
                            decoded.payload[0],
                            decoded.payload[1],
                            decoded.payload[2],
                            decoded.payload[3],
                        ];
                        let seq = u32::from_be_bytes(seq_bytes);
                        if seq < sent {
                            matched = matched.wrapping_add(1);
                            eprintln!("[echo-client] Received matching seq={}", seq);
                        }
                    }
                }
            }
        }

        // 定期输出统计
        if now.duration_since(last_report) >= report_iv {
            eprintln!(
                "[echo-client] sent={} received={} matched={} timeouts={} errors={}",
                sent, received, matched, timeouts, errors
            );
            last_report = now;
        }

        // 结束条件：已发送所有请求且等待足够长的时间
        if sent >= opt.client_count && now.duration_since(last_send) > Duration::from_secs(2) {
            break;
        }

        // 总超时检查
        if now.duration_since(start_time) > Duration::from_secs(60) {
            break;
        }
    }

    let duration = start_time.elapsed();
    eprintln!(
        "[echo-client] Complete: sent={} received={} matched={} duration={:.2}s",
        sent,
        received,
        matched,
        duration.as_secs_f64()
    );

    println!(
        "[result] sent={} matched={} timeouts={} errors={}",
        sent, matched, timeouts, errors
    );

    Ok(())
}

fn run_echo_client_native_with_wasm(
    store: &mut Store<State>,
    build_payload: &Func,
    validate_reply: &Func,
    opt: &Opt,
) -> Result<()> {
    use std::time::{Duration, Instant};

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
    let src_ip = parse_ipv4(&src_ip_str).context("invalid NTX_CLIENT_IP/ default src ip")?;
    let dst_ip = parse_ipv4(&opt.server_ip).context("invalid --server-ip")?;
    let dst_mac = MacAddr::BROADCAST;
    let src_port: u16 = 40000;

    eprintln!("[echo-client] NIC initialized: {}", nic.ifname());

    let packet_interval = if opt.pps > 0 {
        Duration::from_millis(1000 / opt.pps as u64)
    } else {
        Duration::from_millis(20)
    };

    let mut sent: u32 = 0;
    let mut received: u32 = 0;
    let mut matched: u32 = 0;
    let timeouts: u32 = 0;
    let mut errors: u32 = 0;

    let start_time = Instant::now();
    let mut last_send = start_time;
    let mut last_report = start_time;
    let report_iv = Duration::from_secs(1);

    let mut buf = vec![0u8; opt.snaplen];
    let mut ctx = PacketContext::with_capacity(opt.snaplen);

    eprintln!(
        "[echo-client] Generating {} requests at {} pps (payload from WASM)",
        opt.client_count, opt.pps
    );

    loop {
        let now = Instant::now();

        if sent < opt.client_count && now.duration_since(last_send) >= packet_interval {
            // 请求 WASM 生成 payload
            let mut wasm_results = [Val::Bool(false)];
            if let Err(e) = build_payload.call(&mut *store, &[Val::U32(sent)], &mut wasm_results) {
                eprintln!(
                    "[echo-client/wasm] build-payload failed: {e:#}. Falling back to native payload"
                );
                // fallback to native payload builder
                let mut payload = Vec::new();
                payload.extend_from_slice(&sent.to_be_bytes());
                payload.extend_from_slice(b"Echo request data");
                if send_udp_frame(
                    &mut *nic,
                    MacAddr(iface_mac),
                    dst_mac,
                    src_ip,
                    dst_ip,
                    src_port,
                    opt.server_port,
                    &payload,
                ) {
                    sent = sent.wrapping_add(1);
                    eprintln!("[echo-client] Sent seq={} (fallback payload)", sent - 1);
                } else {
                    errors = errors.wrapping_add(1);
                }
                last_send = now;
                continue;
            }

            let maybe_payload = match &wasm_results[0] {
                Val::Result(r) => match r {
                    Ok(Some(okv)) => match okv.as_ref() {
                        Val::List(vs) => {
                            let mut out = Vec::with_capacity(vs.len());
                            for v in vs {
                                if let Val::U8(b) = v {
                                    out.push(*b);
                                }
                            }
                            Some(out)
                        }
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            };

            let Some(payload) = maybe_payload else {
                errors = errors.wrapping_add(1);
                last_send = now;
                continue;
            };

            if send_udp_frame(
                &mut *nic,
                MacAddr(iface_mac),
                dst_mac,
                src_ip,
                dst_ip,
                src_port,
                opt.server_port,
                &payload,
            ) {
                eprintln!("[echo-client] Sent seq={} (wasm payload)", sent);
                sent = sent.wrapping_add(1);
            } else {
                errors = errors.wrapping_add(1);
            }

            last_send = now;
        }

        let _ = nic.poll_readable(Some(Duration::from_millis(10)))?;

        if let Some(n) = nic.recv_nonblocking(&mut buf)? {
            received = received.wrapping_add(1);
            ctx.set_frame(&buf[..n]);
            if let Ok(decoded) = ctx.decode() {
                if let Some(udp) = decoded.udp {
                    if udp.dst_port == src_port {
                        let payload = decoded.payload;
                        if payload.len() >= 4 {
                            let seq_bytes = [payload[0], payload[1], payload[2], payload[3]];
                            let seq = u32::from_be_bytes(seq_bytes);

                            let mut wasm_results = [Val::Bool(false)];
                            let payload_val =
                                Val::List(payload.iter().copied().map(Val::U8).collect());
                            if validate_reply
                                .call(
                                    &mut *store,
                                    &[Val::U32(seq), payload_val],
                                    &mut wasm_results,
                                )
                                .is_ok()
                            {
                                let ok = matches!(
                                    &wasm_results[0],
                                    Val::Result(r) if matches!(r, Ok(Some(v)) if matches!(v.as_ref(), Val::Bool(true)))
                                );
                                if ok {
                                    matched = matched.wrapping_add(1);
                                    eprintln!(
                                        "[echo-client] Received matching seq={} (wasm validate)",
                                        seq
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        if now.duration_since(last_report) >= report_iv {
            eprintln!(
                "[echo-client] sent={} received={} matched={} timeouts={} errors={}",
                sent, received, matched, timeouts, errors
            );
            last_report = now;
        }

        if sent >= opt.client_count && now.duration_since(last_send) > Duration::from_secs(2) {
            break;
        }

        if now.duration_since(start_time) > Duration::from_secs(60) {
            break;
        }
    }

    let duration = start_time.elapsed();
    eprintln!(
        "[echo-client] Complete: sent={} received={} matched={} duration={:.2}s",
        sent,
        received,
        matched,
        duration.as_secs_f64()
    );

    println!(
        "[result] sent={} matched={} timeouts={} errors={}",
        sent, matched, timeouts, errors
    );

    Ok(())
}

fn send_udp_frame(
    nic: &mut dyn Nic,
    src_mac: MacAddr,
    dst_mac: MacAddr,
    src_ip: Ipv4Addr,
    dst_ip: Ipv4Addr,
    src_port: u16,
    dst_port: u16,
    payload: &[u8],
) -> bool {
    match build_udp_frame(
        src_mac, dst_mac, src_ip, dst_ip, src_port, dst_port, payload,
    ) {
        Ok(frame) => nic.send(&frame).is_ok(),
        Err(_) => false,
    }
}

fn parse_ipv4(s: &str) -> Result<Ipv4Addr> {
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
    Ok(Ipv4Addr(octets))
}

fn build_udp_frame(
    src_mac: MacAddr,
    dst_mac: MacAddr,
    src_ip: Ipv4Addr,
    dst_ip: Ipv4Addr,
    src_port: u16,
    dst_port: u16,
    payload: &[u8],
) -> Result<Vec<u8>> {
    let eth_len = EthernetHeader::LEN;
    let ip_len = Ipv4Header::MIN_LEN;
    let udp_len = UdpHeader::LEN;

    let mut bytes = vec![0u8; eth_len + ip_len + udp_len + payload.len()];

    let eth = EthernetHeader {
        dst: dst_mac,
        src: src_mac,
        ethertype: network::ETH_TYPE_IPV4,
    };
    eth.write(&mut bytes[..eth_len])?;

    let ip_hdr = Ipv4Header {
        src: src_ip,
        dst: dst_ip,
        protocol: 17,
        ttl: 64,
        identification: 0,
        flags_fragment: 0,
    };
    ip_hdr.write(
        &mut bytes[eth_len..eth_len + ip_len],
        udp_len + payload.len(),
        0,
    )?;

    let udp_hdr = UdpHeader { src_port, dst_port };
    udp_hdr.write(
        &mut bytes[eth_len + ip_len..eth_len + ip_len + udp_len + payload.len()],
        payload,
        src_ip,
        dst_ip,
    )?;

    Ok(bytes)
}
