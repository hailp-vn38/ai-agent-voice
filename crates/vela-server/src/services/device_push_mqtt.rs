//! Real MQTT adapter for [`WakePublisher`]: the server's own broker client.

use super::device_push_wake::{WakePublishError, WakePublisher};
use async_trait::async_trait;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

/// An MQTT client that reconnects on its own and publishes QoS 1, retained.
pub struct MqttPublisher {
    client: rumqttc::AsyncClient,
    connected: Arc<AtomicBool>,
}

impl MqttPublisher {
    /// `broker_url` is `host:port` or `mqtt://host:port`. Spawns the connection task, which
    /// stops when `stopping` is cancelled.
    pub fn start(
        broker_url: &str,
        username: &str,
        password: &str,
        keepalive_seconds: u32,
        stopping: tokio_util::sync::CancellationToken,
    ) -> Result<Self, WakePublishError> {
        let trimmed = broker_url
            .trim_start_matches("mqtt://")
            .trim_end_matches('/');
        let (host, port) = trimmed
            .rsplit_once(':')
            .and_then(|(host, port)| Some((host, port.parse::<u16>().ok()?)))
            .ok_or_else(|| WakePublishError(format!("invalid broker_url `{broker_url}`")))?;
        let mut options = rumqttc::MqttOptions::new(username, host, port);
        options.set_credentials(username, password);
        options.set_keep_alive(Duration::from_secs(keepalive_seconds.into()));
        let (client, mut eventloop) = rumqttc::AsyncClient::new(options, 64);
        let connected = Arc::new(AtomicBool::new(false));
        let flag = connected.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stopping.cancelled() => return,
                    event = eventloop.poll() => match event {
                        Ok(rumqttc::Event::Incoming(rumqttc::Packet::ConnAck(_))) => {
                            flag.store(true, Ordering::SeqCst);
                        }
                        Ok(_) => {}
                        Err(err) => {
                            if flag.swap(false, Ordering::SeqCst) {
                                tracing::warn!(error = %err, "device push broker connection lost; reconnecting");
                            }
                            tokio::time::sleep(Duration::from_secs(2)).await;
                        }
                    }
                }
            }
        });
        Ok(Self { client, connected })
    }
}

#[async_trait]
impl WakePublisher for MqttPublisher {
    async fn publish(&self, topic: &str, payload: Vec<u8>) -> Result<(), WakePublishError> {
        if !self.connected.load(Ordering::SeqCst) {
            return Err(WakePublishError("broker not connected".into()));
        }
        self.client
            .try_publish(topic, rumqttc::QoS::AtLeastOnce, true, payload)
            .map_err(|err| WakePublishError(err.to_string()))
    }
}
