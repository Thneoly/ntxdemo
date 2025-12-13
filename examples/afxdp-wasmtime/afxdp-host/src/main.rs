use anyhow::{Context, bail};
use aya::maps::{PerCpuArray, XskMap};
use aya::programs::{Xdp, XdpFlags};
use aya_log::EbpfLogger;
use clap::Parser;
use log::{info, warn};

use std::num::NonZeroU32;
use std::os::fd::AsRawFd;
use std::os::raw::c_int;
use xsk_rs as xsk;

use std::time::{Duration, Instant};

use wasmtime::component::{ComponentExportIndex, Func, Instance, Val, types::ComponentItem};

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum CaptureMode {
    /// Try AF_XDP (XSK/UMEM). Best performance when supported.
    Afxdp,
    /// Use an AF_PACKET raw socket to receive frames (copying path, but widely supported).
    Afpacket,
}

// Wasmtime component model host bindings
wit_bindgen::generate!({
    world: "guest",
    path: "wit",
    // Generate host bindings so this crate can instantiate the component and call its exports.
    // (The guest crate uses its own `wit_bindgen::generate!` to implement the exports.)
    generate_all,
});

// Generated WIT bindings live under `crate::exports`.
use crate::exports::afxdp::demo::packet;

// Wasmtime WASI preview2 (p2) expects the store context to implement `WasiView`.
// This small wrapper holds the WASI context and the component-model resource table.
#[derive(Default)]
struct WasiState {
    ctx: wasmtime_wasi::WasiCtx,
    table: wasmtime_wasi::ResourceTable,
}

impl wasmtime_wasi::WasiView for WasiState {
    fn ctx(&mut self) -> wasmtime_wasi::WasiCtxView<'_> {
        wasmtime_wasi::WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum XdpMode {
    Auto,
    Skb,
    Driver,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum XskBindMode {
    /// Use copy mode (most compatible, but not zero-copy on the wire).
    Copy,
    /// Request zero-copy mode (requires driver + kernel support; may fail).
    Zerocopy,
}

#[derive(Debug, Parser)]
struct Opt {
    #[clap(short, long, default_value = "lo")]
    iface: String,

    /// RX queue id to bind the AF_XDP socket to.
    #[clap(long, default_value_t = 0)]
    queue: u32,

    #[clap(long, value_enum, default_value_t = XdpMode::Auto)]
    mode: XdpMode,

    /// Path to the guest component.
    #[clap(
        long,
        default_value = "../afxdp-guest/target/wasm32-wasip1/release/afxdp_guest.wasm"
    )]
    guest: String,

    /// Number of packets to process before exiting (0 = run forever).
    #[clap(long, default_value_t = 0)]
    limit: u64,

    /// Process/call the guest only for every Nth packet.
    ///
    /// - `1` (default): process every packet
    /// - `10`: process ~10% of packets (1 out of 10)
    ///
    /// This is mainly to reduce log spam in AF_PACKET mode on busy interfaces.
    #[clap(long, default_value_t = 1)]
    sample: u64,

    /// AF_XDP bind mode: copy (compatible) or zerocopy (requires NIC support).
    #[clap(long, value_enum, default_value_t = XskBindMode::Copy)]
    xsk_bind: XskBindMode,

    /// When rx.consume() returns 0, poll() the AF_XDP socket for readability.
    /// This helps on kernels/drivers where the RX ring needs explicit wakeups/notifications.
    #[clap(long, default_value_t = true)]
    poll_rx: bool,

    /// poll() timeout in milliseconds when --poll-rx is enabled.
    #[clap(long, default_value_t = 50)]
    poll_timeout_ms: i32,

    /// Fallback demo mode: if AF_XDP RX stays at 0 on this NIC/driver,
    /// still invoke the Wasm guest once per second when XDP hit counters increase.
    ///
    /// This lets you demonstrate the end-to-end control path (XDP -> userspace -> Wasm)
    /// even on hardware that can't deliver packets to AF_XDP.
    #[clap(long, default_value_t = true)]
    fallback_guest_on_xdp_stats: bool,

    /// How to capture packet bytes in userspace.
    ///
    /// - `afxdp`: AF_XDP/XSK (may be unsupported on some NICs/drivers)
    /// - `afpacket`: AF_PACKET raw socket (copy, but works on most NICs; recommended for SKB demo)
    #[clap(long, value_enum, default_value_t = CaptureMode::Afxdp)]
    capture: CaptureMode,
}

