mod files;
mod prompt;
mod tracker;

use super::super::client::VmError;
use super::super::pool::{JobEntry, JobSession, SharedMusePool, VmOperationLease};
use super::super::proto::SERVICE_DAEMON;
use crate::providers::llm::{
    AgentTaskError, AgentTurnAction, AgentTurnOutput, AgentTurnRequest, OperationJournal,
};
use orchestrator_agent_rs::{OperationUpdate, ToolOperationStatus};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::Instant;
use tracker::{TurnStatus, TurnTracker, user_turns};

const STALL_AFTER: Duration = Duration::from_secs(600);
const RECONNECT_GIVE_UP: Duration = Duration::from_secs(300);

fn vm_error(error: VmError) -> AgentTaskError {
    match error {
        VmError::AuthInvalid(_) => AgentTaskError::NeedsReauth,
        other => AgentTaskError::RetryLater(other.to_string()),
    }
}

async fn record(
    journal: &dyn OperationJournal,
    update: OperationUpdate,
) -> Result<(), AgentTaskError> {
    journal
        .record(update)
        .await
        .map_err(|_| AgentTaskError::OutcomeUnknown)
}

async fn history(lease: &VmOperationLease) -> Result<Value, AgentTaskError> {
    let response = lease
        .client()
        .send_request(
            "GET",
            &format!("/chat/history?session_id={}", lease.session_id()),
            None,
            SERVICE_DAEMON,
            Duration::from_secs(15),
            false,
        )
        .await
        .map_err(vm_error)?;
    if response.status == 404 {
        return Err(AgentTaskError::SessionLost);
    }
    if response.status != 200 {
        return Err(AgentTaskError::RetryLater(format!(
            "Muse history HTTP {}",
            response.status
        )));
    }
    let value = response.json().map_err(vm_error)?;
    if user_turns(&value).is_none() {
        return Err(AgentTaskError::OutcomeUnknown);
    }
    Ok(value)
}

pub(crate) fn allow_agent_task_approval(item: &Value) -> bool {
    tracing::info!(
        kind = item
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown"),
        "Muse task approval denied"
    );
    false
}

