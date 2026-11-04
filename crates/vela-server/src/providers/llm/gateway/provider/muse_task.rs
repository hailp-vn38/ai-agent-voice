use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio::sync::Semaphore;

use super::super::muse::{pool::job_entry, workflow::agent_task};
use super::LlmGatewayProvider;
use crate::providers::llm::{
    AgentTaskError, AgentTaskProvider, AgentTurnOutput, AgentTurnRequest, OperationJournal,
};
use crate::services::llm_gateway::{GatewayCallOutcome, GatewayVendor};

static SLOTS: LazyLock<Mutex<HashMap<String, Arc<Semaphore>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[async_trait::async_trait]
impl AgentTaskProvider for LlmGatewayProvider {
    async fn run_turn(
        &self,
        request: &AgentTurnRequest,
        journal: &dyn OperationJournal,
        progress: &(dyn for<'a> Fn(&'a str) + Send + Sync),
    ) -> Result<AgentTurnOutput, AgentTaskError> {
        let job = job_entry(&request.session_key);
        let pinned = request
            .operation
            .account_key
            .clone()
            .or_else(|| job.account_hint());
        let lease = if let Some(key) = pinned {
            self.hub
                .acquire_account_for_polling(&key)
                .map_err(|error| match error {
                    crate::services::llm_gateway::GatewayError::AccountReauthRequired => {
                        AgentTaskError::NeedsReauth
                    }
                    _ => AgentTaskError::RetryLater("pinned Muse account unavailable".into()),
                })?
        } else {
            self.hub
                .acquire_lease(&self.config.gateway_binding)
                .map_err(|_| AgentTaskError::Rejected("Muse account unavailable".into()))?
        };
        if lease.vendor != GatewayVendor::Muse {
            return Err(AgentTaskError::Rejected("provider is not Muse".into()));
        }
        let semaphore = {
            let mut slots = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
            Arc::clone(
                slots
                    .entry(lease.account_key.clone())
                    .or_insert_with(|| Arc::new(Semaphore::new(2))),
            )
        };
        let _permit = tokio::time::timeout(Duration::from_secs(60), semaphore.acquire_owned())
            .await
            .map_err(|_| AgentTaskError::RetryLater("Muse account task slots occupied".into()))?
            .map_err(|_| AgentTaskError::Rejected("task slots closed".into()))?;
        let pool = self.muse_pool_for_lease(&self.config, &lease);
        let output = agent_task::run_turn(&pool, job, request, journal, progress).await;
        if matches!(&output, Err(AgentTaskError::NeedsReauth)) {
            self.hub
                .report_outcome(&lease.account_key, GatewayCallOutcome::Unauthorized);
        }
        output
    }
}
