pub mod reminder_scheduler;

use crate::{
    database::{
        Database,
        notifications::{
            AgentReminderQuery, CompleteAttemptOutcome, CreateReminder, DeviceNotifications,
            FailAttemptOutcome, MutationContext, NotificationError, Reminder, UpdateReminder,
        },
    },
    services::device::active_sessions::ActiveDeviceSessions,
};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Notify;

/// Deep module unifying durable reminder persistence, live WebSocket `sync_required`
/// notifications, and event-driven scheduler wakeups.
#[derive(Clone)]
pub struct ReminderCoordinator {
    repo: DeviceNotifications,
    sessions: Arc<ActiveDeviceSessions>,
    scheduler_wakeup: Arc<Notify>,
}

impl ReminderCoordinator {
    pub fn new(repo: DeviceNotifications, sessions: Arc<ActiveDeviceSessions>) -> Self {
        let scheduler_wakeup = Arc::clone(sessions.scheduler_wakeup());
        Self {
            repo,
            sessions,
            scheduler_wakeup,
        }
    }

    pub fn from_database(database: Database, sessions: Arc<ActiveDeviceSessions>) -> Self {
        Self::new(DeviceNotifications::new(database), sessions)
    }

    pub fn scoped_to_agent(mut self, key: &str) -> Self {
        self.repo = self.repo.scoped_to_agent(key);
        self
    }

    pub fn repo(&self) -> &DeviceNotifications {
        &self.repo
    }

    pub fn sessions(&self) -> &Arc<ActiveDeviceSessions> {
        &self.sessions
    }

    pub fn scheduler_wakeup(&self) -> &Arc<Notify> {
        &self.scheduler_wakeup
    }

    fn notify_schedule_changed(&self, device_row_id: i64, schedule_revision: i64) {
        if let Some(session) = self.sessions.current(device_row_id) {
            tracing::info!(
                device_row_id,
                session_id = %session.session_id,
                schedule_revision,
                reminder_capable = session.features.reminders,
                "reminder sync_required dispatched to active device session"
            );
        } else {
            tracing::info!(
                device_row_id,
                schedule_revision,
                "reminder schedule changed while device is offline; sync pending reconnect"
            );
        }
        self.sessions.sync_required(device_row_id, schedule_revision);
        self.scheduler_wakeup.notify_one();
    }

    fn enrich_item_capability(&self, device_row_id: i64, item: &mut Reminder) {
        if item.status == "pending"
            && self
                .sessions
                .current(device_row_id)
                .is_some_and(|s| !s.features.reminders)
        {
            item.sync_status = "unsupported".into();
        }
    }

    pub async fn create(
        &self,
        device_id: &str,
        ctx: &MutationContext<'_>,
        input: CreateReminder,
    ) -> Result<Reminder, NotificationError> {
        let (mut reminder, device_row_id) = match self
            .repo
            .create_with_context(device_id, ctx, input)
            .await
        {
            Ok(res) => res,
            Err(err) => {
                tracing::warn!(
                    device_id = %device_id,
                    source = %ctx.source.as_str(),
                    caller = %ctx.caller,
                    error = %err,
                    "reminder create failed"
                );
                return Err(err);
            }
        };
        self.enrich_item_capability(device_row_id, &mut reminder);
        let due_at_utc = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(reminder.due_at_ms)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default();
        tracing::info!(
            reminder_id = %reminder.id,
            device_id = %reminder.device_id,
            device_row_id,
            due_at_ms = reminder.due_at_ms,
            due_at_utc = %due_at_utc,
            timezone = %reminder.timezone,
            source = %ctx.source.as_str(),
            caller = %ctx.caller,
            schedule_revision = reminder.schedule_revision,
            sync_status = %reminder.sync_status,
            message = %reminder.message,
            "reminder created"
        );
        self.notify_schedule_changed(device_row_id, reminder.schedule_revision);
        Ok(reminder)
    }

