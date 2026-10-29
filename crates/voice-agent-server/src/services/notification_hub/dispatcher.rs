use super::{
    NotificationHub,
    adapters::{ChannelDeliveryError, OutboundMessagePayload, ResolvedChannelTarget},
    store::{
        self, TelegramTargetConfig, ZaloTargetConfig, claim_due_outbox_batch,
        complete_outbox_failure, complete_outbox_sent, load_channel_config_row, now_millis,
        purge_expired_notifications,
    },
    types::{
        ExternalChannelKind, NotificationContent, NotificationHubError, NotificationSeverity,
        NotificationSourceKind, NotificationTopic, TestChannelInput, TestChannelOutput,
    },
};
use crate::database::{
    credentials,
    secrets::{SecretRef, SecretResolver, notification_channel_secret_env},
};
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

pub(super) fn backoff_delay_ms(attempt: i64, retry_after_secs: Option<u64>) -> i64 {
    if let Some(secs) = retry_after_secs {
        return (secs.clamp(1, 300) as i64) * 1_000;
    }
    match attempt {
        1 => 5_000,
        2 => 30_000,
        _ => 120_000,
    }
}

fn resolve_stored_secret(
    secret_resolver: &dyn SecretResolver,
    channel: ExternalChannelKind,
    encrypted_credential: Option<&str>,
) -> Result<crate::database::secrets::SecretValue, ChannelDeliveryError> {
    let fallback = notification_channel_secret_env(channel.as_str()).map(str::to_owned);
    let Some(raw_ref) =
        credentials::reference(channel.credential_scope(), encrypted_credential, fallback)
    else {
        return Err(ChannelDeliveryError::InvalidConfig("missing_bot_token"));
    };
    let secret_ref = SecretRef::parse(raw_ref)
        .map_err(|_| ChannelDeliveryError::InvalidConfig("invalid_bot_token"))?;
    secret_resolver
        .resolve(&secret_ref)
        .map_err(|_| ChannelDeliveryError::InvalidConfig("missing_bot_token"))
}

fn resolve_delivery_target(
    secret_resolver: &dyn SecretResolver,
    channel: ExternalChannelKind,
    target_json: &str,
    encrypted_credential: Option<&str>,
) -> Result<ResolvedChannelTarget, ChannelDeliveryError> {
    let token = resolve_stored_secret(secret_resolver, channel, encrypted_credential)?;
    match channel {
        ExternalChannelKind::Telegram => {
            let cfg: TelegramTargetConfig = serde_json::from_str(target_json)
                .map_err(|_| ChannelDeliveryError::InvalidConfig("invalid_target_config"))?;
            let chat_id = cfg.chat_id.trim().to_owned();
            if chat_id.is_empty() {
                return Err(ChannelDeliveryError::InvalidConfig("missing_chat_id"));
            }
            Ok(ResolvedChannelTarget {
                token,
                recipient_id: chat_id,
                thread_id: cfg.thread_id,
            })
        }
        ExternalChannelKind::Zalo => {
            let cfg: ZaloTargetConfig = serde_json::from_str(target_json)
                .map_err(|_| ChannelDeliveryError::InvalidConfig("invalid_target_config"))?;
            let recipient_id = cfg.recipient_id.trim().to_owned();
            if recipient_id.is_empty() {
                return Err(ChannelDeliveryError::InvalidConfig("missing_recipient_id"));
            }
            Ok(ResolvedChannelTarget {
                token,
                recipient_id,
                thread_id: None,
            })
        }
    }
}

pub(super) async fn process_due_outbox(
    hub: &NotificationHub,
) -> Result<usize, NotificationHubError> {
    let delivery_attempts = claim_due_outbox_batch(&hub.database, 32).await?;
    if delivery_attempts.is_empty() {
        return Ok(0);
    }

    let mut processed = 0usize;
    for attempt in delivery_attempts {
        processed += 1;
        if !attempt.channel_enabled {
            complete_outbox_failure(&hub.database, &attempt.outbox_id, "channel_disabled", None)
                .await?;
            continue;
        }
        let Some(adapter) = hub.adapters.get(&attempt.channel) else {
            complete_outbox_failure(
                &hub.database,
                &attempt.outbox_id,
                "adapter_unavailable",
                None,
            )
            .await?;
            continue;
        };

        let target = match resolve_delivery_target(
            hub.secret_resolver.as_ref(),
            attempt.channel,
            &attempt.target_json,
            attempt.encrypted_credential.as_deref(),
        ) {
            Ok(t) => t,
            Err(err) => {
                complete_outbox_failure(&hub.database, &attempt.outbox_id, err.error_code(), None)
                    .await?;
                continue;
            }
        };

        let payload = OutboundMessagePayload {
            notification_id: attempt.notification_id.clone(),
            content: attempt.content,
        };

        let delivery_span = tracing::info_span!(
            "notification_delivery",
            notification_id = %attempt.notification_id,
            channel = attempt.channel.as_str(),
        );
        let send_started = std::time::Instant::now();
        match tracing::Instrument::instrument(adapter.send(&target, &payload), delivery_span).await
        {
            Ok(receipt) => {
                tracing::info!(
                    subsystem = "notification",
                    event = "notification_channel_delivered",
                    outcome = "success",
                    channel = attempt.channel.as_str(),
                    notification_id = %attempt.notification_id,
                    attempts = attempt.attempts,
                    duration_ms = send_started.elapsed().as_millis() as u64,
                    "Notification delivered to external channel"
                );
                complete_outbox_sent(
                    &hub.database,
                    &attempt.outbox_id,
                    receipt.external_message_id.as_deref(),
                )
                .await?;
            }
            Err(err) => {
                let retry_delay_ms =
                    if err.is_retryable() && attempt.attempts < attempt.max_attempts {
                        Some(backoff_delay_ms(attempt.attempts, err.retry_after_secs()))
                    } else {
                        None
                    };
                tracing::warn!(
                    subsystem = "notification",
                    event = "notification_channel_failed",
                    outcome = "failure",
                    channel = attempt.channel.as_str(),
                    notification_id = %attempt.notification_id,
                    attempts = attempt.attempts,
                    error_code = err.error_code(),
                    retry_scheduled = retry_delay_ms.is_some(),
                    duration_ms = send_started.elapsed().as_millis() as u64,
                    "External notification delivery failed"
                );
                complete_outbox_failure(
                    &hub.database,
                    &attempt.outbox_id,
                    err.error_code(),
                    retry_delay_ms,
                )
                .await?;
            }
        }
    }

    if processed > 0
        && let Ok(unread_count) = store::count_unread(&hub.database).await
    {
        let _ = hub
            .web_tx
            .send(super::types::NotificationSseEvent::UnreadCountChanged { unread_count });
    }

    Ok(processed)
}

