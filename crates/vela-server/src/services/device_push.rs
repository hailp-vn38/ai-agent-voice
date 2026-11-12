//! Device Push Channel conventions: Device Key, topics, client ID and HMAC credentials.
//!
//! Every naming and credential rule of the channel lives here so OTA, the scheduler and broker
//! authorization cannot drift apart. The module exists only when `[device_push]` is enabled.

use crate::config::DevicePushConfig;
use ring::hmac;
use serde::Serialize;
use sha2::{Digest, Sha256};

const TOPIC_PREFIX: &str = "vela/v1/devices";
const CLIENT_ID_PREFIX: &str = "vela-";
const PASSWORD_VERSION: &str = "v1";

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

pub struct DevicePush {
    endpoint: String,
    keepalive: u32,
    key: hmac::Key,
    service_key: hmac::Key,
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
                keepalive: config.keepalive_seconds,
                key,
                service_key,
            }
        })
    }

    /// First 32 hex characters of SHA-256(device_id): a topic-safe stand-in for any Device ID.
    pub fn device_key(device_id: &str) -> String {
        hex(&Sha256::digest(device_id.as_bytes()))[..32].to_owned()
    }

    pub fn signal_topic(device_id: &str) -> String {
        format!("{TOPIC_PREFIX}/{}/signal", Self::device_key(device_id))
    }

    pub fn client_id(device_id: &str) -> String {
        format!("{CLIENT_ID_PREFIX}{}", Self::device_key(device_id))
    }

    pub fn password(&self, device_id: &str) -> String {
        let message = format!("{PASSWORD_VERSION}:{device_id}");
        hex(hmac::sign(&self.key, message.as_bytes()).as_ref())
    }

    pub fn credentials(&self, device_id: &str) -> PushCredentials {
        PushCredentials {
            endpoint: self.endpoint.clone(),
            client_id: Self::client_id(device_id),
            username: device_id.to_owned(),
            password: self.password(device_id),
            keepalive: self.keepalive,
            topic: Self::signal_topic(device_id),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

// Broker authorization decisions (ticket 02), kept additive below the credential helpers.

const SERVICE_USERNAME: &str = "vela-server";
/// Domain-separation label for the service account's derived key. The service password is
/// HMAC(HMAC(secret, label), "v1:vela-server"), so it can never collide with a Device password,
/// which is HMAC(secret, "v1:<device id>").
const SERVICE_KEY_LABEL: &str = "vela-device-push/service-key";

/// What the broker is asking to do with a topic (mosquitto ACL `acc` values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopicAccess {
    Read,
    Write,
    ReadWrite,
    Subscribe,
}

impl TopicAccess {
    pub fn from_code(code: i64) -> Option<Self> {
        match code {
            1 => Some(Self::Read),
            2 => Some(Self::Write),
            3 => Some(Self::ReadWrite),
            4 => Some(Self::Subscribe),
            _ => None,
        }
    }
}

impl DevicePush {
    pub fn is_service_username(username: &str) -> bool {
        username == SERVICE_USERNAME
    }

    /// Starts the server's own broker client (the Wake Signal publisher) with the service
    /// account. Fails when `broker_url` is unusable so startup can refuse to run half-enabled.
    pub fn start_wake_publisher(
        &self,
        broker_url: &str,
        keepalive_seconds: u32,
        stopping: tokio_util::sync::CancellationToken,
    ) -> Result<super::device_push_mqtt::MqttPublisher, super::device_push_wake::WakePublishError>
    {
        super::device_push_mqtt::MqttPublisher::start(
            broker_url,
            SERVICE_USERNAME,
            &self.service_password(),
            keepalive_seconds,
            stopping,
        )
    }

    pub fn service_username() -> &'static str {
        SERVICE_USERNAME
    }

    pub fn service_password(&self) -> String {
        let message = format!("{PASSWORD_VERSION}:{SERVICE_USERNAME}");
        hex(hmac::sign(&self.service_key, message.as_bytes()).as_ref())
    }

    /// Constant-time check of a Device password. Admission is checked separately by the caller.
    pub fn verify_device_password(&self, device_id: &str, presented: &str) -> bool {
        ring::constant_time::verify_slices_are_equal(
            self.password(device_id).as_bytes(),
            presented.as_bytes(),
        )
        .is_ok()
    }

    pub fn verify_service_password(&self, presented: &str) -> bool {
        ring::constant_time::verify_slices_are_equal(
            self.service_password().as_bytes(),
            presented.as_bytes(),
        )
        .is_ok()
    }

    /// A Device may only read/subscribe its own `signal` topic; it can never publish.
    pub fn device_topic_allowed(device_id: &str, topic: &str, access: TopicAccess) -> bool {
        matches!(access, TopicAccess::Read | TopicAccess::Subscribe)
            && topic == Self::signal_topic(device_id)
    }

    /// The service account may only publish to a well-formed `signal` topic.
    pub fn service_topic_allowed(topic: &str, access: TopicAccess) -> bool {
        if access != TopicAccess::Write {
            return false;
        }
        let Some(key) = topic
            .strip_prefix(TOPIC_PREFIX)
            .and_then(|rest| rest.strip_prefix('/'))
            .and_then(|rest| rest.strip_suffix("/signal"))
        else {
            return false;
        };
        key.len() == 32 && key.bytes().all(|b| b.is_ascii_hexdigit())
    }
}
