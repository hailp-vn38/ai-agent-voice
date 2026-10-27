//! The WebSocket Writer: the only task that calls send for a Voice Session's socket.
//!
//! It drains the bounded outbound lanes in priority `urgent > control > audio`, keeps the
//! connection alive with Pings, bounds every send by the write timeout and arbitrates the terminal
//! playback outcome of each turn before SessionActor commits delivery. The arbitration itself is
//! the I/O-free [`PlaybackArbiter`]; [`WebSocketWriter::run`] only performs what it decides.
use super::*;

/// The receiving ends of the outbound lanes the actor produces into.
pub(super) struct WriterLanes {
    pub(super) urgent: mpsc::Receiver<OutboundMessage>,
    pub(super) control: mpsc::Receiver<OutboundMessage>,
    pub(super) audio: mpsc::Receiver<OutboundMessage>,
    /// The application's stop signal; see the last branch of [`WebSocketWriter::run`].
    pub(super) shutdown: tokio::sync::watch::Receiver<bool>,
}

/// Connection timing reported once, when ServerHello has actually been sent.
pub(super) struct HelloTelemetry {
    pub(super) metrics: Option<Arc<RuntimeMetrics>>,
    pub(super) connect_started: Instant,
    pub(super) socket_started: Instant,
    pub(super) hello_queued: Instant,
}

pub(super) struct WebSocketWriter {
    pub(super) sender: futures_util::stream::SplitSink<WebSocket, Message>,
    pub(super) write_timeout: Duration,
    /// Interval between Pings; zero sends none.
    pub(super) ping_period: Duration,
    pub(super) generation_gate: Arc<crate::session::GenerationGate>,
    pub(super) pipeline_request: Arc<std::sync::atomic::AtomicU64>,
    pub(super) writer_events: mpsc::Sender<WriterEvent>,
    /// Test-only; see [`WriterOutcomeProbe`](crate::session::WriterOutcomeProbe).
    pub(super) probe: Option<Arc<dyn crate::session::WriterOutcomeProbe>>,
    pub(super) hello: HelloTelemetry,
    /// Set once the writer has stopped, whatever the reason.
    pub(super) done: tokio::sync::watch::Sender<bool>,
}

impl WebSocketWriter {
    pub(super) async fn run(mut self, mut lanes: WriterLanes) {
        // Server Pings keep NAT/proxy state alive and give a responsive peer something to answer,
        // which is what the reader's Idle Close measures.
        let pings_enabled = !self.ping_period.is_zero();
        let period = self.ping_period.max(Duration::from_millis(1));
        let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut arbiter = PlaybackArbiter::default();
        let mut hello_pending = true;
        loop {
            tokio::select! {
                biased;
                Some(message) = lanes.urgent.recv() => {
                    if self.on_urgent(&mut arbiter, message).await.is_break() {
                        break;
                    }
                },
                // A normal stop follows the final paced packet, but only after the urgent
                // lane has had its chance to preempt it.
                _ = std::future::ready(()), if arbiter.has_deferred_finish() && lanes.audio.is_empty() => {
                    let (turn_id, text) = arbiter.take_deferred_finish().expect("checked above");
                    if self.send_stop(&mut arbiter, turn_id, text).await.is_break() {
                        break;
                    }
                },
                Some(message) = lanes.control.recv() => {
                    if self.on_control(&mut arbiter, &mut hello_pending, message, lanes.audio.is_empty()).await.is_break() {
                        break;
                    }
                },
                _ = ping.tick(), if pings_enabled => {
                    if let Err(error) = send_frame(&mut self.sender, self.write_timeout, Message::Ping(Default::default())).await {
                        warn!(event = "ws_writer_failed", outcome = "failure", %error, "websocket ping failed");
                        break;
                    }
                },
                Some(message) = lanes.audio.recv() => {
                    if let OutboundMessage::Binary { turn_id, .. } = &message
                        && arbiter.plays(*turn_id)
                        && self.send(message).await
                    {
                        break;
                    }
                },
                // The application's stop signal for this writer is deliberately the *last* branch,
                // and `biased` makes that deterministic: every lane above is served first, so a
                // terminal control message the actor already queued — `Close(1001)` on a controlled
                // close — still reaches the client before the loop ends. Stopping here first would
                // discard it.
                //
                // It is also the only branch that can stay pending once every lane has closed,
                // because it never completes while any sender for the watch channel lives — and
                // until it completes, `select!` keeps the whole loop pending no matter how many
                // lanes have closed. That is why the sender is released explicitly during teardown
                // rather than left to the end of the socket task: a writer waiting on it is a
                // session the drain can never observe finishing.
                changed = lanes.shutdown.changed() => {
                    if changed.is_err() || *lanes.shutdown.borrow() {
                        break;
                    }
                },
                else => {
                    if let Some((turn_id, text)) = arbiter.take_deferred_finish() {
                        let _ = self.send_stop(&mut arbiter, turn_id, text).await;
                    }
                    break;
                },
            }
        }
        let _ = self.done.send(true);
    }

