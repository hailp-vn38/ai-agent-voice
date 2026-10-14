//! The only Background authority shared with an admitted Voice Session.
use super::{JobEngine, JobRequest, JobRequester};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

#[derive(Clone)]
pub struct VoiceJobSubmit {
    engine: Option<Arc<JobEngine>>,
    requester: JobRequester,
}
impl VoiceJobSubmit {
    pub fn admitted(engine: Option<Arc<JobEngine>>, device: i64, agent: i64) -> Option<Self> {
        (device > 0 && agent > 0).then_some(Self {
            engine,
            requester: JobRequester::Voice {
                device_incarnation: device,
                agent_incarnation: agent,
            },
        })
    }
    pub async fn submit(&self, arguments: Value, key: &str, budget: Duration) -> Value {
        if arguments.to_string().len() > 16384 {
            return rejection("invalid_arguments");
        }
        let request: JobRequest = match serde_json::from_value::<JobRequest>(arguments)
            .ok()
            .and_then(|request| request.normalize().ok())
        {
            Some(request) => request,
            None => return rejection("invalid_arguments"),
        };
        let Some(engine) = &self.engine else {
            return rejection("runtime_unavailable");
        };
        let Ok(_permit) = engine.voice_submissions.clone().try_acquire_owned() else {
            return rejection("background_capacity_unavailable");
        };
        let outcome = tokio::time::timeout(
            budget.min(Duration::from_secs(5)),
            engine.submit(request, &self.requester, Some(key)),
        )
        .await;
        match outcome {
            Ok(Ok(receipt)) => json!({"created":true,"job_id":receipt.job.id,"status":"queued"}),
            Ok(Err(error)) => rejection(match error {
                super::JobError::AuthorityRevoked => "device_not_authorized",
                _ => error.code(),
            }),
            Err(_) => {
                let replay = tokio::time::timeout(
                    Duration::from_millis(500),
                    engine.database.receipt_by_key(&self.requester, key),
                )
                .await;
                match replay {
                    Ok(Ok(Some(receipt))) => {
                        json!({"created":true,"job_id":receipt.job.id,"status":"queued"})
                    }
                    _ => rejection("submission_outcome_unknown"),
                }
            }
        }
    }
}
pub(crate) fn rejection(code: &str) -> Value {
    json!({"created":false,"isError":true,"error":{"code":code}})
}
