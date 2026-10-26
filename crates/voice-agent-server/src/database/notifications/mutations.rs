use super::*;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateReminder {
    pub message: Option<String>,
    pub delay_seconds: Option<i64>,
    pub scheduled_at: Option<String>,
    pub timezone: Option<String>,
}
impl DeviceNotifications {
    pub async fn update_with_context(
        &self,
        device: &str,
        reminder: &str,
        revision: i64,
        ctx: &MutationContext<'_>,
        input: UpdateReminder,
    ) -> Result<(Reminder, i64), NotificationError> {
        self.mutate(device, reminder, revision, ctx, Some(input))
            .await
    }

    pub async fn cancel_with_context(
        &self,
        device: &str,
        reminder: &str,
        revision: i64,
        ctx: &MutationContext<'_>,
    ) -> Result<(Reminder, i64), NotificationError> {
        self.mutate(device, reminder, revision, ctx, None).await
    }

    async fn mutate(
        &self,
        device: &str,
        reminder_id: &str,
        revision: i64,
        ctx: &MutationContext<'_>,
        input: Option<UpdateReminder>,
    ) -> Result<(Reminder, i64), NotificationError> {
        if ctx.request_id.is_empty()
            || ctx.request_id.len() > 128
            || ctx.request_id.chars().any(char::is_control)
            || revision < 1
        {
            return Err(NotificationError::Invalid("validation_failed"));
        }
        let scope = ctx.agent_scope.or(self.agent_scope.as_deref());
        let operation = if input.is_some() { "update" } else { "cancel" };
        let payload = serde_json::to_vec(&(reminder_id, revision, &input))
            .map_err(|_| NotificationError::Invalid("validation_failed"))?;
        let hash = Sha256::digest(payload)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let mut tx = self.database.pool().begin_with("BEGIN IMMEDIATE").await?;
        let (owner, kind): (i64, String) =
            sqlx::query_as("SELECT d.id,d.kind FROM devices d JOIN agents a ON a.id=d.agent_id WHERE d.device_id=? AND (? IS NULL OR a.key=?)")
                .bind(device)
                .bind(scope).bind(scope)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(NotificationError::NotFound)?;
        if kind != "physical" {
            return Err(NotificationError::Invalid("unauthorized_target"));
        }
        if let Some((old, response)) = sqlx::query_as::<_, (String, String)>("SELECT payload_hash,response_json FROM reminder_mutation_keys WHERE caller_namespace=? AND request_id=? AND operation=? AND device_row_id=?").bind(ctx.caller).bind(ctx.request_id).bind(operation).bind(owner).fetch_optional(&mut *tx).await? {
            if hash != old {
                return Err(NotificationError::Conflict("idempotency_conflict"));
            }
            let mut reminder: Reminder = serde_json::from_str(&response)
                .map_err(|_| NotificationError::Invalid("stored_response_invalid"))?;
            reminder.sync_status = sync_status(&mut tx, owner, &reminder).await?;
            tx.commit().await?;
            return Ok((reminder, owner));
        }
        let mut reminder: Reminder = sqlx::query_as("SELECT r.id,d.device_id,r.message,r.due_at_ms,r.timezone,r.status,r.row_revision,r.schedule_revision,r.attempts_used,r.created_at_ms,r.updated_at_ms FROM device_reminders r JOIN devices d ON d.id=r.device_row_id WHERE r.id=? AND r.device_row_id=?").bind(reminder_id).bind(owner).fetch_optional(&mut *tx).await?.ok_or(NotificationError::NotFound)?;
        if reminder.row_revision != revision {
            tx.rollback().await?;
            super::super::audit::audit_conflict_action(
                self.database.pool(),
                ctx.audit_request_id.into(),
                "device_reminder_schedule",
                owner,
                revision,
                operation,
            )
            .await;
            return Err(NotificationError::Conflict("revision_conflict"));
        }
        if reminder.status != "pending" {
            return Err(NotificationError::Conflict("reminder_terminal"));
        }
        let now = (self.clock)();
        if let Some(input) = input {
            if input.message.is_none()
                && input.delay_seconds.is_none()
                && input.scheduled_at.is_none()
                && input.timezone.is_none()
            {
                return Err(NotificationError::Invalid("validation_failed"));
            }
            let time_changed = input.delay_seconds.is_some() || input.scheduled_at.is_some();
            let data = CreateReminder {
                message: input.message.unwrap_or_else(|| reminder.message.clone()),
                delay_seconds: input.delay_seconds,
                scheduled_at: input.scheduled_at.or_else(|| {
                    if time_changed {
                        None
                    } else {
                        chrono::DateTime::from_timestamp_millis(reminder.due_at_ms)
                            .map(|v| v.to_rfc3339())
                    }
                }),
                timezone: input.timezone.or_else(|| {
                    if time_changed {
                        None
                    } else {
                        Some(reminder.timezone.clone())
                    }
                }),
            };
            let (message, due, timezone) = validate_input(&data, now)?;
            reminder.message = message;
            reminder.due_at_ms = due;
            reminder.timezone = timezone;
        } else {
            reminder.status = "cancelled".into();
            sqlx::query("DELETE FROM reminder_delivery_attempts WHERE reminder_id=? AND status='claimed' AND started_at_ms IS NULL")
                .bind(reminder_id)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("UPDATE device_reminder_sync SET schedule_revision=schedule_revision+1 WHERE device_row_id=?").bind(owner).execute(&mut *tx).await?;
        let schedule: i64 = sqlx::query_scalar(
            "SELECT schedule_revision FROM device_reminder_sync WHERE device_row_id=?",
        )
        .bind(owner)
        .fetch_one(&mut *tx)
        .await?;
        reminder.row_revision += 1;
        reminder.schedule_revision = schedule;
        reminder.updated_at_ms = now;
        reminder.sync_status = "pending".into();
        sqlx::query("UPDATE device_reminders SET message=?,due_at_ms=?,timezone=?,status=?,row_revision=?,schedule_revision=?,updated_at_ms=?,cancelled_at_ms=?,next_attempt_at_ms=NULL WHERE id=? AND device_row_id=? AND row_revision=? AND status='pending'").bind(&reminder.message).bind(reminder.due_at_ms).bind(&reminder.timezone).bind(&reminder.status).bind(reminder.row_revision).bind(schedule).bind(now).bind(if operation=="cancel"{Some(now)}else{None}).bind(reminder_id).bind(owner).bind(revision).execute(&mut *tx).await?;
        let response = serde_json::to_string(&reminder)
            .map_err(|_| NotificationError::Invalid("validation_failed"))?;
        sqlx::query("INSERT INTO reminder_mutation_keys(caller_namespace,request_id,operation,device_row_id,payload_hash,response_json,created_at_ms) VALUES (?,?,?,?,?,?,?)").bind(ctx.caller).bind(ctx.request_id).bind(operation).bind(owner).bind(hash).bind(response).bind(now).execute(&mut *tx).await?;
        prune_keys(&mut tx, now).await?;
        audit(
            &mut *tx,
            ctx.audit_request_id,
            "device_reminder_schedule",
            Some(owner),
            operation,
            Some(schedule - 1),
            Some(schedule),
            AuditOutcome::Success,
            1,
        )
        .await?;
        tx.commit().await?;
        Ok((reminder, owner))
    }
}