    async fn on_urgent(
        &mut self,
        arbiter: &mut PlaybackArbiter,
        message: OutboundMessage,
    ) -> std::ops::ControlFlow<()> {
        let OutboundMessage::AbortTurn { turn_id, text } = message else {
            return flow(self.send(message).await);
        };
        let start_was_sent = match arbiter.abort(turn_id) {
            AbortStep::Active { start_was_sent } => start_was_sent,
            // Begin may still be queued behind this urgent command. The actor needs a terminal
            // outcome so it can release this turn's permit; a later Begin for the same invalid
            // generation is rejected at the gate.
            AbortStep::NotActive => false,
        };
        let stop_was_sent = if start_was_sent {
            if self.send(OutboundMessage::Text(text)).await {
                self.report_failed(turn_id).await;
                return std::ops::ControlFlow::Break(());
            }
            true
        } else {
            false
        };
        self.report_closed(
            turn_id,
            WriterTurnOutcome::Aborted {
                start_was_sent,
                stop_was_sent,
            },
        )
        .await;
        std::ops::ControlFlow::Continue(())
    }

    async fn on_control(
        &mut self,
        arbiter: &mut PlaybackArbiter,
        hello_pending: &mut bool,
        message: OutboundMessage,
        audio_drained: bool,
    ) -> std::ops::ControlFlow<()> {
        match message {
            OutboundMessage::BeginTurn {
                generation,
                turn_id,
                text,
            } => {
                if arbiter.admits_begin(self.generation_gate.admits(generation)) {
                    if self.send(OutboundMessage::Text(text)).await {
                        self.report_failed(turn_id).await;
                        return std::ops::ControlFlow::Break(());
                    }
                    arbiter.started(turn_id);
                }
                std::ops::ControlFlow::Continue(())
            }
            OutboundMessage::FinishTurn { turn_id, text } => {
                match arbiter.finish(turn_id, text, audio_drained) {
                    FinishStep::AbortUnstarted => {
                        self.report_closed(
                            turn_id,
                            WriterTurnOutcome::Aborted {
                                start_was_sent: false,
                                stop_was_sent: false,
                            },
                        )
                        .await;
                        std::ops::ControlFlow::Continue(())
                    }
                    FinishStep::Deferred => std::ops::ControlFlow::Continue(()),
                    FinishStep::SendStop(text) => self.send_stop(arbiter, turn_id, text).await,
                }
            }
            message => {
                let sends_hello = *hello_pending && matches!(&message, OutboundMessage::Text(_));
                if self.send(message).await {
                    return std::ops::ControlFlow::Break(());
                }
                if sends_hello {
                    *hello_pending = false;
                    self.report_connected();
                }
                std::ops::ControlFlow::Continue(())
            }
        }
    }

    /// Sends the active turn's normal stop and reports its Normal outcome.
    async fn send_stop(
        &mut self,
        arbiter: &mut PlaybackArbiter,
        turn_id: crate::session::TurnId,
        text: String,
    ) -> std::ops::ControlFlow<()> {
        if self.send(OutboundMessage::Text(text)).await {
            self.report_failed(turn_id).await;
            return std::ops::ControlFlow::Break(());
        }
        arbiter.stopped();
        self.report_closed(turn_id, WriterTurnOutcome::Normal).await;
        std::ops::ControlFlow::Continue(())
    }

    /// Returns `true` when the writer must stop.
    async fn send(&mut self, message: OutboundMessage) -> bool {
        send_outbound(
            &mut self.sender,
            self.write_timeout,
            message,
            &self.generation_gate,
            &self.pipeline_request,
        )
        .await
    }

    async fn report_failed(&self, turn_id: crate::session::TurnId) {
        let _ = self
            .writer_events
            .send(WriterEvent::Failed {
                turn_id: Some(turn_id),
            })
            .await;
    }