    pub async fn update(
        &self,
        device_id: &str,
        reminder_id: &str,
        expected_revision: i64,
        ctx: &MutationContext<'_>,
        input: UpdateReminder,
    ) -> Result<Reminder, NotificationError> {
        let (mut reminder, device_row_id) = match self
            .repo
            .update_with_context(device_id, reminder_id, expected_revision, ctx, input)
            .await
        {
            Ok(res) => res,
            Err(err) => {
                tracing::warn!(
                    device_id = %device_id,
                    reminder_id = %reminder_id,
                    expected_revision,
                    source = %ctx.source.as_str(),
                    caller = %ctx.caller,
                    error = %err,
                    "reminder update failed"
                );
                return Err(err);
            }
        };
        self.enrich_item_capability(device_row_id, &mut reminder);
        let due_at_utc = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(reminder.due_at_ms)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default();
        tracing::info!(
            reminder_id = %reminder.id,
            device_id = %reminder.device_id,
            device_row_id,
            due_at_ms = reminder.due_at_ms,
            due_at_utc = %due_at_utc,
            timezone = %reminder.timezone,
            source = %ctx.source.as_str(),
            caller = %ctx.caller,
            row_revision = reminder.row_revision,
            schedule_revision = reminder.schedule_revision,
            sync_status = %reminder.sync_status,
            message = %reminder.message,
            "reminder updated"
        );
        self.notify_schedule_changed(device_row_id, reminder.schedule_revision);
        Ok(reminder)
    }

    pub async fn cancel(
        &self,
        device_id: &str,
        reminder_id: &str,
        expected_revision: i64,
        ctx: &MutationContext<'_>,
    ) -> Result<Reminder, NotificationError> {
        let (reminder, device_row_id) = match self
            .repo
            .cancel_with_context(device_id, reminder_id, expected_revision, ctx)
            .await
        {
            Ok(res) => res,
            Err(err) => {
                tracing::warn!(
                    device_id = %device_id,
                    reminder_id = %reminder_id,
                    expected_revision,
                    source = %ctx.source.as_str(),
                    caller = %ctx.caller,
                    error = %err,
                    "reminder cancel failed"
                );
                return Err(err);
            }
        };
        tracing::info!(
            reminder_id = %reminder.id,
            device_id = %reminder.device_id,
            device_row_id,
            source = %ctx.source.as_str(),
            caller = %ctx.caller,
            row_revision = reminder.row_revision,
            schedule_revision = reminder.schedule_revision,
            "reminder cancelled"
        );
        self.notify_schedule_changed(device_row_id, reminder.schedule_revision);
        Ok(reminder)
    }

    pub async fn list(
        &self,
        device_id: &str,
        status: Option<&str>,
        page: u32,
        page_size: u32,
    ) -> Result<Value, NotificationError> {
        let mut value = self.repo.list(device_id, status, page, page_size).await?;
        let device_row_id = self
            .repo
            .database()
            .device_row_id(device_id)
            .await
            .ok()
            .flatten();
        let session = device_row_id.and_then(|id| self.sessions.current(id));
        let connected = session.is_some();
        let reminder_capable = session.as_ref().is_some_and(|s| s.features.reminders);
        if let Some(summary) = value
            .get_mut("schedule_summary")
            .and_then(Value::as_object_mut)
        {
            summary.insert("session_connected".into(), serde_json::json!(connected));
            summary.insert(
                "reminder_capable".into(),
                serde_json::json!(reminder_capable),
            );
        }
        if connected
            && !reminder_capable
            && let Some(items) = value.get_mut("items").and_then(Value::as_array_mut)
        {
            for item in items {
                if item.get("status").and_then(Value::as_str) == Some("pending")
                    && let Some(obj) = item.as_object_mut()
                {
                    obj.insert("sync_status".into(), Value::String("unsupported".into()));
                }
            }
        }
        Ok(value)
    }

