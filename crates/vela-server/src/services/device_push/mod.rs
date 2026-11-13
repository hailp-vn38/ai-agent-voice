//! Device Push Channel: every convention of the channel and the three things callers may ask.
//!
//! - OTA asks for a registered Device's connection details ([`DevicePush::credentials`]).
//! - The reminder scheduler asks to wake a Device ([`DevicePush::send_wake_signal`]).
//! - The broker asks who may connect and touch a topic ([`DevicePush::authenticate`],
//!   [`DevicePush::authorize_topic`]).
//!
//! Topic names, the Device Key, password derivation, the Wake Signal payload, the service
//! account and the broker connection stay inside so those callers cannot drift apart. The
//! module exists only when `[device_push]` is enabled.

mod mqtt;
mod publisher;

pub use publisher::{RecordingPublisher, WakePublishError, WakePublisher};

use crate::config::DevicePushConfig;
use ring::hmac;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::sync::{Arc, OnceLock};
use tokio_util::sync::CancellationToken;

const TOPIC_PREFIX: &str = "vela/v1/devices";
const CLIENT_ID_PREFIX: &str = "vela-";
const PASSWORD_VERSION: &str = "v1";
const WAKE_SIGNAL_VERSION: u32 = 1;
const SERVICE_USERNAME: &str = "vela-server";
/// Domain-separation label for the service account's derived key. The service password is
/// HMAC(HMAC(secret, label), "v1:vela-server"), so it can never collide with a Device password,
/// which is HMAC(secret, "v1:<device id>").
const SERVICE_KEY_LABEL: &str = "vela-device-push/service-key";

/// Connection details a registered Device needs to reach the broker (the OTA `push` block).
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PushCredentials {
    pub endpoint: String,
    pub client_id: String,
    pub username: String,
    pub password: String,
    pub keepalive: u32,
    pub topic: String,
}

/// Who a broker question is about, once the channel's own rules are satisfied.
#[derive(Debug, PartialEq, Eq)]
pub enum BrokerClient<'a> {
    /// The server's own account: allowed as is.
    Service,
    /// A Device, by Device ID: allowed only while Device Admission still reports it registered.
    Device(&'a str),
}

pub struct DevicePush {
    endpoint: String,
    broker_url: String,
    keepalive: u32,
    key: hmac::Key,
    service_key: hmac::Key,
    publisher: OnceLock<Arc<dyn WakePublisher>>,
}

impl DevicePush {
    /// `None` when the feature is disabled; callers then behave as if the module did not exist.
    pub fn from_config(config: &DevicePushConfig) -> Option<Self> {
        config.enabled.then(|| {
            let key = hmac::Key::new(hmac::HMAC_SHA256, config.secret.expose().as_bytes());
            let service_key = hmac::Key::new(
                hmac::HMAC_SHA256,
                hmac::sign(&key, SERVICE_KEY_LABEL.as_bytes()).as_ref(),
            );
            Self {
                endpoint: config.endpoint.clone(),
                broker_url: config.broker_url.clone(),
                keepalive: config.keepalive_seconds,
                key,
                service_key,
                publisher: OnceLock::new(),
            }
        })
    }

    /// Like [`Self::from_config`], publishing through `publisher` instead of a broker (tests).
    pub fn with_publisher(
        config: &DevicePushConfig,
        publisher: Arc<dyn WakePublisher>,
    ) -> Option<Self> {
        let push = Self::from_config(config)?;
        let _ = push.publisher.set(publisher);
        Some(push)
    }

    /// Starts the server's own broker client with the service account. It reconnects on its own
    /// until `stopping` is cancelled. Fails when the broker address is unusable so startup can
    /// refuse to run half-enabled; an unreachable broker is not a failure.
    pub fn connect_broker(&self, stopping: CancellationToken) -> Result<(), WakePublishError> {
        if self.publisher.get().is_some() {
            return Ok(());
        }
        let client = mqtt::MqttPublisher::start(
            mqtt::BrokerLogin {
                url: &self.broker_url,
                username: SERVICE_USERNAME,
                password: &self.service_password(),
                keepalive_seconds: self.keepalive,
            },
            stopping,
        )?;
        let _ = self.publisher.set(Arc::new(client));
        Ok(())
    }

    /// Connection details for a registered Device. `None` for the one Device ID that would
    /// collide with the service account.
    pub fn credentials(&self, device_id: &str) -> Option<PushCredentials> {
        (device_id != SERVICE_USERNAME).then(|| PushCredentials {
            endpoint: self.endpoint.clone(),
            client_id: format!("{CLIENT_ID_PREFIX}{}", device_key(device_id)),
            username: device_id.to_owned(),
            password: self.device_password(device_id),
            keepalive: self.keepalive,
            topic: Self::signal_topic(device_id),
        })
    }

