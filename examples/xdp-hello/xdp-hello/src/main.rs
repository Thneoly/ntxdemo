use anyhow::{Context, bail};
use aya::programs::{Xdp, XdpFlags};
use aya_log::EbpfLogger;
use clap::Parser;
use log::{info, warn};
use tokio::signal; // (1)

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum XdpMode {
    /// Let the program decide the attach mode (default flags).
    Auto,
    /// Force SKB (generic) mode.
    Skb,
    /// Force driver (native) mode.
    Driver,
}

#[derive(Debug, Parser)]
struct Opt {
    #[clap(short, long, default_value = "wlp10s0")]
    iface: String, // (2)

    /// XDP attach mode.
    ///
    /// Notes:
    /// - Many Wi-Fi devices don't support native (driver) XDP. Use `skb`.
    /// - `auto` uses the kernel default behavior for the chosen flags.
    #[clap(long, value_enum, default_value_t = XdpMode::Auto)]
    mode: XdpMode,
}

#[tokio::main] // (3)
async fn main() -> Result<(), anyhow::Error> {
    let opt = Opt::parse();

    env_logger::init();

    info!(
        "starting xdp-hello: iface={}, mode={:?}, pid={}",
        opt.iface,
        opt.mode,
        std::process::id()
    );

    // This will include your eBPF object file as raw bytes at compile-time and load it at
    // runtime. This approach is recommended for most real-world use cases. If you would
    // like to specify the eBPF program at runtime rather than at compile-time, you can
    // reach for `Ebpf::load_file` instead.
    // (4)
    // (5)
    info!("loading eBPF object (embedded via include_bytes)...");
    let mut bpf = aya::Ebpf::load(aya::include_bytes_aligned!(concat!(
        env!("OUT_DIR"),
        "/xdp-hello"
    )))?;

    info!("initializing eBPF logger (aya-log)...");
    match EbpfLogger::init(&mut bpf) {
        Err(e) => {
            // This can happen if you remove all log statements from your eBPF program.
            warn!("failed to initialize eBPF logger: {e}");
        }
        Ok(logger) => {
            let mut logger = tokio::io::unix::AsyncFd::with_interest(
                logger,
                tokio::io::Interest::READABLE,
            )?;
            tokio::task::spawn(async move {
                loop {
                    let mut guard = logger.readable_mut().await.unwrap();
                    guard.get_inner_mut().flush();
                    guard.clear_ready();
                }
            });
        }
    }
    // (6)
    info!("looking up program 'xdp_hello'...");
    let program: &mut Xdp = bpf
        .program_mut("xdp_hello")
        .context("program 'xdp_hello' not found in object")?
        .try_into()?;

    info!("loading XDP program into kernel...");
    program.load().context("failed to load XDP program")?; // (7)

    let flags = match opt.mode {
        XdpMode::Auto => XdpFlags::default(),
        XdpMode::Skb => XdpFlags::SKB_MODE,
        XdpMode::Driver => XdpFlags::DRV_MODE,
    };

    info!(
        "attaching XDP program to iface={} with flags={:?}...",
        opt.iface, flags
    );
    if let Err(e) = program.attach(&opt.iface, flags) {
        // Provide a high-signal hint for the most common failure mode.
        if matches!(opt.mode, XdpMode::Auto) {
            warn!(
                "attach failed with auto flags: {e}; trying SKB_MODE as a fallback (common fix for Wi-Fi interfaces)"
            );
            program
                .attach(&opt.iface, XdpFlags::SKB_MODE)
                .context("failed to attach even with SKB_MODE")?;
            info!("attached successfully with SKB_MODE");
        } else {
            bail!("failed to attach XDP program: {e}");
        }
    } else {
        info!("attached successfully");
    }

    info!(
        "ready: generate traffic on '{}' (e.g. ping) to see eBPF logs; use Ctrl-C to detach",
        opt.iface
    );

    info!("Waiting for Ctrl-C...");
    signal::ctrl_c().await?;
    info!("Exiting...");

    Ok(())
}