    pub async fn list_agent(
        &self,
        agent_key: &str,
        query: &AgentReminderQuery,
    ) -> Result<Value, NotificationError> {
        let mut value = self.repo.list_agent(agent_key, query).await?;
        if let Some(items) = value.get_mut("items").and_then(Value::as_array_mut) {
            let mut capability_cache = std::collections::HashMap::<String, bool>::new();
            for item in items {
                if item.get("status").and_then(Value::as_str) != Some("pending") {
                    continue;
                }
                let Some(device_id) = item
                    .get("device_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                else {
                    continue;
                };
                let unsupported = if let Some(&cached) = capability_cache.get(&device_id) {
                    cached
                } else {
                    let row_id = self
                        .repo
                        .database()
                        .device_row_id(&device_id)
                        .await
                        .ok()
                        .flatten();
                    let is_unsupported = row_id
                        .and_then(|id| self.sessions.current(id))
                        .is_some_and(|s| !s.features.reminders);
                    capability_cache.insert(device_id, is_unsupported);
                    is_unsupported
                };
                if unsupported && let Some(obj) = item.as_object_mut() {
                    obj.insert("sync_status".into(), Value::String("unsupported".into()));
                }
            }
        }
        Ok(value)
    }

    pub async fn mark_attempt_started(
        &self,
        reminder_id: &str,
        attempt_id: &str,
        session_id: &str,
        estimated_audio_ms: i64,
    ) -> Result<bool, NotificationError> {
        let started = self
            .repo
            .mark_attempt_started(reminder_id, attempt_id, session_id, estimated_audio_ms)
            .await?;
        if started {
            self.scheduler_wakeup.notify_one();
        }
        Ok(started)
    }

    pub async fn complete_attempt_with_outcome(
        &self,
        device_row_id: i64,
        session_id: &str,
        reminder_id: &str,
        attempt_id: &str,
    ) -> Result<Option<CompleteAttemptOutcome>, NotificationError> {
        let outcome = self
            .repo
            .complete_attempt_with_outcome(device_row_id, session_id, reminder_id, attempt_id)
            .await?;
        if let Some(ref outcome) = outcome {
            tracing::info!(
                reminder_id,
                attempt_id,
                device_row_id,
                session_id,
                schedule_revision = outcome.schedule_revision,
                duplicate_ack = outcome.duplicate_ack,
                "reminder playback completed (device ack received)"
            );
            if !outcome.duplicate_ack {
                self.notify_schedule_changed(device_row_id, outcome.schedule_revision);
            }
        }
        Ok(outcome)
    }

    pub async fn fail_attempt(
        &self,
        device_row_id: i64,
        session_id: &str,
        reminder_id: &str,
        attempt_id: &str,
        reason: &str,
    ) -> Result<Option<FailAttemptOutcome>, NotificationError> {
        let outcome = self
            .repo
            .fail_attempt(device_row_id, session_id, reminder_id, attempt_id, reason)
            .await?;
        if let Some(ref outcome) = outcome {
            tracing::warn!(
                reminder_id,
                attempt_id,
                device_row_id,
                session_id,
                reason,
                expired = outcome.expired,
                schedule_revision = outcome.schedule_revision,
                "reminder delivery attempt failed"
            );
            if outcome.expired {
                self.notify_schedule_changed(device_row_id, outcome.schedule_revision);
            } else {
                self.scheduler_wakeup.notify_one();
            }
        }
        Ok(outcome)
    }

    pub async fn on_session_closed(
        &self,
        device_row_id: i64,
        session_id: &str,
    ) -> Result<Option<i64>, NotificationError> {
        let new_revision = self
            .repo
            .on_session_closed(device_row_id, session_id)
            .await?;
        if let Some(rev) = new_revision {
            tracing::warn!(
                device_row_id,
                session_id,
                schedule_revision = rev,
                "in-flight reminder expired on session disconnect"
            );
            self.notify_schedule_changed(device_row_id, rev);
        } else {
            self.scheduler_wakeup.notify_one();
        }
        Ok(new_revision)
    }

    pub async fn reap_expired_leases_and_overdue(
        &self,
    ) -> Result<Vec<(i64, i64)>, NotificationError> {
        let changed = self.repo.reap_expired_leases_and_overdue().await?;
        if !changed.is_empty() {
            tracing::warn!(
                expired_devices = ?changed,
                "reminder reaper expired overdue or exhausted reminders"
            );
        }
        for &(dev_id, rev) in &changed {
            self.sessions.sync_required(dev_id, rev);
        }
        Ok(changed)
    }
}

