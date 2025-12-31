use std::path::Path;

use anyhow::Context;
use tokio::process::Command;

use crate::{state::AppState, util::registry_from_ref};

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
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("oras login failed: {stderr}");
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
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("oras pull failed: {stderr}");
    }

    Ok(())
}

pub async fn oras_push_files(
    state: &AppState,
    reference: &str,
    artifact_type: &str,
    wasm_file: &Path,
    catalog_file: Option<&Path>,
) -> anyhow::Result<()> {
    let mut cmd = Command::new(&state.oras_bin);
    cmd.arg("push")
        .arg(reference)
        .arg("--artifact-type")
        .arg(artifact_type)
        .arg(format!("{}:application/wasm", wasm_file.display()));

    if let Some(catalog_file) = catalog_file {
        cmd.arg(format!("{}:application/json", catalog_file.display()));
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
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("oras push failed: {stderr}");
    }

    Ok(())
}
