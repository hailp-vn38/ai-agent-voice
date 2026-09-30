use std::{collections::HashMap, sync::Arc, time::Duration};

use tokio_util::sync::CancellationToken;

use super::*;
use crate::{
    providers::{
        DatabaseRuntimeState, LlmProvider,
        llm::{LlmError, LlmEventStream, LlmRequest},
    },
    workers::LlmRuntime,
};

struct TestLlm;

#[async_trait::async_trait]
impl LlmProvider for TestLlm {
    fn adapter(&self) -> &'static str {
        "test"
    }

    async fn stream(&self, _: LlmRequest) -> Result<LlmEventStream, LlmError> {
        Err(LlmError::Failed)
    }
}

fn service(desired_revision: i64) -> ProviderDiagnosticService {
    let registry = RuntimeCatalog {
        llm: HashMap::from([(
            "llm-a".into(),
            Arc::new(LlmRuntime::new(
                Arc::new(TestLlm),
                2,
                Duration::from_secs(1),
            )),
        )]),
        ..RuntimeCatalog::default()
    };
    let snapshot = DatabaseRuntimeSnapshot::from_states([(
        "llm-a".into(),
        DatabaseRuntimeState {
            provider_id: 42,
            desired_revision,
            status: DatabaseRuntimeStatus::Loaded,
            failure: None,
        },
    )]);
    ProviderDiagnosticService::new(
        Arc::new(registry),
        Some(Arc::new(snapshot)),
        ProviderDiagnosticLimiter::new(1),
        Duration::from_secs(1),
    )
}

fn target(revision: i64) -> ProviderDiagnosticTarget {
    ProviderDiagnosticTarget {
        key: "llm-a".into(),
        provider_type: ProviderType::Llm,
        desired_revision: revision,
    }
}

#[test]
fn terminal_acknowledgement_releases_runtime_and_diagnostic_capacity() {
    let service = service(7);
    let lease = service.begin(target(7)).unwrap();
    assert_eq!(lease.metadata.tested_runtime, DatabaseRuntimeStatus::Loaded);
    lease.acknowledge_terminal();
    assert!(service.begin(target(7)).is_ok());
}

#[test]
fn quarantine_releases_limiter_but_keeps_exact_runtime_capacity_consumed() {
    let service = service(7);
    service.begin(target(7)).unwrap().quarantine();
    assert!(matches!(
        service.begin(target(7)),
        Err(ProviderDiagnosticError::Busy)
    ));
}

#[test]
fn stale_runtime_is_explicit_but_remains_testable() {
    let service = service(7);
    let lease = service.begin(target(8)).unwrap();
    assert!(!lease.metadata.runtime_matches_desired);
    assert!(lease.metadata.requires_restart);
}

struct TimeoutOperation {
    events: Arc<std::sync::Mutex<Vec<&'static str>>>,
    terminal_acknowledges: bool,
}

#[async_trait::async_trait]
impl ProviderDiagnosticOperation for TimeoutOperation {
    type Output = ();

    async fn execute(
        &mut self,
        _: CancellationToken,
    ) -> Result<Self::Output, ProviderDiagnosticOperationError> {
        std::future::pending().await
    }

    fn cancel_exact(&mut self) {
        self.events.lock().unwrap().push("cancel");
    }

    async fn await_terminal_acknowledgement(&mut self) -> bool {
        self.events.lock().unwrap().push("ack");
        self.terminal_acknowledges
    }

    fn quarantine_exact(&mut self) {
        self.events.lock().unwrap().push("quarantine");
    }
}

#[tokio::test]
async fn timeout_cancels_then_acknowledges_before_releasing_capacity() {
    let service = ProviderDiagnosticService::new(
        Arc::new(RuntimeCatalog {
            llm: HashMap::from([(
                "llm-a".into(),
                Arc::new(LlmRuntime::new(
                    Arc::new(TestLlm),
                    2,
                    Duration::from_secs(1),
                )),
            )]),
            ..RuntimeCatalog::default()
        }),
        Some(Arc::new(DatabaseRuntimeSnapshot::from_states([(
            "llm-a".into(),
            DatabaseRuntimeState {
                provider_id: 42,
                desired_revision: 7,
                status: DatabaseRuntimeStatus::Loaded,
                failure: None,
            },
        )]))),
        ProviderDiagnosticLimiter::new(1),
        Duration::from_millis(1),
    );
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    assert!(matches!(
        service
            .execute(
                target(7),
                TimeoutOperation {
                    events: Arc::clone(&events),
                    terminal_acknowledges: true
                },
                Duration::from_secs(1)
            )
            .await,
        Err(ProviderDiagnosticError::Timeout)
    ));
    assert_eq!(*events.lock().unwrap(), ["cancel", "ack"]);
    assert!(service.begin(target(7)).is_ok());
}

#[tokio::test]
async fn timeout_quarantines_the_exact_operation_when_acknowledgement_is_missing() {
    let service = service(7);
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    assert!(matches!(
        service
            .execute(
                target(7),
                TimeoutOperation {
                    events: Arc::clone(&events),
                    terminal_acknowledges: false,
                },
                Duration::from_secs(1),
            )
            .await,
        Err(ProviderDiagnosticError::Timeout)
    ));
    assert_eq!(*events.lock().unwrap(), ["cancel", "ack", "quarantine"]);
    assert!(matches!(
        service.begin(target(7)),
        Err(ProviderDiagnosticError::Busy)
    ));
}
