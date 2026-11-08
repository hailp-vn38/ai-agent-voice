#[tokio::test]
async fn expired_and_cancelled_calls_return_typed_errors() {
    let (_dir, host) = host(true);
    let mut ctx = context(Access::ReadOnly);
    ctx.deadline = tokio::time::Instant::now() - Duration::from_millis(1);
    assert_eq!(
        host.call(ctx, "wiki_status", json!({})).await,
        Err(ToolError::Timeout)
    );
    let ctx = CallCtx {
        access: Access::ReadOnly,
        cancel: CancellationToken::new(),
        deadline: tokio::time::Instant::now() + Duration::from_secs(1),
    };
    ctx.cancel.cancel();
    assert_eq!(
        host.call(ctx, "wiki_status", json!({})).await,
        Err(ToolError::Cancelled)
    );
}

#[tokio::test]
async fn call_timeout_and_concurrency_saturation_return_typed_errors() {
    let (_dir, host) = host(true);
    host.update_policy(Policy {
        call_timeout: Duration::ZERO,
        ..Policy::default()
    });
    assert_eq!(
        host.call(context(Access::ReadOnly), "wiki_status", json!({}))
            .await,
        Err(ToolError::Timeout)
    );

    host.update_policy(Policy {
        max_concurrency: 1,
        ..Policy::default()
    });
    host.active_requests.store(1, Ordering::Release);
    assert_eq!(
        host.call(context(Access::ReadOnly), "wiki_status", json!({}))
            .await,
        Err(ToolError::Unavailable)
    );
    host.active_requests.store(0, Ordering::Release);
}

use super::{context, host};
use crate::{Access, CallCtx, Policy, ToolError};
use serde_json::json;
use std::{sync::atomic::Ordering, time::Duration};
use tokio_util::sync::CancellationToken;
