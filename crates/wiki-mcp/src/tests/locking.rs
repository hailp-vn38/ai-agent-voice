use super::context;
use crate::{Access, Policy, ToolError, ToolHost};
use serde_json::json;
use std::{
    process::Command,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use tempfile::tempdir;
use wiki_core::Vault;

#[test]
fn subprocess_holds_vault_writer_lease() {
    let Ok(root) = std::env::var("WIKI_MCP_LOCK_HOLDER_ROOT") else {
        return;
    };
    let ready = std::path::PathBuf::from(&root).join("lock-holder-ready");
    let release = std::path::PathBuf::from(&root).join("lock-holder-release");
    let _vault = Vault::open(wiki_core::VaultConfig::new(&root)).unwrap();
    std::fs::write(ready, "ready").unwrap();
    let started = Instant::now();
    while !release.exists() && started.elapsed() < Duration::from_secs(10) {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        release.exists(),
        "parent did not release subprocess lock holder"
    );
}

#[tokio::test]
async fn write_through_toolhost_returns_read_only_when_another_process_holds_the_lock() {
    let dir = tempdir().unwrap();
    let executable = std::env::current_exe().unwrap();
    let mut child = Command::new(executable)
        .arg("--exact")
        .arg("tests::locking::subprocess_holds_vault_writer_lease")
        .arg("--nocapture")
        .env("WIKI_MCP_LOCK_HOLDER_ROOT", dir.path())
        .spawn()
        .unwrap();
    let ready = dir.path().join("lock-holder-ready");
    let started = Instant::now();
    while !ready.exists() && started.elapsed() < Duration::from_secs(5) {
        thread::sleep(Duration::from_millis(10));
    }
    if !ready.exists() {
        let _ = child.kill();
        let _ = child.wait();
        panic!("subprocess did not acquire the vault writer lease");
    }

    let vault = Arc::new(Vault::open(wiki_core::VaultConfig::new(dir.path())).unwrap());
    let host = ToolHost::new(vault, Policy::default());
    let result = host
        .call(
            context(Access::ReadWrite),
            "wiki_write_file",
            json!({"scope":"wiki","rel_path":"locked.md","content":"# Locked"}),
        )
        .await;
    std::fs::write(dir.path().join("lock-holder-release"), "release").unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(result, Err(ToolError::ReadOnly));
}
