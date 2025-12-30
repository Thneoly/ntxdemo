use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use chronetix_registry::{Base, Client};
use clap::{Parser, Subcommand};
use serde_json::Value;
use url::Url;

#[derive(Parser, Debug)]
#[command(name = "ntx-registry", version, about = "NTX Registry CLI")]
struct Cli {
    /// Backend base (file:///path or https://host/)
    #[arg(long, env = "NTX_REGISTRY_BASE")]
    base: String,

    /// Local CAS directory for fetched blobs
    #[arg(long)]
    cas_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Publish a plugin version
    Publish {
        #[arg(short = 'n', long)]
        name: String,
        #[arg(short = 'v', long)]
        version: String,
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        wasm: PathBuf,
        #[arg(long)]
        release: PathBuf,
    },
    /// Yank a version
    Yank {
        #[arg(short = 'n', long)]
        name: String,
        #[arg(short = 'v', long)]
        version: String,
    },
    /// Get files to an output dir
    Get {
        #[arg(short = 'n', long)]
        name: String,
        #[arg(short = 'v', long)]
        version: String,
        #[arg(short = 'o', long)]
        out: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let base = parse_base(&cli.base)?;
    let mut client = Client::new(base);
    if let Some(dir) = &cli.cas_dir {
        client = client.with_cas_dir(dir.clone());
    }

    match cli.command {
        Commands::Publish {
            name,
            version,
            manifest,
            wasm,
            release,
        } => {
            let manifest_json: Value = serde_json::from_slice(&tokio::fs::read(&manifest).await?)?;
            let release_json: Value = serde_json::from_slice(&tokio::fs::read(&release).await?)?;
            let wasm_bytes = tokio::fs::read(&wasm).await?;
            client
                .publish(&name, &version, &manifest_json, &wasm_bytes, &release_json)
                .await?;
            println!("published {name}:{version}");
        }
        Commands::Yank { name, version } => {
            client.yank(&name, &version).await?;
            println!("yanked {name}:{version}");
        }
        Commands::Get { name, version, out } => {
            tokio::fs::create_dir_all(&out).await.ok();
            let m = client.get_manifest(&name, &version).await?;
            let r = client.get_release(&name, &version).await?;
            let wasm = client.fetch_component_strict(&name, &version).await?;
            tokio::fs::write(
                out.join("manifest.json"),
                serde_json::to_vec_pretty(&serde_json::to_value(&m)?)?,
            )
            .await?;
            tokio::fs::write(
                out.join("release.json"),
                serde_json::to_vec_pretty(&serde_json::to_value(&r)?)?,
            )
            .await?;
            tokio::fs::write(out.join("component.wasm"), &wasm).await?;
            println!("wrote files to {}", out.display());
        }
    }
    Ok(())
}

fn parse_base(s: &str) -> Result<Base> {
    if s.starts_with("file://") {
        let u = Url::parse(s).context("parse file url")?;
        let p = u.to_file_path().map_err(|_| anyhow!("bad file url"))?;
        return Ok(Base::File(p));
    }
    if s.starts_with("http://") || s.starts_with("https://") {
        let u = Url::parse(s).context("parse url")?;
        return Ok(Base::Http(u));
    }
    Err(anyhow!("unsupported base: {s}"))
}