fn poll_readable(fd: c_int, timeout_ms: i32) -> std::io::Result<bool> {
    // SAFETY: libc::poll expects a valid pointer to a pollfd.
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let rc = unsafe { libc::poll(&mut pfd as *mut libc::pollfd, 1, timeout_ms) };
    if rc < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(rc > 0 && (pfd.revents & libc::POLLIN) != 0)
}

// Minimal packet dissector for observability (host side)
fn hexdump_prefix(data: &[u8], max: usize) -> String {
    let n = data.len().min(max);
    data[..n]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn open_afpacket_socket(iface: &str) -> anyhow::Result<c_int> {
    // Create RAW packet socket for all Ethernet protocols.
    let fd = unsafe {
        libc::socket(
            libc::AF_PACKET,
            libc::SOCK_RAW,
            (libc::ETH_P_ALL as u16).to_be() as i32,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error()).context("socket(AF_PACKET) failed");
    }

    // Bind to interface index.
    let c_ifname = std::ffi::CString::new(iface)?;
    let ifindex = unsafe { libc::if_nametoindex(c_ifname.as_ptr()) };
    if ifindex == 0 {
        let e = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(e).context("if_nametoindex failed");
    }

    let mut sll: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
    sll.sll_family = libc::AF_PACKET as u16;
    sll.sll_protocol = (libc::ETH_P_ALL as u16).to_be();
    sll.sll_ifindex = ifindex as i32;

    let rc = unsafe {
        libc::bind(
            fd,
            &sll as *const libc::sockaddr_ll as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_ll>() as u32,
        )
    };
    if rc != 0 {
        let e = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(e).context("bind(AF_PACKET) failed");
    }

    Ok(fd)
}

fn find_iface_parent(
    store: &mut wasmtime::Store<WasiState>,
    inst: &Instance,
    candidates: &[&str],
) -> anyhow::Result<ComponentExportIndex> {
    for name in candidates {
        if let Some((item, idx)) = inst.get_export(&mut *store, None, name) {
            if matches!(item, ComponentItem::ComponentInstance(_)) {
                return Ok(idx);
            }
        }
    }
    bail!("could not find exported interface instance: candidates={candidates:?}");
}

fn find_top_level_func(
    store: &mut wasmtime::Store<WasiState>,
    inst: &Instance,
    candidates: &[&str],
) -> anyhow::Result<Func> {
    for name in candidates {
        if let Some((item, idx)) = inst.get_export(&mut *store, None, name) {
            if matches!(item, ComponentItem::ComponentFunc(_)) {
                if let Some(f) = inst.get_func(&mut *store, idx) {
                    return Ok(f);
                }
            }
        }
    }
    bail!("could not find exported function: candidates={candidates:?}");
}

fn get_func_from_iface(
    store: &mut wasmtime::Store<WasiState>,
    inst: &Instance,
    parent: &ComponentExportIndex,
    func_name: &str,
) -> Option<Func> {
    let (_item, func_idx) = inst.get_export(&mut *store, Some(parent), func_name)?;
    inst.get_func(&mut *store, func_idx)
}

fn main() -> Result<(), anyhow::Error> {
    env_logger::init();
    let opt = Opt::parse();

    info!(
        "starting afxdp-host: iface={}, queue={}, pid={}",
        opt.iface,
        opt.queue,
        std::process::id()
    );

    info!("xsk bind mode: {:?}", opt.xsk_bind);

    // --- Load and attach XDP (redirect to XSK map) ---
    info!("loading eBPF object (embedded via include_bytes)...");
    let mut bpf = aya::Ebpf::load(aya::include_bytes_aligned!(concat!(
        env!("OUT_DIR"),
        "/afxdp-ebpf.o"
    )))
    .context("failed to load embedded eBPF")?;

    match EbpfLogger::init(&mut bpf) {
        Err(e) => warn!("eBPF logger init skipped: {e}"),
        Ok(logger) => {
            // best-effort; if unused it’s fine
            let _ = logger;
        }
    }

    let program: &mut Xdp = bpf
        .program_mut("xdp_redirect_to_xsk")
        .context("program 'xdp_redirect_to_xsk' not found")?
        .try_into()?;

    program.load().context("failed to load XDP program")?;

    let flags = match opt.mode {
        XdpMode::Auto => XdpFlags::default(),
        XdpMode::Skb => XdpFlags::SKB_MODE,
        XdpMode::Driver => XdpFlags::DRV_MODE,
    };

    info!(
        "attaching XDP program to iface={} flags={flags:?}...",
        opt.iface
    );
    if let Err(e) = program.attach(&opt.iface, flags) {
        // Aya returns a `ProgramError` here (not `anyhow::Error`), so we can't use
        // `anyhow::Error::chain()`. The Debug formatting usually includes nested
        // causes and is the best we can do without matching all variants.
        warn!("XDP attach error (display): {e}");
        warn!("XDP attach error (debug): {e:?}");
        if matches!(opt.mode, XdpMode::Auto) {
            warn!("attach failed in auto mode: {e}; retrying with SKB_MODE");
            program
                .attach(&opt.iface, XdpFlags::SKB_MODE)
                .context("attach failed even with SKB_MODE")?;
        } else {
            bail!("failed to attach XDP program: {e}");
        }
    }

    // --- Capture setup ---
    // We always attach XDP and keep XDP_STATS for observability.
    // Packet bytes are captured either via AF_XDP (preferred) or via AF_PACKET (copy, but widely supported).

    // Optional AF_XDP state (only initialized when capture==Afxdp)
    const FRAME_COUNT: u32 = 4096;
    let mut afxdp_umem: Option<xsk::Umem> = None;
    let mut afxdp_rx: Option<xsk::RxQueue> = None;
    let mut afxdp_fill_q: Option<xsk::FillQueue> = None;
    let mut afxdp_free_frames: Vec<xsk::umem::frame::FrameDesc> = Vec::new();
    let mut afxdp_rx_fd: Option<c_int> = None;

    // Optional AF_PACKET fd (only initialized when capture==Afpacket)
    let mut afpacket_fd: Option<c_int> = None;

    match opt.capture {
        CaptureMode::Afxdp => {
            // --- Create UMEM + AF_XDP socket (xsk-rs) ---
            // xsk-rs is a libxdp/libbpf-based AF_XDP wrapper and exposes the socket fd via `AsRawFd`.
            info!("creating UMEM...");
            let umem_cfg = xsk::config::UmemConfigBuilder::new()
                // Keep defaults but set frame size explicitly for clarity.
                .frame_size(xsk::config::FrameSize::new(
                    xsk::config::XDP_UMEM_MIN_CHUNK_SIZE,
                )?)
                .build()?;
            let (umem, mut frames) =
                xsk::Umem::new(umem_cfg, NonZeroU32::new(FRAME_COUNT).unwrap(), false)
                    .context("failed to create umem")?;

            info!("creating XSK socket...");
            let bind_flags = match opt.xsk_bind {
                XskBindMode::Copy => xsk::config::BindFlags::XDP_COPY,
                XskBindMode::Zerocopy => xsk::config::BindFlags::XDP_ZEROCOPY,
            };
            let sock_cfg = xsk::config::SocketConfigBuilder::new()
                // Aya is already attaching the XDP program; prevent libxdp from trying to
                // load/replace programs via its dispatcher.
                .libxdp_flags(xsk::config::LibxdpFlags::XSK_LIBXDP_FLAGS_INHIBIT_PROG_LOAD)
                .bind_flags(bind_flags)
                .build();
            let iface: xsk::config::Interface = opt.iface.parse()?;
            let (_tx, rx, mut fq_and_cq) = unsafe {
                xsk::Socket::new(sock_cfg, &umem, &iface, opt.queue)
                    .context("failed to create xsk socket")?
            };

            // When the UMEM isn't shared, socket creation returns fresh Fill+Comp queues.
            // We must seed the FillQueue with free frames; otherwise no RX buffers are available.
            let (mut fill_q, _comp_q) = fq_and_cq.take().ok_or_else(|| {
                anyhow::anyhow!("unexpected: socket creation returned no Fill/Comp queues")
            })?;

            // Submit as many free frames as possible to the kernel.
            let mut push = 0usize;
            while push < frames.len() {
                let submitted = unsafe { fill_q.produce(&frames[push..]) };
                if submitted == 0 {
                    break;
                }
                push += submitted;
            }
            // Keep any frames that weren't accepted immediately.
            let free_frames = if push < frames.len() {
                frames.split_off(push)
            } else {
                Vec::new()
            };

            // Populate XSK map: key = queue.
            info!("updating XSK map (XSKS[queue] = fd)...");
            let mut xsks: XskMap<_> = bpf
                .map_mut("XSKS")
                .ok_or_else(|| anyhow::anyhow!("map 'XSKS' not found"))?
                .try_into()?;
            xsks.set(opt.queue, rx.fd().as_raw_fd(), 0)
                .context("failed to set XSK map entry")?;

            afxdp_rx_fd = Some(rx.fd().as_raw_fd());
            afxdp_umem = Some(umem);
            afxdp_rx = Some(rx);
            afxdp_fill_q = Some(fill_q);
            afxdp_free_frames = free_frames;
        }
        CaptureMode::Afpacket => {
            info!("capture mode: afpacket (raw socket)");
            let fd = open_afpacket_socket(&opt.iface)?;
            afpacket_fd = Some(fd);
            // We won't be able to receive AF_XDP frames, but XDP_STATS still verifies XDP traffic.
        }
    }

    // Optional debug stats from the XDP program.
    // This is especially useful when AF_XDP RX stays at 0: it tells us whether
    // redirect() is happening or if we're falling back to XDP_PASS.
    let mut xdp_stats: Option<PerCpuArray<_, u64>> = match bpf.map_mut("XDP_STATS") {
        Some(m) => match PerCpuArray::try_from(m) {
            Ok(arr) => Some(arr),
            Err(e) => {
                warn!("found map XDP_STATS but failed to open it as PerCpuArray<u64>: {e}");
                None
            }
        },
        None => {
            warn!("map XDP_STATS not found (ok if using older eBPF object)");
            None
        }
    };

    // --- Setup Wasmtime guest ---
    info!("loading guest component: {}", opt.guest);
    let engine = wasmtime::Engine::default();
    let component = wasmtime::component::Component::from_file(&engine, &opt.guest)
        .context("failed to load guest component")?;

    let mut linker: wasmtime::component::Linker<WasiState> =
        wasmtime::component::Linker::new(&engine);
    // Wasmtime 39 provides WASI preview2 under `p2`.
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).context("failed to add WASI")?;

    // Wasmtime setup.
    // We generate *host-side* bindings for this world and will instantiate the component,
    // then call its exports from the RX loop (and from fallback paths).
    // Instantiate the component and discover exported functions.
    let mut store = wasmtime::Store::new(
        &engine,
        WasiState {
            ctx: wasmtime_wasi::WasiCtxBuilder::new()
                .inherit_stdout()
                .inherit_stderr()
                .build(),
            table: wasmtime_wasi::ResourceTable::new(),
        },
    );

    let instance = linker
        .instantiate(&mut store, &component)
        .context("failed to instantiate guest component")?;

    // Top-level export `run`
    let guest_run = find_top_level_func(&mut store, &instance, &["run"])
        .context("failed to find guest export `run`")?;

    // Exported interface instance `packet` (exact export name depends on tooling; try a few).
    let packet_parent = find_iface_parent(
        &mut store,
        &instance,
        &["packet", "afxdp:demo/packet", "afxdp:demo.packet"],
    )
    .context("failed to find exported interface instance for packet")?;

    let guest_on_packet =
        get_func_from_iface(&mut store, &instance, &packet_parent, "on-packet")
            .ok_or_else(|| anyhow::anyhow!("failed to find exported function packet.on-packet"))?;

    info!(
        "ready: generate traffic on {} (e.g. ping 127.0.0.1). Ctrl-C to stop.",
        opt.iface
    );

    if opt.poll_rx {
        info!(
            "rx wakeup: poll() enabled, timeout={}ms",
            opt.poll_timeout_ms
        );
    } else {
        info!("rx wakeup: poll() disabled");
    }

    // --- RX loop ---
    let mut seen: u64 = 0;
    let mut last_report = Instant::now();
    let mut last_seen = 0u64;
    let mut printed_first = false;

    // XDP stats deltas
    let mut last_xdp_hit = 0u64;
    let mut last_xdp_redir_ok = 0u64;
    let mut last_xdp_redir_err = 0u64;
    let mut last_xdp_pass = 0u64;
    let mut last_xdp_action_redirect = 0u64;
    let mut last_xdp_action_drop = 0u64;
    let mut last_xdp_action_aborted = 0u64;
    let mut last_xdp_action_tx = 0u64;
    let mut guest_last_hit_total = 0u64;
    let rx_fd = afxdp_rx_fd.unwrap_or(-1);
    loop {
        match opt.capture {
            CaptureMode::Afxdp => {
                // SAFETY: We only enter this branch if AF_XDP was set up.
                let rx = afxdp_rx.as_mut().expect("afxdp rx missing");
                let umem = afxdp_umem.as_ref().expect("afxdp umem missing");
                let fill_q = afxdp_fill_q.as_mut().expect("afxdp fill_q missing");

                let mut descs = [xsk::umem::frame::FrameDesc::default(); 64];
                let mut n = unsafe { rx.consume(&mut descs) };
                if n == 0 {
                    if opt.poll_rx {
                        match poll_readable(rx_fd, opt.poll_timeout_ms) {
                            Ok(true) => {
                                n = unsafe { rx.consume(&mut descs) };
                            }
                            Ok(false) => {}
                            Err(e) => {
                                warn!("poll(rx_fd={}) failed: {e}", rx_fd);
                            }
                        }
                    } else {
                        std::thread::sleep(Duration::from_millis(0));
                    }
                }

                if n > 0 && !printed_first {
                    info!("rx: first batch received: {} frames", n);
                }

                for desc in descs.into_iter().take(n) {
                    let bytes = unsafe { umem.data(&desc) };
                    let bytes = bytes.contents();

                    // Downsample guest calls to avoid excessive logging.
                    if opt.sample <= 1 || (seen % opt.sample) == 0 {
                        let meta = packet::PacketMeta {
                            frame_len: bytes.len() as u32,
                            rx_queue: opt.queue,
                        };
                        let data = bytes.to_vec();

                        let mut results = [];
                        guest_on_packet
                            .call(
                                &mut store,
                                &[
                                    Val::Record(vec![
                                        ("frame-len".to_string(), Val::U32(meta.frame_len)),
                                        ("rx-queue".to_string(), Val::U32(meta.rx_queue)),
                                    ]),
                                    Val::List(data.into_iter().map(Val::U8).collect()),
                                ],
                                &mut results,
                            )
                            .context("failed to call guest packet.on-packet")?;
                        let _ = guest_on_packet.post_return(&mut store);
                    }

                    if !printed_first {
                        printed_first = true;
                        info!(
                            "first packet: len={} prefix={}",
                            bytes.len(),
                            hexdump_prefix(bytes, 32)
                        );
                    }

                    seen += 1;
                    if opt.limit != 0 && seen >= opt.limit {
                        info!("processed {} packets; exiting due to --limit", seen);
                        return Ok(());
                    }

                    if unsafe { fill_q.produce_one(&desc) } == 0 {
                        afxdp_free_frames.push(desc);
                    }
                }
            }
            CaptureMode::Afpacket => {
                let fd = afpacket_fd.expect("afpacket fd missing");
                let mut buf = [0u8; 2048];
                let n =
                    unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) };
                if n < 0 {
                    let e = std::io::Error::last_os_error();
                    // EINTR is fine.
                    if e.kind() != std::io::ErrorKind::Interrupted {
                        warn!("afpacket recv failed: {e}");
                        std::thread::sleep(Duration::from_millis(10));
                    }
                } else if n == 0 {
                    // unlikely for raw sockets, but keep loop responsive
                    std::thread::sleep(Duration::from_millis(0));
                } else {
                    let bytes = &buf[..(n as usize)];

                    // Downsample guest calls to avoid excessive logging.
                    if opt.sample <= 1 || (seen % opt.sample) == 0 {
                        let meta = packet::PacketMeta {
                            frame_len: bytes.len() as u32,
                            rx_queue: opt.queue,
                        };

                        let data = bytes.to_vec();
                        let mut results = [];
                        guest_on_packet
                            .call(
                                &mut store,
                                &[
                                    Val::Record(vec![
                                        ("frame-len".to_string(), Val::U32(meta.frame_len)),
                                        ("rx-queue".to_string(), Val::U32(meta.rx_queue)),
                                    ]),
                                    Val::List(data.into_iter().map(Val::U8).collect()),
                                ],
                                &mut results,
                            )
                            .context("failed to call guest packet.on-packet")?;
                        let _ = guest_on_packet.post_return(&mut store);
                    }

                    if !printed_first {
                        printed_first = true;
                        info!(
                            "first packet (afpacket): len={} prefix={}",
                            bytes.len(),
                            hexdump_prefix(bytes, 32)
                        );
                    }

                    seen += 1;
                    if opt.limit != 0 && seen >= opt.limit {
                        info!("processed {} packets; exiting due to --limit", seen);
                        return Ok(());
                    }
                }
            }
        }

        // Lightweight metrics reporting.
        let now = Instant::now();
        if now.duration_since(last_report) >= Duration::from_secs(1) {
            let delta = seen - last_seen;
            let secs = now.duration_since(last_report).as_secs_f64();
            let pps = (delta as f64) / secs.max(1e-9);

            let mut xdp_stats_line = String::new();
            let mut xdp_hit_total_for_guest: Option<u64> = None;
            if let Some(arr) = xdp_stats.as_mut() {
                // Each index returns a Vec<u64> of per-CPU values.
                let sum = |idx: u32| -> anyhow::Result<u64> {
                    let v = arr.get(&idx, 0)?;
                    Ok(v.iter().copied().sum())
                };

                // Base counters (must exist)
                match (sum(0), sum(1), sum(2), sum(3)) {
                    (Ok(hit), Ok(ro), Ok(re), Ok(pass)) => {
                        xdp_hit_total_for_guest = Some(hit);
                        let dh = hit - last_xdp_hit;
                        let dro = ro - last_xdp_redir_ok;
                        let dre = re - last_xdp_redir_err;
                        let dp = pass - last_xdp_pass;
                        last_xdp_hit = hit;
                        last_xdp_redir_ok = ro;
                        last_xdp_redir_err = re;
                        last_xdp_pass = pass;

                        // Optional action counters (may not exist if older map layout)
                        let mut action_suffix = String::new();
                        if let (Ok(r), Ok(d), Ok(a), Ok(tx)) = (sum(4), sum(5), sum(6), sum(7)) {
                            let dr = r - last_xdp_action_redirect;
                            let dd = d - last_xdp_action_drop;
                            let da = a - last_xdp_action_aborted;
                            let dtx = tx - last_xdp_action_tx;
                            last_xdp_action_redirect = r;
                            last_xdp_action_drop = d;
                            last_xdp_action_aborted = a;
                            last_xdp_action_tx = tx;
                            action_suffix = format!(
                                " | action: redirect+{} drop+{} aborted+{} tx+{}",
                                dr, dd, da, dtx
                            );
                        }

                        xdp_stats_line = format!(
                            " | xdp: hit+{} redir_ok+{} redir_err+{} pass+{}{}",
                            dh, dro, dre, dp, action_suffix
                        );
                    }
                    (Err(e), _, _, _) => {
                        warn!("failed to read XDP_STATS: {e}");
                        xdp_stats = None;
                    }
                    _ => {}
                }
            }

            // Fallback demo path: no AF_XDP RX, but XDP hits increase.
            // Invoke guest `run()` so the user can see a Wasm call happening.
            if opt.fallback_guest_on_xdp_stats {
                if let Some(hit_total) = xdp_hit_total_for_guest {
                    if hit_total > guest_last_hit_total {
                        guest_last_hit_total = hit_total;
                        // Call guest `run()` once per second when we detect XDP traffic.
                        // This keeps the SKB/generic demo end-to-end even when AF_XDP RX is 0.
                        let mut results = [];
                        if let Err(e) = guest_run.call(&mut store, &[], &mut results) {
                            warn!("fallback: guest.run() failed: {e}");
                        } else {
                            let _ = guest_run.post_return(&mut store);
                            info!(
                                "fallback: invoked guest.run() due to XDP traffic (hit_total={})",
                                hit_total
                            );
                        }
                    }
                }
            }

            info!(
                "rx: total={} pps={:.0} free_backlog={}{}",
                seen,
                pps,
                if matches!(opt.capture, CaptureMode::Afxdp) {
                    afxdp_free_frames.len()
                } else {
                    0
                },
                xdp_stats_line
            );
            last_report = now;
            last_seen = seen;
        }

        // If we failed to return some AF_XDP frames earlier, try again now.
        if matches!(opt.capture, CaptureMode::Afxdp) && !afxdp_free_frames.is_empty() {
            if let Some(fill_q) = afxdp_fill_q.as_mut() {
                let mut i = 0usize;
                while i < afxdp_free_frames.len() {
                    let submitted = unsafe { fill_q.produce(&afxdp_free_frames[i..]) };
                    if submitted == 0 {
                        break;
                    }
                    i += submitted;
                }
                if i > 0 {
                    afxdp_free_frames.drain(0..i);
                }
            }
        }
    }
}
