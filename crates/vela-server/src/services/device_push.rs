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
}

impl DevicePush {
    /// `None` when the feature is disabled; callers then behave as if the module did not exist.
    pub fn from_config(config: &DevicePushConfig) -> Option<Self> {
        config.enabled.then(|| Self {
            endpoint: config.endpoint.clone(),
            keepalive: config.keepalive_seconds,
            key: hmac::Key::new(hmac::HMAC_SHA256, config.secret.expose().as_bytes()),
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
