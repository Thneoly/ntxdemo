//! NTX registry client
//! - Coordinates: registry:plugins/{name}/{version}/… and by-digest immutable paths
//! - Backends: file:// and http(s):// (via feature `http`)
//! - Validate: JSON Schema + wasmtime component validate + digest

mod model;
mod paths;
mod validate;

pub use model::*;
pub use paths::to_oci_reference;
pub use paths::*;
pub use validate::{validate_component_bytes, validate_manifest_schema};

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("invalid uri: {0}")]
    InvalidUri(String),
    #[error("network: {0}")]
    Network(String),
    #[error("storage: {0}")]
    Storage(String),
    #[error("validate: {0}")]
    Validate(String),
}

/// A minimal client that can read manifests, release metadata and components
pub struct Client {
    base: Base,
    /// Optional local CAS directory; if set, fetched blobs are written to sha256/<hex>
    cas_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub enum Base {
    /// file backend root directory
    File(PathBuf),
    /// http backend base URL, e.g. https://registry.example.com/
    Http(Url),
}

impl Client {
    pub fn new(base: Base) -> Self {
        Self {
            base,
            cas_dir: None,
        }
    }
    pub fn with_cas_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cas_dir = Some(dir.into());
        self
    }

    /// Resolve a logical registry: URI to a concrete URL or local path
    pub fn resolve(&self, uri: &str) -> Result<Resolved> {
        paths::resolve_uri(&self.base, uri)
    }

    /// Fetch bytes from a resolved path/URL
    pub async fn fetch_bytes(&self, res: &Resolved) -> Result<Bytes> {
        match res {
            Resolved::File(p) => {
                let data = tokio::fs::read(p).await.context("read file")?;
                Ok(Bytes::from(data))
            }
            Resolved::Http(u) => {
                #[cfg(feature = "http")]
                {
                    let resp = reqwest::Client::new()
                        .get(u.clone())
                        .send()
                        .await
                        .context("http get")?;
                    if !resp.status().is_success() {
                        return Err(anyhow!(RegistryError::Network(format!(
                            "status {}",
                            resp.status()
                        ))));
                    }
                    Ok(resp.bytes().await.context("http bytes")?)
                }
                #[cfg(not(feature = "http"))]
                {
                    let _ = u;
                    Err(anyhow!(RegistryError::Network(
                        "http feature not enabled".into()
                    )))
                }
            }
        }
    }

    /// Read manifest.json for a given name/version
    pub async fn get_manifest(&self, name: &str, version: &str) -> Result<Manifest> {
        let uri = format!("registry:plugins/{name}/{version}/manifest.json");
        let res = self.resolve(&uri)?;
        let bytes = self.fetch_bytes(&res).await?;
        let m: Manifest = serde_json::from_slice(&bytes).context("parse manifest.json")?;
        Ok(m)
    }

    /// Read release.json for a given name/version
    pub async fn get_release(&self, name: &str, version: &str) -> Result<Release> {
        let uri = format!("registry:plugins/{name}/{version}/release.json");
        let res = self.resolve(&uri)?;
        let bytes = self.fetch_bytes(&res).await?;
        let r: Release = serde_json::from_slice(&bytes).context("parse release.json")?;
        Ok(r)
    }

