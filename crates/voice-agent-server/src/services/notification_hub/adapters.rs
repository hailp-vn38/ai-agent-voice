use super::types::{ExternalChannelKind, NotificationContent};
use crate::database::secrets::SecretValue;
use reqwest::{Client, StatusCode, redirect};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

const TELEGRAM_API_BASE_URL: &str = "https://api.telegram.org";
const ZALO_BOT_API_BASE_URL: &str = "https://bot-api.zapps.me";
const OUTBOUND_HTTP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub struct ResolvedChannelTarget {
    pub token: SecretValue,
    pub recipient_id: String,
    pub thread_id: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct OutboundMessagePayload {
    pub notification_id: String,
    pub content: NotificationContent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExternalDeliveryReceipt {
    pub external_message_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelDeliveryError {
    InvalidConfig(&'static str),
    Unauthorized,
    BadRequest,
    RateLimited(Option<u64>),
    UpstreamTimeout,
    TransientNetwork,
}

impl ChannelDeliveryError {
    pub const fn error_code(&self) -> &'static str {
        match self {
            Self::InvalidConfig(code) => code,
            Self::Unauthorized => "unauthorized",
            Self::BadRequest => "invalid_recipient",
            Self::RateLimited(_) => "rate_limited",
            Self::UpstreamTimeout => "upstream_timeout",
            Self::TransientNetwork => "upstream_error",
        }
    }

    pub const fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimited(_) | Self::UpstreamTimeout | Self::TransientNetwork
        )
    }

    pub const fn retry_after_secs(&self) -> Option<u64> {
        match self {
            Self::RateLimited(secs) => *secs,
            _ => None,
        }
    }
}

pub fn format_outbound_text(content: &NotificationContent) -> String {
    let emoji = content.severity.emoji();
    let label = content.topic.label_vi();
    let title = content.title.trim();
    let body = content.body.trim();
    if body.is_empty() || body == title {
        format!("{emoji} [{label}] {title}")
    } else {
        format!("{emoji} [{label}] {title}\n{body}")
    }
}

#[async_trait::async_trait]
pub trait OutboundChannelAdapter: Send + Sync {
    fn channel_kind(&self) -> ExternalChannelKind;
    async fn send(
        &self,
        target: &ResolvedChannelTarget,
        payload: &OutboundMessagePayload,
    ) -> Result<ExternalDeliveryReceipt, ChannelDeliveryError>;
}

fn build_http_client() -> Client {
    Client::builder()
        .timeout(OUTBOUND_HTTP_TIMEOUT)
        .redirect(redirect::Policy::none())
        .build()
        .unwrap_or_else(|_| Client::new())
}

async fn post_bot_message(
    client: &Client,
    url: &str,
    body: &Value,
) -> Result<ExternalDeliveryReceipt, ChannelDeliveryError> {
    let response = client.post(url).json(body).send().await.map_err(|err| {
        if err.is_timeout() {
            ChannelDeliveryError::UpstreamTimeout
        } else {
            ChannelDeliveryError::TransientNetwork
        }
    })?;

    let status = response.status();
    let parsed: Value = response.json().await.unwrap_or(Value::Null);

    if status.is_success() {
        if parsed.get("ok").and_then(Value::as_bool) == Some(false) {
            return Err(ChannelDeliveryError::BadRequest);
        }
        let message_id = parsed
            .pointer("/result/message_id")
            .or_else(|| parsed.get("message_id"))
            .and_then(|v| {
                v.as_i64()
                    .map(|n| n.to_string())
                    .or_else(|| v.as_str().map(str::to_owned))
            });
        return Ok(ExternalDeliveryReceipt {
            external_message_id: message_id,
        });
    }

    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(ChannelDeliveryError::Unauthorized),
        StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND => Err(ChannelDeliveryError::BadRequest),
        StatusCode::TOO_MANY_REQUESTS => {
            let retry_after = parsed
                .pointer("/parameters/retry_after")
                .and_then(Value::as_u64);
            Err(ChannelDeliveryError::RateLimited(retry_after))
        }
        StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => {
            Err(ChannelDeliveryError::UpstreamTimeout)
        }
        _ => Err(ChannelDeliveryError::TransientNetwork),
    }
}

pub struct TelegramBotAdapter {
    client: Client,
    base_url: String,
}

impl Default for TelegramBotAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl TelegramBotAdapter {
    pub fn new() -> Self {
        Self {
            client: build_http_client(),
            base_url: TELEGRAM_API_BASE_URL.to_owned(),
        }
    }

    #[cfg(test)]
    pub fn with_base_url_for_test(base_url: impl Into<String>) -> Self {
        Self {
            client: build_http_client(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
        }
    }
}

#[async_trait::async_trait]
impl OutboundChannelAdapter for TelegramBotAdapter {
    fn channel_kind(&self) -> ExternalChannelKind {
        ExternalChannelKind::Telegram
    }