pub(crate) async fn run_turn(
    pool: &Arc<SharedMusePool>,
    job: Arc<JobEntry>,
    request: &AgentTurnRequest,
    journal: &dyn OperationJournal,
    progress: &(dyn Fn(&str) + Send + Sync),
) -> Result<AgentTurnOutput, AgentTaskError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AgentTaskError::Timeout { partial: None })?
        .as_secs() as i64;
    if request.operation.deadline_at <= now {
        return Err(AgentTaskError::Timeout { partial: None });
    }
    let deadline =
        Instant::now() + Duration::from_secs((request.operation.deadline_at - now) as u64);
    let slice_end = Instant::now() + request.wait_slice;
    let limit = deadline.min(slice_end);
    let mut slot = tokio::time::timeout_at(
        limit.min(Instant::now() + Duration::from_secs(60)),
        job.lock(),
    )
    .await
    .map_err(|_| AgentTaskError::Busy)?;
    let retained = slot.clone();
    let sid = request
        .operation
        .remote_session_id
        .clone()
        .or_else(|| retained.as_ref().map(|session| session.session_id.clone()));
    if sid.is_none() && matches!(request.action, AgentTurnAction::Collect { .. }) {
        return Err(AgentTaskError::NoSession);
    }
    let fresh = sid.is_none();
    let mut lease =
        tokio::time::timeout_at(limit.min(Instant::now() + Duration::from_secs(60)), async {
            match sid {
                Some(sid) => pool.acquire_existing_session(sid).await,
                None => pool.acquire_fresh().await,
            }
        })
        .await
        .map_err(|_| AgentTaskError::RetryLater("Muse slot wait expired".into()))?
        .map_err(|(kind, reason)| {
            if kind == super::super::pool::UpstreamFailureKind::AuthInvalid {
                AgentTaskError::NeedsReauth
            } else if fresh {
                AgentTaskError::Rejected(reason)
            } else {
                AgentTaskError::RetryLater(reason)
            }
        })?;
    lease.retain_session();
    if retained.as_ref().is_some_and(|session| {
        pool.gateway_account_key().as_deref() != Some(session.account_key.as_str())
    }) {
        return Err(AgentTaskError::RetryLater(
            "Muse session account was pinned concurrently".into(),
        ));
    }
    let completed = retained
        .as_ref()
        .map_or(0, |session| session.completed_turns);
    let account_key = lease.account_key().unwrap_or_default().to_owned();
    job.set_account_hint(Some(account_key.clone()));
    *slot = Some(JobSession::new(
        account_key.clone(),
        lease.session_id().to_owned(),
        Some(Arc::clone(lease.client())),
        false,
        None,
        Vec::new(),
    ));
    slot.as_mut().unwrap().completed_turns = completed;
    let mut events = lease.client().subscribe_events();
    let mut stream_events = Vec::new();
    let mut baseline = request.operation.user_turns_before;
    let resuming = matches!(
        request.operation.status,
        ToolOperationStatus::Running | ToolOperationStatus::Downloading
    );
    let mut data: Value = match request.operation.request_json.as_deref() {
        Some(raw) => serde_json::from_str(raw).map_err(|_| AgentTaskError::OutcomeUnknown)?,
        None => json!({}),
    };
    if !data.is_object() {
        return Err(AgentTaskError::OutcomeUnknown);
    }
    if baseline.is_some()
        && !resuming
        && matches!(&request.action, AgentTurnAction::Send { .. })
        && !data
            .get("pending_turn_items")
            .is_some_and(|items| items.as_array().is_some_and(|items| !items.is_empty()))
    {
        return Err(AgentTaskError::OutcomeUnknown);
    }
    let before = if fresh {
        None
    } else {
        match tokio::time::timeout_at(limit, history(&lease))
            .await
            .map_err(|_| AgentTaskError::RetryLater("history timed out".into()))?
        {
            Ok(value) => {
                if user_turns(&value).unwrap_or(0) < completed {
                    return Err(AgentTaskError::SessionLost);
                }
                Some(value)
            }
            Err(AgentTaskError::SessionLost) if completed == 0 => None,
            Err(error) => {
                return Err(if baseline.is_some() && !resuming {
                    AgentTaskError::OutcomeUnknown
                } else {
                    error
                });
            }
        }
    };
    let mut expected;
    match &request.action {
        AgentTurnAction::Send { prompt, .. } if !resuming => {
            let count = before.as_ref().and_then(user_turns).unwrap_or(0);
            let should_send = if let Some(saved) = baseline {
                if before.is_none() && !fresh {
                    return Err(AgentTaskError::OutcomeUnknown);
                }
                if count < saved {
                    return Err(AgentTaskError::SessionLost);
                }
                count == saved
            } else {
                if count > completed {
                    let mut previous = TurnTracker::new(count, Instant::now());
                    if let Some(value) = before.as_ref() {
                        previous.on_history(value, Instant::now());
                    }
                    if previous.status(Instant::now()) != TurnStatus::Done {
                        return Err(AgentTaskError::Busy);
                    }
                }
                baseline = Some(count);
                true
            };
            expected = baseline.unwrap() + 1;
            if should_send {
                let wrapped = prompt::build(&request.session_key, &request.action, prompt);
                let items = data
                    .get("pending_turn_items")
                    .cloned()
                    .unwrap_or_else(|| json!([{"type":"text", "text":wrapped}]));
                data["pending_turn_items"] = items.clone();
                record(
                    journal,
                    OperationUpdate {
                        status: Some(ToolOperationStatus::Submitting),
                        account_key: Some(account_key),
                        remote_session_id: Some(lease.session_id().to_owned()),
                        user_turns_before: baseline,
                        request_json: Some(data.to_string()),
                        ..Default::default()
                    },
                )
                .await?;
                if Instant::now() >= deadline {
                    return Err(AgentTaskError::Timeout { partial: None });
                }
                let body = json!({"items":items,"node_id":"root","session_id":lease.session_id(),"capabilities":["chat_cancel","delta_stream"]}).to_string();
                let response = tokio::time::timeout_at(
                    limit,
                    lease.client().send_request(
                        "POST",
                        "/chat/stream",
                        Some(body.as_bytes()),
                        SERVICE_DAEMON,
                        Duration::from_secs(30),
                        true,
                    ),
                )
                .await
                .map_err(|_| AgentTaskError::OutcomeUnknown)?
                .map_err(|_| AgentTaskError::OutcomeUnknown)?;
                if response.status >= 400 {
                    if response.status == 429 {
                        lease
                            .fail_account(super::super::pool::UpstreamFailureKind::QuotaExceeded)
                            .await;
                    }
                    return Err(match response.status {
                        401 | 403 => AgentTaskError::NeedsReauth,
                        429 => AgentTaskError::RetryLater("Muse quota exceeded".into()),
                        408 | 500..=599 => AgentTaskError::OutcomeUnknown,
                        status => AgentTaskError::Rejected(format!("Muse send HTTP {status}")),
                    });
                }
                if let Ok(value) = response.json() {
                    if let Some(sid) = value
                        .get("session_id")
                        .or_else(|| value.get("result").and_then(|v| v.get("session_id")))
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        lease.set_session_id(sid.to_owned());
                        slot.as_mut().unwrap().session_id = sid.to_owned();
                    }
                }
            }
            record(
                journal,
                OperationUpdate {
                    status: Some(ToolOperationStatus::Running),
                    remote_session_id: Some(lease.session_id().to_owned()),
                    ..Default::default()
                },
            )
            .await?;
        }
        AgentTurnAction::Collect { .. } => {
            record(
                journal,
                OperationUpdate {
                    account_key: Some(account_key.clone()),
                    remote_session_id: Some(lease.session_id().to_owned()),
                    ..Default::default()
                },
            )
            .await?;
            expected = baseline
                .map(|value| value + 1)
                .unwrap_or_else(|| before.as_ref().and_then(user_turns).unwrap_or(completed));
            if expected == 0 {
                return Err(AgentTaskError::NoSession);
            }
        }
        _ => {
            expected = baseline
                .map(|value| value + 1)
                .ok_or(AgentTaskError::OutcomeUnknown)?;
        }
    }
    let mut tracker = TurnTracker::new(expected, Instant::now());
    let mut poll_count = 0u32;
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(AgentTaskError::Timeout {
                partial: tracker.partial_text(),
            });
        }
        if now >= slice_end {
            return Err(AgentTaskError::RetryLater(
                "Muse still running; collect later".into(),
            ));
        }
        if now.saturating_duration_since(tracker.last_signal_at()) >= STALL_AFTER {
            return Err(AgentTaskError::Stalled {
                partial: tracker.partial_text(),
            });
        }
        if !lease.client().is_connected() {
            let started = Instant::now();
            let mut attempt = 0;
            loop {
                match tokio::time::timeout_at(
                    limit.min(started + RECONNECT_GIVE_UP),
                    lease.reconnect(),
                )
                .await
                {
                    Ok(Ok(receiver)) => {
                        events = receiver;
                        break;
                    }
                    Ok(Err((super::super::pool::UpstreamFailureKind::AuthInvalid, _))) => {
                        return Err(AgentTaskError::NeedsReauth);
                    }
                    Ok(Err((_, _))) | Err(_) => {
                        if Instant::now() >= (started + RECONNECT_GIVE_UP).min(limit) {
                            return Err(AgentTaskError::RetryLater(
                                "Muse reconnect unavailable".into(),
                            ));
                        }
                        let backoff = [1, 2, 5, 10, 30][attempt.min(4)];
                        attempt += 1;
                        tokio::time::sleep_until(
                            (Instant::now() + Duration::from_secs(backoff))
                                .min(limit)
                                .min(started + RECONNECT_GIVE_UP),
                        )
                        .await;
                    }
                }
            }
        }
        let response = tokio::time::timeout_at(limit, history(&lease)).await;
        while let Ok(event) = events.try_recv() {
            if event
                .session_id()
                .is_none_or(|sid| sid == lease.session_id())
            {
                if event.event_name().contains("approval") {
                    super::handle_approval_event(&lease, &event, allow_agent_task_approval).await;
                    return Err(AgentTaskError::Rejected("Muse approval denied".into()));
                }
                if event.event_name() == "delta.presentation"
                    || event.event_name().starts_with("task.")
                {
                    stream_events.push(event.raw.clone());
                }
                tracker.on_event(&event, Instant::now());
            }
        }
        match response {
            Ok(Ok(value)) => {
                if user_turns(&value).unwrap_or(0) < completed {
                    return Err(AgentTaskError::SessionLost);
                }
                tracker.on_history(&value, Instant::now());
                if tracker.status(Instant::now()) == TurnStatus::Done {
                    let message = tracker.partial_text().unwrap_or_default();
                    let turn_events = super::extract_history_turn_events(&value);
                    let mut paths = files::collect_file_paths(
                        &stream_events,
                        &turn_events,
                        std::slice::from_ref(&message),
                    );
                    if request.operation.status == ToolOperationStatus::Downloading {
                        let saved: Vec<String> = request
                            .operation
                            .batch_paths_json
                            .as_deref()
                            .and_then(|raw| serde_json::from_str(raw).ok())
                            .unwrap_or_default();
                        for path in saved {
                            if !paths.contains(&path) {
                                paths.push(path);
                            }
                        }
                    }
                    if let Some(contract) = prompt::contract(&request.session_key, &request.action)
                    {
                        if !paths.contains(&contract) {
                            paths.insert(0, contract);
                        }
                    }
                    record(
                        journal,
                        OperationUpdate {
                            status: Some(ToolOperationStatus::Downloading),
                            batch_index: Some(expected),
                            batch_paths_json: Some(json!(paths).to_string()),
                            ..Default::default()
                        },
                    )
                    .await?;
                    let downloaded =
                        files::download(&lease, paths, limit, deadline, &message).await?;
                    let lower = message.trim().to_ascii_lowercase();
                    let error_reply = [
                        "rate limit",
                        "quota exceeded",
                        "too many requests",
                        "you've reached",
                        "you have reached",
                        "sorry",
                        "we hit",
                        "i hit",
                        "hit a glitch",
                        "something went wrong",
                        "temporarily unavailable",
                        "error:",
                        "daily limit",
                        "usage limit",
                        "try again",
                    ]
                    .iter()
                    .any(|prefix| lower.starts_with(prefix));
                    if downloaded.is_empty() && error_reply {
                        if let Some((kind, reason)) = super::classify_assistant_error(&message) {
                            if kind == super::super::pool::UpstreamFailureKind::QuotaExceeded {
                                lease.fail_account(kind).await;
                            }
                            return Err(AgentTaskError::RetryLater(reason));
                        }
                    }
                    let files_missing = downloaded.is_empty()
                        && matches!(
                            &request.action,
                            AgentTurnAction::Send {
                                output: crate::providers::llm::AgentOutputKind::Files,
                                ..
                            } | AgentTurnAction::Collect {
                                output: crate::providers::llm::AgentOutputKind::Files,
                                ..
                            }
                        );
                    if files_missing
                        && matches!(&request.action, AgentTurnAction::Send { .. })
                        && data
                            .get("attachment_reminder_used")
                            .and_then(Value::as_bool)
                            != Some(true)
                    {
                        let items = json!([{"type":"text", "text":prompt::build(&request.session_key, &request.action, "Please attach the requested output files now. Do not repeat the research.")}]);
                        data["attachment_reminder_used"] = json!(true);
                        data["pending_turn_items"] = items.clone();
                        record(
                            journal,
                            OperationUpdate {
                                status: Some(ToolOperationStatus::Submitting),
                                user_turns_before: Some(expected),
                                request_json: Some(data.to_string()),
                                repairs_used: Some(1),
                                ..Default::default()
                            },
                        )
                        .await?;
                        let body = json!({"items":items,"node_id":"root","session_id":lease.session_id(),"capabilities":["chat_cancel","delta_stream"]}).to_string();
                        let response = tokio::time::timeout_at(
                            limit,
                            lease.client().send_request(
                                "POST",
                                "/chat/stream",
                                Some(body.as_bytes()),
                                SERVICE_DAEMON,
                                Duration::from_secs(30),
                                true,
                            ),
                        )
                        .await
                        .map_err(|_| AgentTaskError::OutcomeUnknown)?
                        .map_err(|_| AgentTaskError::OutcomeUnknown)?;
                        if response.status >= 400 {
                            return Err(AgentTaskError::OutcomeUnknown);
                        }
                        expected += 1;
                        record(
                            journal,
                            OperationUpdate {
                                status: Some(ToolOperationStatus::Running),
                                ..Default::default()
                            },
                        )
                        .await?;
                        tracker = TurnTracker::new(expected, Instant::now());
                        stream_events.clear();
                        continue;
                    }
                    slot.as_mut().unwrap().completed_turns = expected;
                    lease.finish_clean(None);
                    return Ok(AgentTurnOutput {
                        turn: expected,
                        message,
                        files: downloaded,
                        files_missing,
                    });
                }
            }
            Ok(Err(AgentTaskError::SessionLost)) if completed == 0 => (),
            Ok(Err(AgentTaskError::NeedsReauth)) => return Err(AgentTaskError::NeedsReauth),
            Ok(Err(AgentTaskError::SessionLost)) => return Err(AgentTaskError::SessionLost),
            _ => {
                if !lease.client().is_connected() {
                    continue;
                }
            }
        }
        let poll_delay = if poll_count < 5 {
            2
        } else if poll_count < 17 {
            5
        } else {
            15
        };
        poll_count += 1;
        let wake = (Instant::now() + Duration::from_secs(poll_delay)).min(limit);
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(wake) => break,
                _ = tokio::time::sleep(Duration::from_secs(1)) => if !lease.client().is_connected() { break; },
                event = events.recv() => match event {
                    Ok(event) if event.session_id().is_none_or(|sid| sid == lease.session_id()) => {
                        if event.event_name().contains("approval") { super::handle_approval_event(&lease, &event, allow_agent_task_approval).await; return Err(AgentTaskError::Rejected("Muse approval denied".into())); }
                        if event.event_name() == "delta.presentation" || event.event_name().starts_with("task.") { stream_events.push(event.raw.clone()); }
                        tracker.on_event(&event, Instant::now()); progress("Muse task active");
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    _ => (),
                }
            }
        }
    }
}
