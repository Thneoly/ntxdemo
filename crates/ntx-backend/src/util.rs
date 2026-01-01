use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tokio::fs;

pub fn ref_key(reference: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(reference.as_bytes());
    let digest = hasher.finalize();
    hex::encode(digest)
}

pub fn catalog_path(data_dir: &Path, reference: &str) -> PathBuf {
    data_dir
        .join("catalog")
        .join(format!("{}.json", ref_key(reference)))
}

pub fn wasm_path(data_dir: &Path, wasm_sha256_hex: &str) -> PathBuf {
    data_dir
        .join("wasm")
        .join(format!("{}.wasm", wasm_sha256_hex))
}

pub fn wasm_catalog_path(data_dir: &Path, wasm_sha256_hex: &str) -> PathBuf {
    data_dir
        .join("wasm")
        .join(format!("{}.catalog.json", wasm_sha256_hex))
}

pub fn workflow_path(data_dir: &Path, id: &str) -> PathBuf {
    data_dir.join("workflows").join(format!("{}.json", id))
}

pub fn looks_like_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn registry_from_ref(reference: &str) -> Option<&str> {
    reference.split('/').next().filter(|s| !s.is_empty())
}

pub fn ref_has_registry_and_repo(reference: &str) -> bool {
    if reference.contains("://") {
        return false;
    }
    let mut it = reference.split('/');
    let registry = it.next().unwrap_or("");
    let repo_rest = it.next().unwrap_or("");
    !registry.is_empty() && !repo_rest.is_empty()
}

pub async fn read_first_wasm_file(dir: &Path) -> anyhow::Result<PathBuf> {
    let mut rd = fs::read_dir(dir).await?;
    while let Some(entry) = rd.next_entry().await? {
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension() {
                if ext == "wasm" {
                    return Ok(path);
                }
            }
        }
    }
    anyhow::bail!("no .wasm file found in {}", dir.display())
}
