use anyhow::Result;
use clap::Parser;
use ntx::{app_config::AppConfig, kernel, logger, scheduler};
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(name = "ntx")]
struct Cli {
    /// Unified app config file path (YAML).
    #[arg(long, default_value = "config/app.yaml")]
    config: PathBuf,
}

fn resolve_app_config_path(p: &Path) -> PathBuf {
    // Accept either a YAML file path, or a directory produced by ntx-backend
    // config bundles: <bundle>/config/app.yaml.
    if p.is_dir() {
        let candidate = p.join("config").join("app.yaml");
        if candidate.is_file() {
            return candidate;
        }
        let candidate = p.join("app.yaml");
        if candidate.is_file() {
            return candidate;
        }
    }
    p.to_path_buf()
}

/// Host entrypoint.
///
/// Final direction (see `component/doc/HOST.md`): Tokio-native orchestration and
/// a single owner that drives the guest scheduler `run()`.
#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    logger::logger_init();
    tracing::info!(target: "ntx::main", "ntx starting");

    // Unified app config (kernel + scheduler/wasm). We fall back to defaults
    // if the file doesn't exist so the binary remains runnable.
    let resolved_config = resolve_app_config_path(&cli.config);
    let cfg = match AppConfig::load_yaml_file(&resolved_config) {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::warn!(
                target: "ntx::main",
                config = %cli.config.display(),
                resolved_config = %resolved_config.display(),
                error = %e,
                "failed to load app config; falling back to defaults"
            );
            AppConfig::default()
        }
    };

    kernel::init(&cfg.kernel.config_path)?;
    tracing::info!(target: "ntx::main", "kernel initialized");

    // Start guest scheduler main loop on a dedicated thread.
    //
    // NOTE: the composed scheduler's `scheduler-component.run(config-dir)` is expected to
    // block (it owns the guest event loop). We keep host scheduling (NIC RX/TX and rx-ring
    // batch enqueue) on the main thread.
    let guest_config_dir: PathBuf = cfg
        .scheduler
        .wasm
        .config_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from("."));

    let scheduler_cfg = cfg.scheduler.clone();
    scheduler::init_with_config(scheduler_cfg.clone());
    tracing::info!(target: "ntx::main", "scheduler initialized; starting guest scheduler run() task");

    // End-state: WASM engine initialization is async and must be explicit.
    scheduler::init_wasm_engine_with_config(scheduler_cfg.wasm.clone()).await;

    // Engine owner: move the default engine out of EngineManager so we never hold
    // the EngineManager mutex across the long-running `run()` call.
    let cfg_dir = guest_config_dir.display().to_string();
    let engine = {
        let mut mgr = ntx::wasm_engine::EngineManager::global()
            .lock()
            .expect("engine manager poisoned");
        mgr.try_take_default_engine()
    };

    let engine = match engine {
        Some(engine) => engine,
        None => {
            return Err(anyhow::anyhow!(
                "wasm engine not initialized (no default engine registered). Check your app config has scheduler.wasm.component_path set and that it points to a valid .wasm component."
            ));
        }
    };

    // Spawn the engine owner and give scheduler the TX for RX batches.
    let engine_tx = ntx::engine_owner::spawn_engine_owner(engine, cfg_dir);
    scheduler::set_engine_tx(engine_tx);

    // Keep WasmCall tasks reserved for future non-blocking guest calls.
    // (The long-running guest `run()` is now executed on the dedicated thread above.)
    tracing::info!(target: "ntx::main", "entering scheduler loop");

    // Keep the process alive: run the scheduler on the main thread.
    // (The thread-spawned scheduler is mainly for embedding; for a standalone binary
    // we want the main thread to block so tasks keep executing.)
    // Keep the process alive.
    // NOTE: the current scheduler loop is blocking/condvar-based; until it's fully
    // Tokio-native, we run it on a blocking threadpool.
    tokio::task::spawn_blocking(|| scheduler::Scheduler::global().run())
        .await
        .expect("scheduler task join");

    // If we ever exit the scheduler loop (tests/embedders), request engine shutdown.
    scheduler::request_engine_shutdown();

    Ok(())
}
