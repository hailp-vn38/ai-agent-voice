//! Real MQTT adapter for [`WakePublisher`]: the server's own broker client.

use super::publisher::{WakePublishError, WakePublisher};
use async_trait::async_trait;
use rumqttc::{Event, Outgoing, Packet};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

const RECONNECT_DELAY: Duration = Duration::from_secs(2);
/// How long a publish waits for the broker's PUBACK before it counts as failed.
const ACK_TIMEOUT: Duration = Duration::from_secs(5);

/// How the server's own client reaches and logs in to the broker.
pub(super) struct BrokerLogin<'a> {
    /// `host:port` or `mqtt://host:port`.
    pub url: &'a str,
    pub username: &'a str,
    pub password: &'a str,
    pub keepalive_seconds: u32,
}

type AckSender = oneshot::Sender<Result<(), WakePublishError>>;

/// Publishes waiting for the broker. The session is clean, so nothing unacknowledged survives
/// a reconnect: a lost connection fails every waiter instead of leaving it to time out.
#[derive(Default)]
struct Acks {
    connected: bool,
    /// Handed to the client, not yet written to the socket (in request order).
    queued: VecDeque<AckSender>,
    /// Written with this packet id, awaiting PUBACK.
    in_flight: HashMap<u16, AckSender>,
}

impl Acks {
    fn fail_all(&mut self, reason: &str) {
        for waiter in self
            .queued
            .drain(..)
            .chain(self.in_flight.drain().map(|(_, w)| w))
        {
            let _ = waiter.send(Err(WakePublishError(reason.to_owned())));
        }
    }
}

/// An MQTT client that reconnects on its own and publishes QoS 1, retained.
pub(super) struct MqttPublisher {
    client: rumqttc::AsyncClient,
    acks: Arc<Mutex<Acks>>,
}

impl MqttPublisher {
    /// Spawns the connection task, which stops when `stopping` is cancelled.
    pub(super) fn start(
        login: BrokerLogin<'_>,
        stopping: CancellationToken,
    ) -> Result<Self, WakePublishError> {
        let trimmed = login
            .url
            .trim_start_matches("mqtt://")
            .trim_end_matches('/');
        let (host, port) = trimmed
            .rsplit_once(':')
            .and_then(|(host, port)| Some((host, port.parse::<u16>().ok()?)))
            .ok_or_else(|| WakePublishError(format!("invalid broker_url `{}`", login.url)))?;
        let mut options = rumqttc::MqttOptions::new(login.username, host, port);
        options.set_credentials(login.username, login.password);
        options.set_keep_alive(Duration::from_secs(login.keepalive_seconds.into()));
        let (client, mut eventloop) = rumqttc::AsyncClient::new(options, 64);
        let acks = Arc::new(Mutex::new(Acks::default()));
        let shared = Arc::clone(&acks);
        tokio::spawn(async move {
            loop {
                let event = tokio::select! {
                    _ = stopping.cancelled() => return,
                    event = eventloop.poll() => event,
                };
                match event {
                    Ok(Event::Incoming(Packet::ConnAck(_))) => {
                        shared.lock().unwrap().connected = true;
                    }
                    Ok(Event::Outgoing(Outgoing::Publish(packet_id))) => {
                        let mut acks = shared.lock().unwrap();
                        if let Some(waiter) = acks.queued.pop_front() {
                            acks.in_flight.insert(packet_id, waiter);
                        }
                    }
                    Ok(Event::Incoming(Packet::PubAck(ack))) => {
                        if let Some(waiter) = shared.lock().unwrap().in_flight.remove(&ack.pkid) {
                            let _ = waiter.send(Ok(()));
                        }
                    }
                    Ok(_) => {}
                    Err(err) => {
                        let was_connected = {
                            let mut acks = shared.lock().unwrap();
                            acks.fail_all("broker connection lost before acknowledgement");
                            std::mem::replace(&mut acks.connected, false)
                        };
                        if was_connected {
                            tracing::warn!(error = %err, "device push broker connection lost; reconnecting");
                        }
                        tokio::select! {
                            _ = stopping.cancelled() => return,
                            _ = tokio::time::sleep(RECONNECT_DELAY) => {}
                        }
                    }
                }
            }
        });
        Ok(Self { client, acks })
    }
}

#[async_trait]
impl WakePublisher for MqttPublisher {
    async fn publish(&self, topic: &str, payload: Vec<u8>) -> Result<(), WakePublishError> {
        let (sender, acknowledged) = oneshot::channel();
        {
            // Queue the waiter and the request under one lock so their orders match.
            let mut acks = self.acks.lock().unwrap();
            if !acks.connected {
                return Err(WakePublishError("broker not connected".into()));
            }
            self.client
                .try_publish(topic, rumqttc::QoS::AtLeastOnce, true, payload)
                .map_err(|err| WakePublishError(err.to_string()))?;
            acks.queued.push_back(sender);
        }
        match tokio::time::timeout(ACK_TIMEOUT, acknowledged).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(WakePublishError("broker client stopped".into())),
            Err(_) => Err(WakePublishError(
                "broker did not acknowledge in time".into(),
            )),
        }
    }
}