pub(super) async fn test_channel_delivery(
    hub: &NotificationHub,
    channel: ExternalChannelKind,
    input: TestChannelInput,
) -> Result<TestChannelOutput, NotificationHubError> {
    let now_ms = now_millis();
    let Some(adapter) = hub.adapters.get(&channel) else {
        return Ok(TestChannelOutput {
            ok: false,
            channel,
            external_message_id: None,
            delivered_at_ms: now_ms,
            error: Some("adapter_unavailable".to_owned()),
        });
    };

    let row = load_channel_config_row(&hub.database, channel).await?;

    let token = match input.bot_token {
        Some(draft) if !draft.expose().trim().is_empty() => {
            if !credentials::valid_input(&draft) {
                return Err(NotificationHubError::InvalidInput("invalid_bot_token"));
            }
            draft
        }
        _ => match resolve_stored_secret(
            hub.secret_resolver.as_ref(),
            channel,
            row.encrypted_credential.as_deref(),
        ) {
            Ok(secret) => secret,
            Err(err) => {
                return Ok(TestChannelOutput {
                    ok: false,
                    channel,
                    external_message_id: None,
                    delivered_at_ms: now_ms,
                    error: Some(err.error_code().to_owned()),
                });
            }
        },
    };

    let (recipient_id, thread_id) = match channel {
        ExternalChannelKind::Telegram => {
            let saved: TelegramTargetConfig =
                serde_json::from_str(&row.target_json).unwrap_or_default();
            let chat_id = input
                .chat_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| saved.chat_id.trim().to_owned());
            if chat_id.is_empty() {
                return Ok(TestChannelOutput {
                    ok: false,
                    channel,
                    external_message_id: None,
                    delivered_at_ms: now_ms,
                    error: Some("missing_chat_id".to_owned()),
                });
            }
            let thread = input.thread_id.or(saved.thread_id);
            (chat_id, thread)
        }
        ExternalChannelKind::Zalo => {
            let saved: ZaloTargetConfig =
                serde_json::from_str(&row.target_json).unwrap_or_default();
            let recipient = input
                .recipient_id
                .as_deref()
                .or(input.chat_id.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| saved.recipient_id.trim().to_owned());
            if recipient.is_empty() {
                return Ok(TestChannelOutput {
                    ok: false,
                    channel,
                    external_message_id: None,
                    delivered_at_ms: now_ms,
                    error: Some("missing_recipient_id".to_owned()),
                });
            }
            (recipient, None)
        }
    };

    let target = ResolvedChannelTarget {
        token,
        recipient_id,
        thread_id,
    };
    let body_text = input
        .message
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Kết nối kênh thông báo từ Voice Agent Server hoạt động tốt.")
        .to_owned();
    let payload = OutboundMessagePayload {
        notification_id: format!("test-{}", uuid::Uuid::new_v4()),
        content: NotificationContent {
            topic: NotificationTopic::ReminderDue,
            severity: NotificationSeverity::Info,
            title: "Thông báo thử nghiệm".to_owned(),
            body: body_text,
            source_kind: Some(NotificationSourceKind::System),
            source_id: None,
        },
    };

    match adapter.send(&target, &payload).await {
        Ok(receipt) => Ok(TestChannelOutput {
            ok: true,
            channel,
            external_message_id: receipt.external_message_id,
            delivered_at_ms: now_millis(),
            error: None,
        }),
        Err(err) => Ok(TestChannelOutput {
            ok: false,
            channel,
            external_message_id: None,
            delivered_at_ms: now_millis(),
            error: Some(err.error_code().to_owned()),
        }),
    }
}

pub(super) fn spawn_worker_loop(hub: Arc<NotificationHub>, stopping: CancellationToken) {
    tokio::spawn(async move {
        let mut outbox_tick = tokio::time::interval(Duration::from_secs(2));
        outbox_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut purge_tick = tokio::time::interval(Duration::from_secs(3600));
        purge_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                biased;
                () = stopping.cancelled() => break,
                () = hub.outbox_wake.notified() => {
                    while !stopping.is_cancelled() {
                        match process_due_outbox(&hub).await {
                            Ok(count) if count > 0 => continue,
                            _ => break,
                        }
                    }
                }
                _ = outbox_tick.tick() => {
                    let _ = process_due_outbox(&hub).await;
                }
                _ = purge_tick.tick() => {
                    let _ = purge_expired_notifications(&hub.database).await;
                }
            }
        }
    });
}