    /// Publishes a Wake Signal (QoS 1, retained) to the Device. Returns once the broker
    /// acknowledged it, or with the reason it did not.
    pub async fn send_wake_signal(
        &self,
        device_id: &str,
        wake: WakeRequest<'_>,
    ) -> Result<(), WakePublishError> {
        let Some(publisher) = self.publisher.get() else {
            return Err(WakePublishError("broker client not started".into()));
        };
        let payload = serde_json::to_vec(&WakeSignal {
            v: WAKE_SIGNAL_VERSION,
            kind: "wake",
            id: wake.reminder_id,
            at: wake.trigger_at_ms,
            expires_at: wake.expires_at_ms,
        })
        .expect("wake signal serializes");
        publisher
            .publish(&Self::signal_topic(device_id), payload)
            .await
    }

    /// Broker question "may this client connect?". The password is checked in constant time.
    pub fn authenticate<'a>(&self, username: &'a str, password: &str) -> Option<BrokerClient<'a>> {
        let (key, client) = if username == SERVICE_USERNAME {
            (&self.service_key, BrokerClient::Service)
        } else {
            (&self.key, BrokerClient::Device(username))
        };
        let presented = unhex(password)?;
        hmac::verify(key, password_message(username).as_bytes(), &presented)
            .is_ok()
            .then_some(client)
    }

    /// Broker question "may this client use this topic?" (`access_code` is mosquitto's `acc`).
    /// A Device may only read/subscribe its own `signal` topic and can never publish; the
    /// service account may only publish to well-formed `signal` topics.
    pub fn authorize_topic<'a>(
        username: &'a str,
        topic: &str,
        access_code: i64,
    ) -> Option<BrokerClient<'a>> {
        let access = TopicAccess::from_code(access_code)?;
        if username == SERVICE_USERNAME {
            return (access == TopicAccess::Write && is_signal_topic(topic))
                .then_some(BrokerClient::Service);
        }
        (matches!(access, TopicAccess::Read | TopicAccess::Subscribe)
            && topic == Self::signal_topic(username))
        .then_some(BrokerClient::Device(username))
    }

    /// The one server→Device topic of a Device.
    pub fn signal_topic(device_id: &str) -> String {
        format!("{TOPIC_PREFIX}/{}/signal", device_key(device_id))
    }

    fn device_password(&self, device_id: &str) -> String {
        hex(hmac::sign(&self.key, password_message(device_id).as_bytes()).as_ref())
    }

    fn service_password(&self) -> String {
        hex(hmac::sign(
            &self.service_key,
            password_message(SERVICE_USERNAME).as_bytes(),
        )
        .as_ref())
    }
}

/// What a broker password signs: a version prefix and the username.
fn password_message(username: &str) -> String {
    format!("{PASSWORD_VERSION}:{username}")
}

/// What one Wake Signal is about.
#[derive(Clone, Copy, Debug)]
pub struct WakeRequest<'a> {
    pub reminder_id: &'a str,
    /// The due moment or retry checkpoint this signal stands for. Together with the Reminder
    /// it identifies the signal, so a Device can tell a new one from a retained copy of one
    /// it already handled.
    pub trigger_at_ms: i64,
    /// Last moment the Reminder can still be delivered.
    pub expires_at_ms: i64,
}

/// Wake Signal payload. Carries no Reminder content, only identifiers and an expiry.
#[derive(Serialize)]
struct WakeSignal<'a> {
    v: u32,
    #[serde(rename = "type")]
    kind: &'static str,
    id: &'a str,
    at: i64,
    expires_at: i64,
}

/// What the broker is asking to do with a topic (mosquitto ACL `acc` values).
#[derive(Clone, Copy, PartialEq, Eq)]
enum TopicAccess {
    Read,
    Write,
    ReadWrite,
    Subscribe,
}

impl TopicAccess {
    fn from_code(code: i64) -> Option<Self> {
        match code {
            1 => Some(Self::Read),
            2 => Some(Self::Write),
            3 => Some(Self::ReadWrite),
            4 => Some(Self::Subscribe),
            _ => None,
        }
    }
}

/// Device Key: first 32 hex characters of SHA-256(device_id), a topic-safe stand-in for any
/// Device ID.
fn device_key(device_id: &str) -> String {
    hex(&Sha256::digest(device_id.as_bytes()))[..32].to_owned()
}

fn is_signal_topic(topic: &str) -> bool {
    topic
        .strip_prefix(TOPIC_PREFIX)
        .and_then(|rest| rest.strip_prefix('/'))
        .and_then(|rest| rest.strip_suffix("/signal"))
        .is_some_and(|key| key.len() == 32 && key.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.is_ascii() || !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).ok())
        .collect()
}
