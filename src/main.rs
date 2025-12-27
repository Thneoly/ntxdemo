use anyhow::Result;
use ntx::{app_config::AppConfig, kernel, logger, scheduler};
use std::path::PathBuf;
use std::thread;

fn main() -> Result<()> {
    logger::logger_init();
    tracing::info!(target: "ntx::main", "ntx starting");

    // Unified app config (kernel + scheduler/wasm). We fall back to defaults
    // if the file doesn't exist so the binary remains runnable.
    let cfg = AppConfig::load_yaml_file("config/app.yaml").unwrap_or_default();

    kernel::init(&cfg.kernel.config_path)?;
    tracing::info!(target: "ntx::main", "kernel initialized");

    // Start guest scheduler main loop on a dedicated thread.
    //
    // NOTE: the composed scheduler's `scheduler-component.run(config-dir)` is expected to
    // block (it owns the guest event loop). We keep host scheduling (NIC RX/TX, notify-rx)
    // on the main thread.
    let guest_config_dir: PathBuf = cfg
        .scheduler
        .wasm
        .config_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from("."));

    scheduler::init_with_config(cfg.scheduler);
    tracing::info!(target: "ntx::main", "scheduler initialized; submitting wasm-run task");
    thread::Builder::new()
        .name("ntx-guest-scheduler".into())
        .spawn(move || {
            let cfg_dir = guest_config_dir.display().to_string();
            tracing::info!(target: "ntx::main", config_dir = %cfg_dir, "starting guest scheduler run() thread");
            let mut mgr = ntx::wasm_engine::EngineManager::global()
                .lock()
                .expect("engine manager poisoned");
            if let Err(e) = mgr.run(cfg_dir) {
                tracing::error!(target: "ntx::main", error = %e, error_dbg = ?e, "guest scheduler run() exited with error");
            }
        })?;

    // Keep WasmCall tasks reserved for future non-blocking guest calls.
    // (The long-running guest `run()` is now executed on the dedicated thread above.)
    tracing::info!(target: "ntx::main", "entering scheduler loop");

    // Keep the process alive: run the scheduler on the main thread.
    // (The thread-spawned scheduler is mainly for embedding; for a standalone binary
    // we want the main thread to block so tasks keep executing.)
    scheduler::Scheduler::global().run();
}
