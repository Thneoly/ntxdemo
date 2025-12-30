use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};
use url::Url;

use crate::{Base, Resolved};

/// Resolve registry:* URIs against a base
pub fn resolve_uri(base: &Base, uri: &str) -> Result<Resolved> {
    if let Some(rest) = uri.strip_prefix("registry:") {
        match base {
            Base::File(root) => {
                let p = normalize_file_path(root, rest);
                return Ok(Resolved::File(p));
            }
            Base::Http(url) => {
                let u = normalize_http_url(url, rest)?;
                return Ok(Resolved::Http(u));
            }
        }
    }
    // absolute file url
    if let Ok(url) = Url::parse(uri) {
        match (url.scheme(), base) {
            ("file", _) => {
                let p = url.to_file_path().map_err(|_| anyhow!("bad file url"))?;
                return Ok(Resolved::File(p));
            }
            ("http" | "https", _) => return Ok(Resolved::Http(url)),
            _ => {}
        }
    }
    Err(anyhow!("unsupported uri: {}", uri))
}

fn normalize_file_path(root: &Path, rest: &str) -> PathBuf {
    let clean = rest.trim_start_matches('/');
    root.join(clean)
}

fn normalize_http_url(base: &Url, rest: &str) -> Result<Url> {
    let mut u = base.clone();
    let clean = rest.trim_start_matches('/');
    u.path_segments_mut()
        .map_err(|_| anyhow!("base url cannot be a base"))?
        .extend(clean.split('/'));
    Ok(u)
}

/// Convert a registry: URI to an OCI reference string oci://HOST/chronetix/plugins/{name}:{version}
/// or oci://HOST/chronetix/plugins/{name}@sha256:{hex} for by-digest. Only understands
/// the plugin paths defined by the spec; returns error for unsupported shapes.
pub fn to_oci_reference(host: &str, registry_uri: &str) -> Result<String> {
    let rest = registry_uri
        .strip_prefix("registry:")
        .ok_or_else(|| anyhow!("not a registry: uri: {}", registry_uri))?;

    // supported shapes:
    // plugins/{name}/{version}/manifest.json | component.wasm | release.json
    // plugins/by-digest/sha256/{hex}/component.wasm
    let parts: Vec<&str> = rest.trim_start_matches('/').split('/').collect();
    if parts.is_empty() || parts[0] != "plugins" {
        return Err(anyhow!("unsupported registry uri: {}", registry_uri));
    }
    if parts.len() >= 4 && parts[1] == "by-digest" && parts[2] == "sha256" {
        let hex = parts[3];
        // optional trailing component path is ignored; digest addresses the artifact
        let repo = format!("chronetix/plugins/{}", "unknown");
        // we cannot infer name from by-digest path; require callers to provide explicit repo later
        // For now, map to chronetix/plugins/_ by digest only
        let reference = format!("oci://{host}/{repo}@sha256:{hex}");
        return Ok(reference);
    }
    if parts.len() >= 4 {
        let name = parts[1];
        let version = parts[2];
        let repo = format!("chronetix/plugins/{name}");
        let reference = format!("oci://{host}/{repo}:{version}");
        return Ok(reference);
    }
    Err(anyhow!("unsupported registry uri: {}", registry_uri))
}
