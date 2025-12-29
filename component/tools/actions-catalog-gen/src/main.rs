use anyhow::{Context, Result, anyhow};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);

    let component_path = args.next().ok_or_else(|| {
        anyhow!("usage: actions-catalog-gen <path-to-action-executor-component.wasm> [output.json]")
    })?;

    let output_path = args.next().map(PathBuf::from);

    let component_path = PathBuf::from(component_path);
    let catalog = actions_catalog_gen::load_catalog_from_component(&component_path).await?;

    let json = serde_json::to_string_pretty(&catalog).context("serialize catalog")?;

    match output_path {
        Some(p) => {
            std::fs::write(&p, json).with_context(|| format!("write {}", p.display()))?;
        }
        None => {
            println!("{json}");
        }
    }

    Ok(())
}