    /// Reports a turn's terminal outcome to the actor, giving a test harness its boundary probe.
    /// Every `TurnClosed` the writer produces goes through here so no boundary can bypass the seam.
    async fn report_closed(&self, turn_id: crate::session::TurnId, outcome: WriterTurnOutcome) {
        if let Some(probe) = &self.probe {
            probe
                .before_terminal_outcome(turn_id, outcome.clone())
                .await;
        }
        let _ = self
            .writer_events
            .send(WriterEvent::TurnClosed {
                turn_id,
                outcome: outcome.clone(),
            })
            .await;
        if let Some(probe) = &self.probe {
            probe
                .after_terminal_outcome_reported(turn_id, outcome)
                .await;
        }
    }

    fn report_connected(&self) {
        let hello = &self.hello;
        if let Some(metrics) = &hello.metrics {
            metrics.observe(RuntimePhase::WsHelloSend, hello.hello_queued.elapsed());
            metrics.observe(RuntimePhase::WsConnect, hello.connect_started.elapsed());
        }
        info!(
            event = "ws_connected",
            outcome = "success",
            elapsed_ms = hello.connect_started.elapsed().as_millis() as u64,
            pre_upgrade_ms = hello
                .socket_started
                .duration_since(hello.connect_started)
                .as_millis() as u64,
            after_upgrade_ms = hello.socket_started.elapsed().as_millis() as u64,
            "ServerHello sent; voice session connected"
        );
    }
}

fn flow(stop: bool) -> std::ops::ControlFlow<()> {
    if stop {
        std::ops::ControlFlow::Break(())
    } else {
        std::ops::ControlFlow::Continue(())
    }
}

/// The turn the writer is currently playing.
struct PlaybackTurn {
    id: crate::session::TurnId,
    start_was_sent: bool,
}

/// I/O-free playback arbitration. Its one invariant: every turn-scoped command the actor sends
/// ends in exactly one terminal outcome the actor can act on, because the actor holds the turn's
/// delivery and Active Turn capacity until it hears one. A duplicate for a turn already closed is
/// harmless: Turn IDs never repeat and the actor ignores outcomes for other turns.
#[derive(Default)]
struct PlaybackArbiter {
    active: Option<PlaybackTurn>,
    /// The active turn's normal stop, waiting for its paced audio to drain.
    deferred_finish: Option<(crate::session::TurnId, String)>,
}

enum FinishStep {
    /// The writer never admitted this turn's Begin (stale generation or another turn still
    /// playing): it closes now as aborted with nothing sent.
    AbortUnstarted,
    /// The active turn still has paced audio queued; its stop waits for the audio lane.
    Deferred,
    /// The active turn's stop may be sent now.
    SendStop(String),
}

enum AbortStep {
    Active { start_was_sent: bool },
    NotActive,
}

impl PlaybackArbiter {
    fn admits_begin(&self, generation_admitted: bool) -> bool {
        self.active.is_none() && generation_admitted
    }

    fn started(&mut self, turn_id: crate::session::TurnId) {
        self.active = Some(PlaybackTurn {
            id: turn_id,
            start_was_sent: true,
        });
    }

    fn is_active(&self, turn_id: crate::session::TurnId) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.id == turn_id)
    }

    fn plays(&self, turn_id: crate::session::TurnId) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.start_was_sent && active.id == turn_id)
    }

    fn finish(
        &mut self,
        turn_id: crate::session::TurnId,
        text: String,
        audio_drained: bool,
    ) -> FinishStep {
        if !self.is_active(turn_id) {
            FinishStep::AbortUnstarted
        } else if !audio_drained {
            self.deferred_finish = Some((turn_id, text));
            FinishStep::Deferred
        } else {
            FinishStep::SendStop(text)
        }
    }

    fn has_deferred_finish(&self) -> bool {
        self.deferred_finish.is_some()
    }

    fn take_deferred_finish(&mut self) -> Option<(crate::session::TurnId, String)> {
        self.deferred_finish.take()
    }

    /// The active turn's stop was sent.
    fn stopped(&mut self) {
        self.active = None;
    }

    fn abort(&mut self, turn_id: crate::session::TurnId) -> AbortStep {
        if !self.is_active(turn_id) {
            return AbortStep::NotActive;
        }
        self.deferred_finish = None;
        let start_was_sent = self
            .active
            .take()
            .is_some_and(|active| active.start_was_sent);
        AbortStep::Active { start_was_sent }
    }
}