#[derive(Clone, Debug)]
pub struct BoundReminderTarget {
    pub device_row_id: i64,
    pub device_id: String,
    pub expires_at: tokio::time::Instant,
}

#[derive(Clone)]
pub enum SessionReminderTarget {
    Physical {
        device_row_id: i64,
        device_id: String,
    },
    WebAssistant {
        assistant_device_id: String,
        bound_target: Arc<std::sync::Mutex<Option<BoundReminderTarget>>>,
    },
}

#[derive(Clone)]
pub struct SessionReminderTools {
    pub coordinator: ReminderCoordinator,
    pub agent_id: i64,
    pub agent_key: String,
    pub default_timezone: String,
    pub target: SessionReminderTarget,
}

impl SessionReminderTools {
    fn active_bound_target(
        bound_target: &std::sync::Mutex<Option<BoundReminderTarget>>,
    ) -> Option<BoundReminderTarget> {
        let mut guard = bound_target.lock().unwrap_or_else(|e| e.into_inner());
        if guard
            .as_ref()
            .is_some_and(|t| t.expires_at <= tokio::time::Instant::now())
        {
            *guard = None;
        }
        guard.clone()
    }

    pub fn turn_context(&self) -> crate::session::prompt::ReminderTurnContext {
        let now_ms = self.coordinator.repo().clock_now();
        let current_time = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(now_ms)
            .unwrap_or_default()
            .to_rfc3339();
        let (current_device_id, device_kind, target_device_id) = match &self.target {
            SessionReminderTarget::Physical { device_id, .. } => {
                (device_id.clone(), "physical".to_owned(), None)
            }
            SessionReminderTarget::WebAssistant {
                assistant_device_id,
                bound_target,
            } => (
                assistant_device_id.clone(),
                "web_assistant".to_owned(),
                Self::active_bound_target(bound_target).map(|t| t.device_id),
            ),
        };
        crate::session::prompt::ReminderTurnContext {
            timezone: self.default_timezone.clone(),
            current_time,
            current_device_id,
            device_kind,
            target_device_id,
            reminder_tools_enabled: true,
        }
    }

