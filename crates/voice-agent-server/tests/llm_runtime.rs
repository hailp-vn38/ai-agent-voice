use std::{sync::Arc, time::Duration};

use futures_util::stream;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use voice_agent_server::{
    providers::{LlmError, LlmEvent, LlmProvider, llm::LlmEventStream},
    workers::{LlmRuntime, LlmRuntimeEvent, WorkerIdentity},
};

struct EventProvider(Vec<Result<LlmEvent, LlmError>>);

#[async_trait::async_trait]
impl LlmProvider for EventProvider {
    fn adapter(&self) -> &'static str {
        "test"
    }

    async fn stream(&self, _: String) -> Result<LlmEventStream, LlmError> {
        Ok(Box::pin(stream::iter(self.0.clone())))
    }
}

struct PendingProvider;

#[async_trait::async_trait]
impl LlmProvider for PendingProvider {
    fn adapter(&self) -> &'static str {
        "pending"
    }

    async fn stream(&self, _: String) -> Result<LlmEventStream, LlmError> {
        Ok(Box::pin(futures_util::stream::pending()))
    }
}

#[tokio::test]
async fn runtime_routes_domain_events_and_releases_permit_after_terminal() {
    let runtime = LlmRuntime::new(
        Arc::new(EventProvider(vec![
            Ok(LlmEvent::TextDelta("xin chao".into())),
            Ok(LlmEvent::Finished),
        ])),
        1,
        Duration::from_secs(1),
    );
    let mut events = runtime.register_session("session", 4);
    let first = WorkerIdentity::new("session", 1, 1);
    runtime
        .start(first.clone(), "prompt".into(), CancellationToken::new())
        .unwrap();
    assert_eq!(
        events.recv().await,
        Some(LlmRuntimeEvent::TextDelta {
            identity: first.clone(),
            text: "xin chao".into()
        })
    );
    assert_eq!(
        events.recv().await,
        Some(LlmRuntimeEvent::Finished { identity: first })
    );

    // A terminal event drops the owned permit, so the next generation can be accepted.
    let second = WorkerIdentity::new("session", 2, 2);
    runtime
        .start(second.clone(), "prompt".into(), CancellationToken::new())
        .unwrap();
    assert_eq!(
        events.recv().await,
        Some(LlmRuntimeEvent::TextDelta {
            identity: second.clone(),
            text: "xin chao".into()
        })
    );
    assert_eq!(
        events.recv().await,
        Some(LlmRuntimeEvent::Finished { identity: second })
    );
}

#[tokio::test]
async fn unexpected_tool_call_is_terminal_without_a_retry_or_mcp_event() {
    let runtime = LlmRuntime::new(
        Arc::new(EventProvider(vec![Ok(LlmEvent::UnexpectedToolCall)])),
        1,
        Duration::from_secs(1),
    );
    let mut events = runtime.register_session("session", 2);
    let identity = WorkerIdentity::new("session", 3, 3);
    runtime
        .start(identity.clone(), "prompt".into(), CancellationToken::new())
        .unwrap();
    assert_eq!(
        timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap(),
        Some(LlmRuntimeEvent::UnexpectedToolCall { identity })
    );
    assert!(events.try_recv().is_err());
}

#[tokio::test]
async fn turn_owned_cancellation_token_terminates_the_matching_operation() {
    let runtime = LlmRuntime::new(Arc::new(PendingProvider), 1, Duration::from_secs(1));
    let mut events = runtime.register_session("session", 2);
    let identity = WorkerIdentity::new("session", 4, 4);
    let cancellation = CancellationToken::new();
    runtime
        .start(identity.clone(), "prompt".into(), cancellation.clone())
        .unwrap();

    cancellation.cancel();

    assert_eq!(
        timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap(),
        Some(LlmRuntimeEvent::Cancelled { identity })
    );
}
