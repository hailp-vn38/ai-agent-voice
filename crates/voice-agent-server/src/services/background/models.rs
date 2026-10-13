//! Exact-version Background LLM admission, sharing the provider's Voice reservation.
use super::{JobError, ProviderPin, providers};
use crate::{
    config::AppConfig,
    database::Database,
    providers::{
        config_catalog::{ProviderOrigin, ProviderRef},
        llm::LlmRequest,
    },
    services::provider_runtime::ProviderRuntimeManager,
};
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

#[async_trait::async_trait]
pub(crate) trait JobModels: Send + Sync {
    async fn ready(&self, _: &ProviderPin, _: i64) -> Result<(), JobError> {
        Ok(())
    }
    async fn complete(
        &self,
        job_id: &str,
        pin: &ProviderPin,
        request: LlmRequest,
        deadline_at: i64,
        cancellation: CancellationToken,
    ) -> Result<String, JobError>;
}

pub(crate) struct ManagedModels {
    pub database: Arc<Database>,
    pub config: Arc<AppConfig>,
    pub manager: Arc<ProviderRuntimeManager>,
    pub quarantine: std::sync::Mutex<
        Vec<(
            crate::services::provider_runtime::ResourceLease,
            crate::workers::ProviderCapacityPermit,
        )>,
    >,
}

pub(crate) fn remaining(deadline_at: i64) -> Result<Duration, JobError> {
    let deadline = std::time::UNIX_EPOCH
        .checked_add(Duration::from_secs(
            u64::try_from(deadline_at).map_err(|_| JobError::DeadlineExceeded)?,
        ))
        .ok_or(JobError::DeadlineExceeded)?;
    deadline
        .duration_since(std::time::SystemTime::now())
        .map_err(|_| JobError::DeadlineExceeded)
}

impl ManagedModels {
    async fn acquire(
        &self,
        previous: &ProviderPin,
        deadline_at: i64,
        cancellation: CancellationToken,
    ) -> Result<
        (
            crate::services::provider_runtime::ResourceLease,
            Arc<crate::workers::LlmRuntime>,
        ),
        JobError,
    > {
        let mut connection = self.database.pool().acquire().await?;
        if &providers::pin(&mut connection, &self.config, &previous.reference).await? != previous {
            return Err(JobError::ConfigurationChanged);
        }
        let parsed = ProviderRef::parse(&previous.reference, Some("llm"))
            .map_err(|_| JobError::ProviderUnavailable)?;
        let snapshot = match parsed.origin {
            ProviderOrigin::Config => parsed
                .snapshot(&self.config)
                .map_err(|_| JobError::ProviderUnavailable)?,
            ProviderOrigin::Database => {
                let (id,key,kind,adapter,config_json,revision,secret_ref): (i64,String,String,String,String,i64,Option<String>) = sqlx::query_as("SELECT id,key,type,adapter,config_json,revision,credential_json FROM providers WHERE key=? AND type='llm' AND enabled=1")
                    .bind(&parsed.key).fetch_optional(&mut *connection).await?.ok_or(JobError::ProviderUnavailable)?;
                crate::database::DesiredProvider {
                    id,
                    key,
                    kind,
                    adapter,
                    config_json,
                    revision,
                    secret_ref,
                }
            }
        };
        drop(connection);
        let deadline = tokio::time::Instant::now() + remaining(deadline_at)?;
        let provider_wait_started = std::time::Instant::now();
        let lease = tokio::select! {
            _ = cancellation.cancelled() => return Err(JobError::Cancelled),
            result = async {
                match parsed.origin {
                    ProviderOrigin::Config => self.manager.acquire_deployment_until(snapshot, deadline).await,
                    ProviderOrigin::Database => self.manager.acquire_until(snapshot, deadline).await,
                }
            } => result.map_err(|_| JobError::ProviderUnavailable)?,
        };
        tracing::info!(event="background_provider_acquired", provider=%previous.reference, provider_wait_ms=provider_wait_started.elapsed().as_millis() as u64);
        let catalog = lease.runtimes().ok_or(JobError::ProviderUnavailable)?;
        let runtime = catalog
            .llm
            .get(&parsed.key)
            .cloned()
            .ok_or(JobError::ProviderUnavailable)?;
        Ok((lease, runtime))
    }
}

