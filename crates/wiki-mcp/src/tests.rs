use crate::{Access, CallCtx, Policy, ToolHost};
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;
use wiki_core::Vault;

fn host(allow_write: bool) -> (tempfile::TempDir, ToolHost) {
    let dir = tempdir().unwrap();
    let vault = Arc::new(Vault::open(wiki_core::VaultConfig::new(dir.path())).unwrap());
    let host = ToolHost::new(
        vault,
        Policy {
            allow_write,
            max_concurrency: 2,
            call_timeout: Duration::from_secs(2),
            max_result_chars: 12_000,
        },
    );
    (dir, host)
}

fn context(access: Access) -> CallCtx {
    CallCtx {
        access,
        cancel: CancellationToken::new(),
        deadline: tokio::time::Instant::now() + Duration::from_secs(5),
    }
}

#[path = "tests/access.rs"]
mod access;
#[path = "tests/calls.rs"]
mod calls;
#[path = "tests/lifecycle.rs"]
mod lifecycle;
#[path = "tests/locking.rs"]
mod locking;
#[path = "tests/writes.rs"]
mod writes;