    /// Fetch component by digest (preferred) and optionally persist to local CAS
    pub async fn get_component_by_digest(&self, sha256_hex: &str) -> Result<(Bytes, String)> {
        let uri = format!("registry:plugins/by-digest/sha256/{sha256_hex}/component.wasm");
        let res = self.resolve(&uri)?;
        let bytes = self.fetch_bytes(&res).await?;
        let digest = hex_sha256(&bytes);
        if digest != sha256_hex {
            return Err(anyhow!(RegistryError::Validate("digest mismatch".into())));
        }
        if let Some(dir) = &self.cas_dir {
            let path = dir.join("sha256").join(sha256_hex);
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await.ok();
            }
            tokio::fs::write(&path, &bytes).await.ok();
        }
        Ok((bytes, digest))
    }

    /// Fallback: fetch component from name/version path, verify digest, and store in CAS
    pub async fn get_component_by_nv(
        &self,
        name: &str,
        version: &str,
        expect_sha256: &str,
    ) -> Result<Bytes> {
        let uri = format!("registry:plugins/{name}/{version}/component.wasm");
        let res = self.resolve(&uri)?;
        let bytes = self.fetch_bytes(&res).await?;
        let got = hex_sha256(&bytes);
        if got != expect_sha256 {
            return Err(anyhow!(RegistryError::Validate(format!(
                "digest mismatch: expect {expect_sha256}, got {got}"
            ))));
        }
        if let Some(dir) = &self.cas_dir {
            let path = dir.join("sha256").join(expect_sha256);
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await.ok();
            }
            tokio::fs::write(&path, &bytes).await.ok();
        }
        Ok(bytes)
    }

    /// High-level: given name/version, try release.json → by-digest → nv
    pub async fn fetch_component_strict(&self, name: &str, version: &str) -> Result<Bytes> {
        let rel = self.get_release(name, version).await?;
        let digest = rel
            .component
            .digest_sha256
            .ok_or_else(|| anyhow!("release missing sha256"))?;
        if let Ok((bytes, _)) = self.get_component_by_digest(&digest).await {
            return Ok(bytes);
        }
        self.get_component_by_nv(name, version, &digest).await
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// Concrete resolved resource (file path or URL)
#[derive(Debug, Clone)]
pub enum Resolved {
    File(PathBuf),
    Http(Url),
}

impl Client {
    /// Publish a plugin version: writes manifest.json, component.wasm, and release.json.
    /// For file backend: creates directories and writes files. For http backend: POST multipart.
    pub async fn publish(
        &self,
        name: &str,
        version: &str,
        manifest: &serde_json::Value,
        component: &[u8],
        release: &serde_json::Value,
    ) -> Result<()> {
        match &self.base {
            Base::File(root) => {
                let vdir = root.join("plugins").join(name).join(version);
                tokio::fs::create_dir_all(&vdir)
                    .await
                    .context("mkdir version dir")?;
                let mpath = vdir.join("manifest.json");
                let cpath = vdir.join("component.wasm");
                let rpath = vdir.join("release.json");
                write_atomic(&mpath, serde_json::to_vec_pretty(manifest)?.as_slice()).await?;
                write_atomic(&cpath, component).await?;
                write_atomic(&rpath, serde_json::to_vec_pretty(release)?.as_slice()).await?;
                // create by-digest hardlink/copy if sha present
                if let Some(sha) = release
                    .pointer("/component/sha256")
                    .and_then(|v| v.as_str())
                {
                    let bd = root
                        .join("plugins")
                        .join("by-digest")
                        .join("sha256")
                        .join(sha)
                        .join("component.wasm");
                    if let Some(parent) = bd.parent() {
                        tokio::fs::create_dir_all(parent).await.ok();
                    }
                    // Try hard link else copy
                    if tokio::fs::hard_link(&cpath, &bd).await.is_err() {
                        tokio::fs::copy(&cpath, &bd).await.ok();
                    }
                }
                Ok(())
            }
            Base::Http(base) => {
                #[cfg(feature = "http")]
                {
                    let mut u = base.clone();
                    u.path_segments_mut()
                        .expect("base")
                        .extend(["plugins", name, version]);
                    let form = reqwest::multipart::Form::new()
                        .part(
                            "manifest.json",
                            reqwest::multipart::Part::bytes(serde_json::to_vec(manifest)?)
                                .file_name("manifest.json"),
                        )
                        .part(
                            "component.wasm",
                            reqwest::multipart::Part::bytes(component.to_vec())
                                .file_name("component.wasm"),
                        )
                        .part(
                            "release.json",
                            reqwest::multipart::Part::bytes(serde_json::to_vec(release)?)
                                .file_name("release.json"),
                        );
                    let resp = reqwest::Client::new()
                        .post(u)
                        .multipart(form)
                        .send()
                        .await?;
                    if !resp.status().is_success() {
                        return Err(anyhow!(RegistryError::Network(format!(
                            "publish status {}",
                            resp.status()
                        ))));
                    }
                    Ok(())
                }
                #[cfg(not(feature = "http"))]
                {
                    let _ = (&base, name, version, manifest, component, release);
                    Err(anyhow!(RegistryError::Network(
                        "http feature not enabled".into()
                    )))
                }
            }
        }
    }

    /// Mark a version as yanked (file: update plugin index; http: POST :yank)
    pub async fn yank(&self, name: &str, version: &str) -> Result<()> {
        match &self.base {
            Base::File(root) => {
                // Minimal: create a yanked marker file
                let p = root
                    .join("plugins")
                    .join(name)
                    .join(version)
                    .join(".yanked");
                write_atomic(&p, b"yanked").await?;
                Ok(())
            }
            Base::Http(base) => {
                #[cfg(feature = "http")]
                {
                    let mut u = base.clone();
                    u.path_segments_mut()
                        .expect("base")
                        .extend(["plugins", name, version, "yank"]);
                    let resp = reqwest::Client::new().post(u).send().await?;
                    if !resp.status().is_success() {
                        return Err(anyhow!(RegistryError::Network(format!(
                            "yank status {}",
                            resp.status()
                        ))));
                    }
                    Ok(())
                }
                #[cfg(not(feature = "http"))]
                {
                    let _ = (&base, name, version);
                    Err(anyhow!(RegistryError::Network(
                        "http feature not enabled".into()
                    )))
                }
            }
        }
    }
}

async fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    let tmp = path.with_extension(".tmp");
    tokio::fs::write(&tmp, data).await.context("write tmp")?;
    tokio::fs::rename(&tmp, path).await.context("rename")?;
    Ok(())
}