#[async_trait::async_trait]
impl JobModels for ManagedModels {
    async fn ready(&self, pin: &ProviderPin, deadline_at: i64) -> Result<(), JobError> {
        self.acquire(pin, deadline_at, CancellationToken::new())
            .await
            .map(|_| ())
    }

    async fn complete(
        &self,
        job_id: &str,
        previous: &ProviderPin,
        request: LlmRequest,
        deadline_at: i64,
        cancellation: CancellationToken,
    ) -> Result<String, JobError> {
        let bytes = request
            .messages
            .iter()
            .map(|message| match message {
                crate::providers::llm::ChatMessage::System { content }
                | crate::providers::llm::ChatMessage::User { content }
                | crate::providers::llm::ChatMessage::AssistantText { content }
                | crate::providers::llm::ChatMessage::ToolResult { content, .. } => content.len(),
                crate::providers::llm::ChatMessage::AssistantToolCalls { .. } => usize::MAX,
            })
            .fold(0usize, usize::saturating_add);
        if bytes > 65536 || !request.tools.is_empty() {
            return Err(JobError::CapacityUnavailable);
        }
        let (lease, runtime) = self
            .acquire(previous, deadline_at, cancellation.clone())
            .await?;
        let deadline = tokio::time::Instant::now() + remaining(deadline_at)?;
        let capacity_wait_started = std::time::Instant::now();
        let permit = loop {
            match runtime.admit_background() {
                Ok(permit) => break permit,
                Err(_) => tokio::select! {
                    _ = cancellation.cancelled() => return Err(JobError::Cancelled),
                    _ = tokio::time::sleep_until(deadline) => return Err(JobError::DeadlineExceeded),
                    _ = tokio::time::sleep(Duration::from_millis(20)) => {},
                },
            }
        };
        tracing::info!(event="background_provider_permit", job_id, provider=%previous.reference, capacity_wait_ms=capacity_wait_started.elapsed().as_millis() as u64);
        let publication = self.database.tool_security.publication.read().await;
        let row = self.database.job_row(job_id).await?;
        if !matches!(row.status.as_str(), "planning" | "running") {
            return Err(JobError::Cancelled);
        }
        let mut connection = self.database.pool().acquire().await?;
        crate::database::jobs::authorize(&mut connection, &row.requester()?).await?;
        let enabled: i64 =
            sqlx::query_scalar("SELECT enabled FROM background_settings WHERE singleton=1")
                .fetch_one(&mut *connection)
                .await?;
        if enabled != 1 {
            return Err(JobError::Disabled);
        }
        if &providers::pin(&mut connection, &self.config, &previous.reference).await? != previous {
            return Err(JobError::ConfigurationChanged);
        }
        drop(connection);
        let mut unknown = true;
        let opened = tokio::select! {
            _=cancellation.cancelled()=>Err(JobError::Cancelled),
            _=tokio::time::sleep_until(deadline)=>Err(JobError::DeadlineExceeded),
            result=tokio::time::timeout(Duration::from_secs(10),runtime.background_stream(request))=>match result {Ok(result)=>{if result.is_err(){unknown=false;}result.map_err(|_| JobError::ModelFailed)},Err(_)=>Err(JobError::ModelFailed)},
        };
        drop(publication);
        let outcome = match opened {
            Ok(mut stream) => {
                use futures_util::StreamExt;
                let consume = async {
                    let mut text = String::new();
                    while let Some(event) = stream.next().await {
                        match event {
                            Ok(crate::providers::LlmEvent::TextDelta(delta)) => {
                                if text.len().saturating_add(delta.len()) > 65536 {
                                    return Err(JobError::ModelFailed);
                                }
                                text.push_str(&delta);
                            }
                            Ok(crate::providers::LlmEvent::Finished) => return Ok(text),
                            _ => return Err(JobError::ModelFailed),
                        }
                    }
                    Ok(text)
                };
                tokio::select! {_=cancellation.cancelled()=>Err(JobError::Cancelled),_=tokio::time::sleep_until(deadline)=>Err(JobError::DeadlineExceeded),result=consume=>result}
            }
            Err(error) => Err(error),
        };
        if outcome.is_err() && unknown {
            self.quarantine
                .lock()
                .expect("Background LLM quarantine")
                .push((lease, permit));
        }
        outcome
    }
}