    pub async fn execute(
        &self,
        session_id: &str,
        call_id: &str,
        tool_name: &str,
        arguments: Value,
    ) -> Result<Value, &'static str> {
        let result = self
            .execute_inner(session_id, call_id, tool_name, &arguments)
            .await;
        match &result {
            Ok(output) => {
                tracing::info!(
                    event = "voice_reminder_tool_succeeded",
                    session_id = %session_id,
                    agent_key = %self.agent_key,
                    tool = %tool_name,
                    call_id = %call_id,
                    result = %output,
                    "voice reminder tool call succeeded"
                );
            }
            Err(code) => {
                tracing::warn!(
                    event = "voice_reminder_tool_failed",
                    session_id = %session_id,
                    agent_key = %self.agent_key,
                    tool = %tool_name,
                    call_id = %call_id,
                    error_code = %code,
                    arguments = %arguments,
                    "voice reminder tool call failed"
                );
            }
        }
        result
    }

    async fn execute_inner(
        &self,
        session_id: &str,
        call_id: &str,
        tool_name: &str,
        arguments: &Value,
    ) -> Result<Value, &'static str> {
        match self.coordinator.repo().grant(&self.agent_key).await {
            Ok((true, _)) => {}
            Ok((false, _)) => return Err("reminders_disabled"),
            Err(_) => return Err("device_notifications_unavailable"),
        }
        let Some(obj) = arguments.as_object() else {
            return Err("invalid_arguments");
        };
        let (target_device_id, source) = self.resolve_target_device(obj).await?;
        let caller = format!("voice:{session_id}");
        let ctx = MutationContext {
            caller: &caller,
            request_id: call_id,
            audit_request_id: call_id,
            source,
            agent_scope: Some(&self.agent_key),
        };
        let scoped = self.coordinator.clone().scoped_to_agent(&self.agent_key);
        match tool_name {
            crate::tools::builtin::CREATE_REMINDER_TOOL_NAME
            | crate::tools::builtin::LEGACY_CREATE_REMINDER_TOOL_NAME => {
                Self::execute_create(&scoped, &target_device_id, &ctx, obj).await
            }
            crate::tools::builtin::LIST_REMINDERS_TOOL_NAME
            | crate::tools::builtin::LEGACY_LIST_REMINDERS_TOOL_NAME => {
                Self::execute_list(&scoped, &target_device_id, obj).await
            }
            crate::tools::builtin::UPDATE_REMINDER_TOOL_NAME => {
                Self::execute_update(&scoped, &target_device_id, &ctx, obj).await
            }
            crate::tools::builtin::CANCEL_REMINDER_TOOL_NAME
            | crate::tools::builtin::LEGACY_DELETE_REMINDER_TOOL_NAME
            | crate::tools::builtin::LEGACY_COMPLETE_REMINDER_TOOL_NAME => {
                Self::execute_cancel(&scoped, &target_device_id, &ctx, obj).await
            }
            _ => Err("unknown_tool"),
        }
    }

    async fn resolve_target_device(
        &self,
        obj: &serde_json::Map<String, Value>,
    ) -> Result<(String, crate::database::notifications::ReminderSource), &'static str> {
        match &self.target {
            SessionReminderTarget::Physical { device_id, .. } => {
                if let Some(arg_dev) = obj.get("device_id").and_then(Value::as_str)
                    && arg_dev != device_id
                {
                    return Err("unauthorized_target");
                }
                Ok((
                    device_id.clone(),
                    crate::database::notifications::ReminderSource::Voice,
                ))
            }
            SessionReminderTarget::WebAssistant { bound_target, .. } => {
                let Some(bound) = Self::active_bound_target(bound_target) else {
                    return Err("target_selection_required");
                };
                if let Some(arg_dev) = obj.get("device_id").and_then(Value::as_str)
                    && arg_dev != bound.device_id
                {
                    return Err("unauthorized_target");
                }
                let valid = self
                    .coordinator
                    .repo()
                    .is_active_physical_device_for_agent(
                        bound.device_row_id,
                        &bound.device_id,
                        self.agent_id,
                    )
                    .await
                    .map_err(|_| "device_notifications_unavailable")?;
                if !valid {
                    return Err("target_selection_required");
                }
                Ok((
                    bound.device_id,
                    crate::database::notifications::ReminderSource::WebAssistant,
                ))
            }
        }
    }

    async fn execute_create(
        scoped: &ReminderCoordinator,
        target_device_id: &str,
        ctx: &MutationContext<'_>,
        obj: &serde_json::Map<String, Value>,
    ) -> Result<Value, &'static str> {
        for key in obj.keys() {
            if !matches!(
                key.as_str(),
                "message" | "delay_seconds" | "scheduled_at" | "timezone" | "device_id"
            ) {
                return Err("invalid_arguments");
            }
        }
        let Some(message) = obj.get("message").and_then(Value::as_str) else {
            return Err("invalid_arguments");
        };
        let delay_seconds = match obj.get("delay_seconds") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_i64().ok_or("invalid_arguments")?),
        };
        let scheduled_at = match obj.get("scheduled_at") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_str().ok_or("invalid_arguments")?.to_owned()),
        };
        let timezone = match obj.get("timezone") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_str().ok_or("invalid_arguments")?.to_owned()),
        };
        let reminder = scoped
            .create(
                target_device_id,
                ctx,
                CreateReminder {
                    message: message.to_owned(),
                    delay_seconds,
                    scheduled_at,
                    timezone,
                },
            )
            .await
            .map_err(map_notification_error)?;
        Ok(reminder_tool_response("saved", &reminder))
    }

    async fn execute_list(
        scoped: &ReminderCoordinator,
        target_device_id: &str,
        obj: &serde_json::Map<String, Value>,
    ) -> Result<Value, &'static str> {
        for key in obj.keys() {
            if !matches!(key.as_str(), "status" | "limit" | "device_id") {
                return Err("invalid_arguments");
            }
        }
        let status = match obj.get("status") {
            None | Some(Value::Null) => Some("pending"),
            Some(v) => Some(v.as_str().ok_or("invalid_arguments")?),
        };
        let limit = match obj.get("limit") {
            None | Some(Value::Null) => 20u32,
            Some(v) => {
                let raw = v.as_u64().ok_or("invalid_arguments")?;
                if !(1..=50).contains(&raw) {
                    return Err("invalid_arguments");
                }
                raw as u32
            }
        };
        let list = scoped
            .list(target_device_id, status, 1, limit)
            .await
            .map_err(map_notification_error)?;
        Ok(serde_json::json!({
            "ok": true,
            "device_id": target_device_id,
            "items": list["items"],
            "total": list["total"],
            "schedule_summary": list["schedule_summary"],
        }))
    }

    async fn execute_update(
        scoped: &ReminderCoordinator,
        target_device_id: &str,
        ctx: &MutationContext<'_>,
        obj: &serde_json::Map<String, Value>,
    ) -> Result<Value, &'static str> {
        for key in obj.keys() {
            if !matches!(
                key.as_str(),
                "reminder_id"
                    | "expected_revision"
                    | "message"
                    | "delay_seconds"
                    | "scheduled_at"
                    | "timezone"
                    | "device_id"
            ) {
                return Err("invalid_arguments");
            }
        }
        let Some(reminder_id) = obj
            .get("reminder_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            return Err("invalid_arguments");
        };
        let Some(expected_revision) = obj
            .get("expected_revision")
            .and_then(Value::as_i64)
            .filter(|r| *r >= 1)
        else {
            return Err("invalid_arguments");
        };
        let message = match obj.get("message") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_str().ok_or("invalid_arguments")?.to_owned()),
        };
        let delay_seconds = match obj.get("delay_seconds") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_i64().ok_or("invalid_arguments")?),
        };
        let scheduled_at = match obj.get("scheduled_at") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_str().ok_or("invalid_arguments")?.to_owned()),
        };
        let timezone = match obj.get("timezone") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_str().ok_or("invalid_arguments")?.to_owned()),
        };
        let reminder = scoped
            .update(
                target_device_id,
                reminder_id,
                expected_revision,
                ctx,
                UpdateReminder {
                    message,
                    delay_seconds,
                    scheduled_at,
                    timezone,
                },
            )
            .await
            .map_err(map_notification_error)?;
        Ok(reminder_tool_response("saved", &reminder))
    }

    async fn execute_cancel(
        scoped: &ReminderCoordinator,
        target_device_id: &str,
        ctx: &MutationContext<'_>,
        obj: &serde_json::Map<String, Value>,
    ) -> Result<Value, &'static str> {
        for key in obj.keys() {
            if !matches!(
                key.as_str(),
                "reminder_id" | "expected_revision" | "device_id"
            ) {
                return Err("invalid_arguments");
            }
        }
        let Some(reminder_id) = obj
            .get("reminder_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            return Err("invalid_arguments");
        };
        let Some(expected_revision) = obj
            .get("expected_revision")
            .and_then(Value::as_i64)
            .filter(|r| *r >= 1)
        else {
            return Err("invalid_arguments");
        };
        let reminder = scoped
            .cancel(target_device_id, reminder_id, expected_revision, ctx)
            .await
            .map_err(map_notification_error)?;
        Ok(reminder_tool_response("cancelled", &reminder))
    }
}

fn map_notification_error(error: NotificationError) -> &'static str {
    match error {
        NotificationError::Invalid(code) => code,
        NotificationError::NotFound => "not_found",
        NotificationError::Conflict(code) => code,
        NotificationError::Sql(_) => "device_notifications_unavailable",
    }
}

fn reminder_tool_response(status: &str, reminder: &Reminder) -> Value {
    let scheduled_at = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(reminder.due_at_ms)
        .map(|dt| dt.to_rfc3339());
    serde_json::json!({
        "ok": true,
        "status": status,
        "reminder_id": reminder.id,
        "device_id": reminder.device_id,
        "message": reminder.message,
        "due_at_ms": reminder.due_at_ms,
        "scheduled_at": scheduled_at,
        "timezone": reminder.timezone,
        "row_revision": reminder.row_revision,
        "schedule_revision": reminder.schedule_revision,
        "sync_status": reminder.sync_status,
    })
}
