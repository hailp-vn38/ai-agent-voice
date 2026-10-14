//! Private RPC to the shared installer/runner; no Docker socket or image builder.
use super::{
    PluginError,
    archive::{Package, Runtime},
};
use crate::config::BackgroundConfig;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};

pub(super) fn digest_for_package(package_digest: &str) -> String {
    let bytes = Sha256::digest(format!("{package_digest}:shared-v1").as_bytes());
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(super) async fn rpc(socket: &Path, request: Value) -> Result<Value, PluginError> {
    let result = tokio::time::timeout(Duration::from_secs(240), async {
        let mut stream = UnixStream::connect(socket).await?;
        let bytes = serde_json::to_vec(&request)?;
        if bytes.len() > 80 * 1024 {
            anyhow::bail!("RPC request too large");
        }
        stream.write_all(&bytes).await?;
        stream.shutdown().await?;
        let mut response = Vec::new();
        (&mut stream)
            .take(80 * 1024 + 1)
            .read_to_end(&mut response)
            .await?;
        if response.len() > 80 * 1024 || !response.ends_with(b"\n") {
            anyhow::bail!("invalid RPC response size");
        }
        let reply: Value = serde_json::from_slice(&response)?;
        if reply["ok"] != true {
            anyhow::bail!("remote runtime rejected request");
        }
        reply
            .get("value")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing value"))
    })
    .await;
    result
        .map_err(|_| PluginError::WorkerTimeout)?
        .map_err(|_| PluginError::SandboxUnavailable)
}

pub(super) async fn preflight(config: &BackgroundConfig, _: &Runtime) -> Result<(), PluginError> {
    let path = config
        .plugin_installer_socket
        .as_deref()
        .ok_or(PluginError::SandboxUnavailable)?;
    if !path.exists() {
        return Err(PluginError::SandboxUnavailable);
    }
    Ok(())
}
pub(super) async fn runner_preflight(config: &BackgroundConfig) -> Result<(), PluginError> {
    let path = config
        .plugin_runner_socket
        .as_deref()
        .ok_or(PluginError::SandboxUnavailable)?;
    if !path.exists() {
        return Err(PluginError::SandboxUnavailable);
    }
    Ok(())
}
pub(super) async fn build(
    config: &BackgroundConfig,
    stage: &Path,
    package: &Package,
) -> Result<String, PluginError> {
    preflight(config, &package.manifest.runtime).await?;
    let stage_id = stage
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(PluginError::InvalidPackage)?;
    let socket = config
        .plugin_installer_socket
        .as_deref()
        .ok_or(PluginError::SandboxUnavailable)?;
    let value = rpc(
        socket,
        json!({"action":"Prepare","stage_id":stage_id,"package_digest":package.digest}),
    )
    .await?;
    let digest = value["installation_digest"]
        .as_str()
        .ok_or(PluginError::SandboxUnavailable)?;
    if digest != digest_for_package(&package.digest) {
        return Err(PluginError::SandboxUnavailable);
    }
    Ok(digest.to_owned())
}
pub(super) async fn verify_image(
    config: &BackgroundConfig,
    installation: &str,
    package_digest: &str,
    _runtime: &Runtime,
) -> Result<(), PluginError> {
    if installation != digest_for_package(package_digest) {
        return Err(PluginError::InvalidPackage);
    }
    let root = config
        .plugin_installations_dir
        .as_deref()
        .ok_or(PluginError::SandboxUnavailable)?;
    let path = root.join(installation).join(".installation.json");
    let contents = tokio::fs::read(path)
        .await
        .map_err(|_| PluginError::SandboxUnavailable)?;
    let receipt: Value =
        serde_json::from_slice(&contents).map_err(|_| PluginError::SandboxUnavailable)?;
    if receipt["installation_digest"] != installation || receipt["package_digest"] != package_digest
    {
        return Err(PluginError::SandboxUnavailable);
    }
    Ok(())
}