    async fn send(
        &self,
        target: &ResolvedChannelTarget,
        payload: &OutboundMessagePayload,
    ) -> Result<ExternalDeliveryReceipt, ChannelDeliveryError> {
        let chat_id = target.recipient_id.trim();
        if chat_id.is_empty() {
            return Err(ChannelDeliveryError::InvalidConfig("missing_chat_id"));
        }
        let token = target.token.expose().trim();
        if token.is_empty() {
            return Err(ChannelDeliveryError::InvalidConfig("missing_bot_token"));
        }

        let url = format!("{}/bot{}/sendMessage", self.base_url, token);
        let mut body = json!({
            "chat_id": chat_id,
            "text": format_outbound_text(&payload.content),
            "disable_web_page_preview": true,
        });
        if let Some(thread_id) = target.thread_id {
            body["message_thread_id"] = json!(thread_id);
        }

        post_bot_message(&self.client, &url, &body).await
    }
}

pub struct ZaloBotAdapter {
    client: Client,
    base_url: String,
}

impl Default for ZaloBotAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl ZaloBotAdapter {
    pub fn new() -> Self {
        Self {
            client: build_http_client(),
            base_url: ZALO_BOT_API_BASE_URL.to_owned(),
        }
    }

    #[cfg(test)]
    pub fn with_base_url_for_test(base_url: impl Into<String>) -> Self {
        Self {
            client: build_http_client(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
        }
    }
}

#[async_trait::async_trait]
impl OutboundChannelAdapter for ZaloBotAdapter {
    fn channel_kind(&self) -> ExternalChannelKind {
        ExternalChannelKind::Zalo
    }

    async fn send(
        &self,
        target: &ResolvedChannelTarget,
        payload: &OutboundMessagePayload,
    ) -> Result<ExternalDeliveryReceipt, ChannelDeliveryError> {
        let recipient_id = target.recipient_id.trim();
        if recipient_id.is_empty() {
            return Err(ChannelDeliveryError::InvalidConfig("missing_recipient_id"));
        }
        let token = target.token.expose().trim();
        if token.is_empty() {
            return Err(ChannelDeliveryError::InvalidConfig("missing_bot_token"));
        }

        let url = format!("{}/bot{}/sendMessage", self.base_url, token);
        let body = json!({
            "chat_id": recipient_id,
            "text": format_outbound_text(&payload.content),
        });

        post_bot_message(&self.client, &url, &body).await
    }
}

#[derive(Debug, Clone)]
pub struct RecordedDelivery {
    pub channel: ExternalChannelKind,
    pub recipient_id: String,
    pub thread_id: Option<i64>,
    pub notification_id: String,
    pub text: String,
}

#[derive(Clone)]
pub struct RecordingChannelAdapter {
    channel: ExternalChannelKind,
    deliveries: Arc<Mutex<Vec<RecordedDelivery>>>,
    scripted_outcomes: Arc<Mutex<VecDeque<Result<ExternalDeliveryReceipt, ChannelDeliveryError>>>>,
}

impl RecordingChannelAdapter {
    pub fn new(channel: ExternalChannelKind) -> Self {
        Self {
            channel,
            deliveries: Arc::new(Mutex::new(Vec::new())),
            scripted_outcomes: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub fn push_outcome(&self, outcome: Result<ExternalDeliveryReceipt, ChannelDeliveryError>) {
        self.scripted_outcomes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(outcome);
    }

    pub fn recorded(&self) -> Vec<RecordedDelivery> {
        self.deliveries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

#[async_trait::async_trait]
impl OutboundChannelAdapter for RecordingChannelAdapter {
    fn channel_kind(&self) -> ExternalChannelKind {
        self.channel
    }

    async fn send(
        &self,
        target: &ResolvedChannelTarget,
        payload: &OutboundMessagePayload,
    ) -> Result<ExternalDeliveryReceipt, ChannelDeliveryError> {
        if let Some(outcome) = self
            .scripted_outcomes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front()
        {
            if outcome.is_ok() {
                self.deliveries
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(RecordedDelivery {
                        channel: self.channel,
                        recipient_id: target.recipient_id.clone(),
                        thread_id: target.thread_id,
                        notification_id: payload.notification_id.clone(),
                        text: format_outbound_text(&payload.content),
                    });
            }
            return outcome;
        }

        let seq = {
            let mut guard = self.deliveries.lock().unwrap_or_else(|e| e.into_inner());
            guard.push(RecordedDelivery {
                channel: self.channel,
                recipient_id: target.recipient_id.clone(),
                thread_id: target.thread_id,
                notification_id: payload.notification_id.clone(),
                text: format_outbound_text(&payload.content),
            });
            guard.len()
        };

        Ok(ExternalDeliveryReceipt {
            external_message_id: Some(format!("rec-{}-{seq}", self.channel.as_str())),
        })
    }
}

pub fn default_outbound_adapters() -> Vec<Arc<dyn OutboundChannelAdapter>> {
    vec![
        Arc::new(TelegramBotAdapter::new()),
        Arc::new(ZaloBotAdapter::new()),
    ]
}
