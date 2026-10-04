use std::sync::Arc;

use futures_util::stream;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::{
    providers::llm::{LlmError, LlmEventStream},
    services::provider_diagnostic::{
        ProviderDiagnosticOperation, ProviderDiagnosticOperationError,
    },
};

struct ScriptedLlm {
    events: Vec<Result<LlmEvent, LlmError>>,
}

struct OpenStreamFailure;

#[async_trait::async_trait]
impl LlmProvider for OpenStreamFailure {
    fn adapter(&self) -> &'static str {
        "open-stream-failure"
    }

    async fn stream(&self, _: LlmRequest) -> Result<LlmEventStream, LlmError> {
        Err(LlmError::Failed)
    }
}

#[async_trait::async_trait]
impl LlmProvider for ScriptedLlm {
    fn adapter(&self) -> &'static str {
        "scripted"
    }

    async fn stream(&self, _: LlmRequest) -> Result<LlmEventStream, LlmError> {
        Ok(Box::pin(stream::iter(self.events.clone())))
    }
}

#[tokio::test]
async fn diagnostic_collects_text_from_a_tool_free_independent_request() {
    let runtime = LlmRuntime::new(
        Arc::new(ScriptedLlm {
            events: vec![
                Ok(LlmEvent::TextDelta("xin ".into())),
                Ok(LlmEvent::TextDelta("chao".into())),
                Ok(LlmEvent::Finished),
            ],
        }),
        2,
        Duration::from_secs(1),
    );

    let mut operation = runtime.diagnostic(LlmRequest::text_turn("hello".into()), 32 * 1024);
    assert_eq!(
        operation.execute(CancellationToken::new()).await,
        Ok("xin chao".into())
    );
}

#[tokio::test]
async fn diagnostic_rejects_tool_calls_and_oversized_text() {
    let tool_runtime = LlmRuntime::new(
        Arc::new(ScriptedLlm {
            events: vec![Ok(LlmEvent::ToolCall(ToolCall {
                id: "call-1".into(),
                name: "should_not_run".into(),
                arguments: serde_json::json!({}),
            }))],
        }),
        2,
        Duration::from_secs(1),
    );
    let mut tool_operation = tool_runtime.diagnostic(LlmRequest::text_turn("hello".into()), 16);
    assert_eq!(
        tool_operation.execute(CancellationToken::new()).await,
        Err(ProviderDiagnosticOperationError::InvalidResponse)
    );

    let oversized_runtime = LlmRuntime::new(
        Arc::new(ScriptedLlm {
            events: vec![Ok(LlmEvent::TextDelta("12345".into()))],
        }),
        2,
        Duration::from_secs(1),
    );
    let mut oversized_operation =
        oversized_runtime.diagnostic(LlmRequest::text_turn("hello".into()), 4);
    assert_eq!(
        oversized_operation.execute(CancellationToken::new()).await,
        Err(ProviderDiagnosticOperationError::InvalidResponse)
    );
}

#[tokio::test]
async fn session_reports_an_open_stream_failure_without_exposing_provider_details() {
    let runtime = LlmRuntime::new(Arc::new(OpenStreamFailure), 1, Duration::from_secs(1));
    let mut events = runtime.register_session("session", 1);
    let identity = WorkerIdentity::new("session", 1, 1);

    runtime
        .start(
            identity.clone(),
            LlmRequest::text_turn("hello".into()),
            CancellationToken::new(),
        )
        .expect("the operation starts");

    assert_eq!(
        events.recv().await,
        Some(LlmRuntimeEvent::Failed {
            identity,
            reason: LlmFailureReason::OpenStream,
        })
    );
}
