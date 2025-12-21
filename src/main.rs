use anyhow::Result;
use ntx::{app_config::AppConfig, kernel, logger, scheduler};

fn main() -> Result<()> {
    logger::logger_init();
    tracing::info!(target: "ntx::main", "ntx starting");

    // Unified app config (kernel + scheduler/wasm). We fall back to defaults
    // if the file doesn't exist so the binary remains runnable.
    let cfg = AppConfig::load_yaml_file("config/app.yaml").unwrap_or_default();

    kernel::init(&cfg.kernel.config_path)?;
    tracing::info!(target: "ntx::main", "kernel initialized");

    scheduler::init_with_config(cfg.scheduler);
    tracing::info!(target: "ntx::main", "scheduler initialized; submitting wasm-run task");

    // Trigger a one-shot guest `run()` via the scheduler.
    // The engine is auto-loaded by `Scheduler::new()` if `NTX_COMPONENT` is set.
    scheduler::Scheduler::global().submit(scheduler::Task::wasm_call("wasm-run", "run"));
    tracing::info!(target: "ntx::main", "entering scheduler loop");

    // Keep the process alive: run the scheduler on the main thread.
    // (The thread-spawned scheduler is mainly for embedding; for a standalone binary
    // we want the main thread to block so tasks keep executing.)
    scheduler::Scheduler::global().run();
}
