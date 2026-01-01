use std::path::Path;

use anyhow::Context;
use tokio::process::Command;

use crate::{state::AppState, util::registry_from_ref};

fn hint_for_oras_output(stdout: &str, stderr: &str) -> Option<&'static str> {
    if stdout.contains("x509: certificate signed by unknown authority")
        || stderr.contains("x509: certificate signed by unknown authority")
    {
        return Some(
            "TLS verify failed (self-signed/unknown CA). Configure harbor.ca_file in crates/ntx-backend/conf/ntx-backend.yaml (or install the CA into the system trust store), then retry.",
        );
    }
    None
}

fn cap_text(s: &str, max_len: usize) -> (&str, bool) {
    if s.len() <= max_len {
        return (s, false);
    }
    // Safe because `max_len` is a byte index; adjust to char boundary.
    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (&s[..end], true)
}

fn format_oras_failure(
    op: &str,
    status: std::process::ExitStatus,
    stdout: &str,
    stderr: &str,
) -> anyhow::Error {
    const CAP: usize = 16 * 1024;

    let stdout = stdout.trim();
    let stderr = stderr.trim();

    let (stdout_cap, stdout_trunc) = cap_text(stdout, CAP);
    let (stderr_cap, stderr_trunc) = cap_text(stderr, CAP);

    let mut msg = String::new();
    msg.push_str(op);
    msg.push_str(" failed (exit=");
    msg.push_str(&status.to_string());
    msg.push_str(")");

    if !stdout_cap.is_empty() {
        msg.push_str("\n--- stdout ---\n");
        msg.push_str(stdout_cap);
        if stdout_trunc {
            msg.push_str("\n... <stdout truncated>");
        }
    }

    if !stderr_cap.is_empty() {
        msg.push_str("\n--- stderr ---\n");
        msg.push_str(stderr_cap);
        if stderr_trunc {
            msg.push_str("\n... <stderr truncated>");
        }
    }

    if let Some(hint) = hint_for_oras_output(stdout, stderr) {
        msg.push_str("\n--- hint ---\n");
        msg.push_str(hint);
    }

    anyhow::anyhow!(msg)
}

pub async fn maybe_oras_login(state: &AppState, reference: &str) -> anyhow::Result<()> {
    let Some(registry) = registry_from_ref(reference) else {
        anyhow::bail!("invalid ref (missing registry host): {reference}");
    };

    let Some(user) = state.harbor_user.as_deref() else {
        return Ok(());
    };

    let mut cmd = Command::new(&state.oras_bin);
    cmd.arg("login");

    if let Some(ca) = state.harbor_ca_file.as_deref() {
        cmd.arg("--ca-file").arg(ca);
    }

    cmd.arg("-u")
        .arg(user)
        .arg("--password-stdin")
        .arg(registry);

    let mut child = cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let Some(pass) = state.harbor_pass.as_deref() else {
        anyhow::bail!(
            "NTX_HARBOR_USER is set but NTX_HARBOR_PASS is missing; either set it or pre-login via oras credential store"
        );
    };

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin.write_all(pass.as_bytes()).await?;
    }

    let output = child.wait_with_output().await?;
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format_oras_failure(
            "oras login",
            output.status,
            &stdout,
            &stderr,
        ));
    }

    Ok(())
}

pub async fn oras_pull_to_dir(
    state: &AppState,
    reference: &str,
    out_dir: &Path,
) -> anyhow::Result<()> {
    let mut cmd = Command::new(&state.oras_bin);
    cmd.arg("pull").arg(reference).arg("-o").arg(out_dir);

    if let Some(ca) = state.harbor_ca_file.as_deref() {
        cmd.arg("--ca-file").arg(ca);
    }

    let output = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await?;

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format_oras_failure(
            "oras pull",
            output.status,
            &stdout,
            &stderr,
        ));
    }

    Ok(())
}

pub async fn oras_push_files(
    state: &AppState,
    reference: &str,
    artifact_type: &str,
    wasm_file: &Path,
    catalog_file: Option<&Path>,
    composed_wasm_file: Option<&Path>,
) -> anyhow::Result<()> {
    let base_dir = wasm_file
        .parent()
        .ok_or_else(|| anyhow::anyhow!("wasm_file has no parent dir: {}", wasm_file.display()))?;

    // ORAS rejects absolute file paths by default. All layer files we pass live in the same
    // tmp dir, so run from that directory and pass relative filenames.
    let rel_layer = |p: &Path, media_type: &str| -> anyhow::Result<String> {
        let rel = p
            .strip_prefix(base_dir)
            .unwrap_or(p)
            .to_string_lossy()
            .to_string();

        if rel.starts_with('/') {
            anyhow::bail!(
                "refusing to pass absolute path to oras (base_dir={} path={} rel={})",
                base_dir.display(),
                p.display(),
                rel
            );
        }

        Ok(format!("{rel}:{media_type}"))
    };

    let mut cmd = Command::new(&state.oras_bin);
    cmd.current_dir(base_dir)
        .arg("push")
        .arg(reference)
        .arg("--artifact-type")
        .arg(artifact_type)
        .arg(rel_layer(wasm_file, "application/wasm")?);

    if let Some(catalog_file) = catalog_file {
        cmd.arg(rel_layer(catalog_file, "application/json")?);
    }

    if let Some(composed) = composed_wasm_file {
        cmd.arg(rel_layer(composed, "application/wasm")?);
    }

    if let Some(ca) = state.harbor_ca_file.as_deref() {
        cmd.arg("--ca-file").arg(ca);
    }

    let output = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .context("run oras push")?;

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format_oras_failure(
            "oras push",
            output.status,
            &stdout,
            &stderr,
        ));
    }

    Ok(())
}
