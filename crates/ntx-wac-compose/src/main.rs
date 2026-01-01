use std::{
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

use anyhow::{Context, bail};
use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "ntx-wac-compose",
    about = "Compose wasm components by invoking `wac compose`",
    disable_help_subcommand = true
)]
struct Cli {
    /// Path to `wac` executable.
    #[arg(long, default_value = "wac")]
    wac_bin: String,

    /// Working directory to run `wac` from.
    ///
    /// Defaults to the repository root (relative to this crate).
    #[arg(long)]
    cwd: Option<PathBuf>,

    /// Input .wac file.
    #[arg(long, default_value = "component/wac/scheduler-composition.wac")]
    wac_file: PathBuf,

    /// Dependencies directory passed to `wac compose --deps-dir`.
    #[arg(long, default_value = "component/wac/deps")]
    deps_dir: PathBuf,

    /// Output wasm component path.
    #[arg(
        short = 'o',
        long,
        default_value = "component/wac/scheduler-composed.wasm"
    )]
    out: PathBuf,

    /// Extra args forwarded to `wac compose` (after `--`).
    #[arg(last = true)]
    extra_args: Vec<String>,
}

fn default_repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn ensure_exists(cwd: &Path, p: &Path, what: &str) -> anyhow::Result<()> {
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    };
    if !abs.exists() {
        bail!(
            "{what} not found: {} (cwd={})",
            abs.display(),
            cwd.display()
        );
    }
    Ok(())
}

fn run_wac_compose(cli: &Cli, cwd: &Path) -> anyhow::Result<ExitStatus> {
    let mut cmd = Command::new(&cli.wac_bin);
    cmd.current_dir(cwd)
        .arg("compose")
        .arg(&cli.wac_file)
        .arg("--deps-dir")
        .arg(&cli.deps_dir)
        .arg("-o")
        .arg(&cli.out);

    if !cli.extra_args.is_empty() {
        cmd.args(&cli.extra_args);
    }

    cmd.status().with_context(|| {
        format!(
            "failed to spawn `{}` (is it installed and on PATH?)",
            cli.wac_bin
        )
    })
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let cwd = cli.cwd.clone().unwrap_or_else(default_repo_root);

    ensure_exists(&cwd, &cli.wac_file, "wac_file")?;
    ensure_exists(&cwd, &cli.deps_dir, "deps_dir")?;

    let status = run_wac_compose(&cli, &cwd)?;
    if !status.success() {
        bail!("wac compose failed with status: {status}");
    }

    // Best-effort: validate output existence after success.
    ensure_exists(&cwd, &cli.out, "output")?;

    Ok(())
}
