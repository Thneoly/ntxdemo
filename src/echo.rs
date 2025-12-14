use std::time::Duration;

use anyhow::{Context, Result, bail};
use wasmtime::Store;
use wasmtime::component::{Func, Val};

use crate::component_utils::{find_iface_parent, get_func_from_iface};
use crate::network::stack::{PacketContext, build_udp_reply};
use crate::network::{self, EthernetHeader, Ipv4Addr, Ipv4Header, MacAddr, Nic, UdpHeader};
use crate::{Backend, Opt, State};

pub fn run_echo_server_wasm(
    store: &mut Store<State>,
    instance: &wasmtime::component::Instance,
    opt: &Opt,
) -> Result<()> {
    eprintln!(
        "ntx(echo-server-wasm) starting: iface={} port={} component={} wasm_path={}",
        opt.iface, opt.port, "echo-server.wasm", "plugins/scheduler/wac/echo-server.wasm"
    );

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
            return run_echo_server_native_impl(opt);
        }
    };
    let Some(on_packet_received) =
        get_func_from_iface(store, instance, &parent, "on-packet-received")
    else {
        eprintln!("[wasm] Missing func export server/on-packet-received. Falling back to native");
        return run_echo_server_native_impl(opt);
    };

    eprintln!("[wasm] Echo server export resolved: server/on-packet-received");

    run_echo_server_wasm_loop(store, &on_packet_received, opt)
}

pub fn run_echo_server_local(opt: &Opt) -> Result<()> {
    eprintln!("[echo-server] using native implementation (WASM load failed)");
    run_echo_server_native_impl(opt)
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
                eprintln!(
                    "[echo-server/wasm] guest call failed: {e:#}. Falling back to native loop"
                );
                return run_echo_server_native_impl(opt);
            }
        }

        let maybe_payload: Option<Vec<u8>> = match &results[0] {
            Val::Result(r) => match r {
                Ok(Some(okv)) => {
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

pub fn run_echo_server_native_impl(opt: &Opt) -> Result<()> {
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

pub fn run_echo_client_wasm(
    store: &mut Store<State>,
    instance: &wasmtime::component::Instance,
    opt: &Opt,
) -> Result<()> {
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

pub fn run_echo_client_local(opt: &Opt) -> Result<()> {
    eprintln!("[echo-client] using native implementation (WASM load failed)");
    run_echo_client_native(opt)
}

pub fn run_echo_client_native(opt: &Opt) -> Result<()> {
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
        "[echo-client] Generating {} requests at {} pps",
        opt.client_count, opt.pps
    );

    loop {
        let now = Instant::now();

        if sent < opt.client_count && now.duration_since(last_send) >= packet_interval {
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

        let _ = nic.poll_readable(Some(Duration::from_millis(10)))?;

        if let Some(n) = nic.recv_nonblocking(&mut buf)? {
            received = received.wrapping_add(1);

            ctx.set_frame(&buf[..n]);
            if let Ok(decoded) = ctx.decode() {
                if decoded.udp.is_some() {
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

pub fn run_echo_client_native_with_wasm(
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
            let mut wasm_results = [Val::Bool(false)];
            if let Err(e) = build_payload.call(&mut *store, &[Val::U32(sent)], &mut wasm_results) {
                eprintln!(
                    "[echo-client/wasm] build-payload failed: {e:#}. Falling back to native payload"
                );
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
