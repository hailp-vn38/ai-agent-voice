//! Session-local builtin actions, including next-turn Template activation.

use super::*;

use crate::{
    providers::ResolvedAgentRuntimes,
    tools::builtin::{BuiltinTool, EXIT_TOOL_NAME, parse_exit_args, parse_switch_template_args},
};

impl SessionActor {
    pub(in super::super) fn execute_builtin_tool(&mut self, call: ToolCall, tool: BuiltinTool) {
        match tool {
            BuiltinTool::EndConversation => {
                let Ok(args) = parse_exit_args(&call.arguments) else {
                    self.complete_tool_call(call, Err("invalid_arguments"));
                    return;
                };
                let Some(turn_id) = self.current_turn_id() else {
                    self.complete_tool_call(call, Err("no_active_turn"));
                    return;
                };
                tracing::info!(
                    event = "builtin_tool_called",
                    tool = EXIT_TOOL_NAME,
                    turn_id = turn_id.get(),
                    "Builtin conversation exit tool called"
                );
                self.pending_actions.close_after_turn = Some(turn_id);
                tracing::info!(
                    event = "session_close_after_turn_armed",
                    turn_id = turn_id.get(),
                    "Session will close after normal turn completion"
                );
                self.complete_builtin_tool_call(
                    call,
                    serde_json::json!({
                        "ok": true,
                        "code": null,
                        "content": "conversation_exit_requested",
                        "truncated": false,
                    })
                    .to_string(),
                    args.say_goodbye,
                );
            }
            BuiltinTool::SwitchTemplate => self.execute_template_switch(call),
        }
    }

    pub(in super::super) fn execute_template_switch(&mut self, call: ToolCall) {
        let Ok(args) = parse_switch_template_args(&call.arguments) else {
            self.complete_tool_call(call, Err("invalid_arguments"));
            return;
        };
        let Some(turn_id) = self.current_turn_id() else {
            self.complete_tool_call(call, Err("no_active_turn"));
            return;
        };
        if let Err(reject) = self.authorize_speaker_switch(&args.template) {
            tracing::warn!(
                event = "session_profile_switch_rejected",
                reason = reject.reason(),
                template_key = %args.template,
                "A template switch would not keep the locked speaker gated"
            );
            self.complete_tool_call(call, Err(reject.reason()));
            return;
        }
        if let Some(configuration) = self
            .switch_catalog
            .cold
            .iter()
            .find(|candidate| candidate.template_key() == args.template)
            .cloned()
        {
            let Some(manager) = self.switch_catalog.manager.clone() else {
                self.complete_tool_call(call, Err("template_switch_unavailable"));
                return;
            };
            if self.template_prepare.is_some() || self.managed_switch_boundary.is_some() {
                self.complete_tool_call(call, Err("provider_runtime_busy"));
                return;
            }
            let Ok(runtime) = tokio::runtime::Handle::try_current() else {
                self.complete_tool_call(call, Err("template_switch_unavailable"));
                return;
            };
            let cancellation = self
                .turn
                .as_ref()
                .expect("active turn checked above")
                .cancellation
                .clone();
            let (sender, completion) = tokio::sync::oneshot::channel();
            let deadline = manager.admission_deadline();
            runtime.spawn(async move {
                let _timer = manager
                    .metrics()
                    .timer(crate::services::provider_runtime::RuntimePhase::SwitchPrepare);
                let result = tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => return,
                    result = configuration.prepare(&manager, deadline) => result,
                };
                let _ = sender.send(result);
            });
            self.template_prepare = Some(TemplatePreparation {
                turn_id,
                generation: self.generation,
                profile_revision: self.profile.revision,
                call,
                completion,
            });
            return;
        }
        if self.switch_catalog.find(&args.template).is_none() {
            tracing::warn!(
                event = "session_profile_switch_rejected",
                reason = "candidate_not_admitted",
                "A template switch named a candidate this session never admitted"
            );
            self.complete_tool_call(call, Err("template_switch_unavailable"));
            return;
        }
        tracing::info!(
            event = "builtin_tool_called",
            tool = crate::tools::builtin::SWITCH_TEMPLATE_TOOL_NAME,
            template_key = %args.template,
            turn_id = turn_id.get(),
            "Builtin template switch tool called"
        );
        self.pending_actions.switch_template_after_turn = Some(PendingTemplateSwitch {
            turn_id,
            template_key: args.template.clone(),
            prepared: None,
        });
        self.complete_tool_call(
            call,
            Ok(serde_json::json!({
                "ok": true,
                "code": null,
                "content": "template_switch_scheduled",
                "truncated": false,
            })),
        );
    }

    pub(in super::super) fn drain_template_preparation(&mut self) {
        let current_turn_id = self.current_turn_id();
        let Some(pending) = self.template_prepare.as_mut() else {
            return;
        };
        if current_turn_id != Some(pending.turn_id)
            || self.generation != pending.generation
            || self.profile.revision != pending.profile_revision
        {
            self.template_prepare = None;
            return;
        }
        let result = match pending.completion.try_recv() {
            Ok(result) => result,
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => return,
            Err(_) => Err(crate::services::provider_runtime::RuntimeError::Unavailable),
        };
        if result.is_err()
            && let Some(manager) = &self.switch_catalog.manager
        {
            manager
                .metrics()
                .increment(crate::services::provider_runtime::RuntimeCounter::SwitchFailure);
        }
        let pending = self
            .template_prepare
            .take()
            .expect("one pending preparation");
        match result {
            Ok(prepared) => {
                self.pending_actions.switch_template_after_turn = Some(PendingTemplateSwitch {
                    turn_id: pending.turn_id,
                    template_key: prepared.configuration.template_key().to_owned(),
                    prepared: Some(prepared),
                });
                self.complete_tool_call(pending.call, Ok(serde_json::json!({"ok":true,"code":null,"content":"template_switch_scheduled","truncated":false})));
            }
            Err(error) => self.complete_tool_call(
                pending.call,
                Err(match error {
                    crate::services::provider_runtime::RuntimeError::Busy => {
                        "provider_runtime_busy"
                    }
                    crate::services::provider_runtime::RuntimeError::MemoryPressure => {
                        "provider_runtime_memory_pressure"
                    }
                    crate::services::provider_runtime::RuntimeError::Timeout => {
                        "provider_runtime_timeout"
                    }
                    _ => "template_switch_unavailable",
                }),
            ),
        }
    }

    pub(in super::super) fn begin_managed_switch_boundary(
        &mut self,
        prepared: crate::session::PreparedTemplateProfile,
    ) {
        // Keep the original runtime routes and leases until their exact cleanup events arrive.
        self.cancel_asr();
        self.close_vad();
        self.speech_output.release();
        self.managed_switch_started = Some(Instant::now());
        self.managed_switch_boundary = Some(prepared);
        self.drain_managed_switch_boundary();
    }

    pub(in super::super) fn drain_managed_switch_boundary(&mut self) {
        if self.managed_switch_boundary.is_none()
            || !self.asr_cleanup_pending.is_empty()
            || self.asr_stream.is_some()
            || self.vad_session.is_some()
        {
            return;
        }
        let prepared = self
            .managed_switch_boundary
            .take()
            .expect("prepared boundary checked");
        let template_key = prepared.configuration.template_key().to_owned();
        if let Err(reject) = self.authorize_speaker_switch(&template_key) {
            tracing::warn!(
                event = "session_profile_switch_dropped",
                reason = reject.reason(),
                %template_key,
                "A prepared template switch would not keep the locked speaker gated"
            );
            self.complete_recognition();
            return;
        }
        if install_candidate_runtimes(self, &prepared.runtimes).is_err() {
            self.complete_recognition();
            return;
        }
        if let (Some(started), Some(manager)) = (
            self.managed_switch_started.take(),
            self.switch_catalog.manager.as_ref(),
        ) {
            manager.metrics().observe(
                crate::services::provider_runtime::RuntimePhase::SwitchCommit,
                started.elapsed(),
            );
        }
        let configuration = prepared.configuration;
        self.profile.source = configuration.source;
        self.profile.language = configuration.language;
        self.profile.system_prompt = configuration.system_prompt;
        self.profile.providers = configuration.providers;
        self.profile.revision = self.profile.revision.saturating_add(1);
        self.switch_catalog.active_leases = prepared.leases;
        self.install_switch_speaker(&template_key);
        self.complete_recognition();
        self.replay_switch_ingress();
    }

    /// Ticket 17: the SessionActor keeps no database handle, so the target Template's authority
    /// is read from the guard admission installed. `None` means the session is speaker-free and
    /// membership is the whole switch rule.
    pub(in super::super) fn authorize_speaker_switch(
        &self,
        template_key: &str,
    ) -> Result<
        Option<std::sync::Arc<crate::session::SwitchSpeakerAuthority>>,
        crate::session::SwitchReject,
    > {
        let Some(guard) = &self.speaker_switch else {
            return Ok(None);
        };
        let locked = self.speaker_gate.as_ref().and_then(SpeakerGate::locked);
        guard.authorize(template_key, locked).map(Some)
    }

    /// Ticket 17: install the target Template's frozen Observe plan so the next turn scores the
    /// locked Speaker anew on it. The runtime and lease are unchanged because the authority is
    /// only accepted in the session's own embedding space.
    fn install_switch_speaker(&mut self, template_key: &str) {
        let (Some(guard), Some(observe)) =
            (self.speaker_switch.as_ref(), self.speaker_observe.as_ref())
        else {
            return;
        };
        let Some(authority) = guard.authorities().get(template_key) else {
            return;
        };
        if authority.embedding_space.as_ref() != observe.embedding_space() {
            return;
        }
        self.speaker_observe = Some(std::sync::Arc::new(
            observe.retargeted(authority.plan.clone()),
        ));
    }

    /// The only place a Template change becomes effective.
    pub(in super::super) fn apply_template_switch(&mut self, template_key: &str) {
        let Some(candidate) = self.switch_catalog.find(template_key).cloned() else {
            tracing::warn!(
                event = "session_profile_switch_dropped",
                reason = "candidate_not_admitted",
                "A scheduled template switch no longer names an admitted candidate"
            );
            return;
        };
        if let Err(reject) = self.authorize_speaker_switch(template_key) {
            tracing::warn!(
                event = "session_profile_switch_dropped",
                reason = reject.reason(),
                template_key,
                "A scheduled template switch would not keep the locked speaker gated"
            );
            return;
        }
        if let Err(error) = install_candidate_runtimes(self, &candidate.runtimes) {
            tracing::warn!(
                event = "session_profile_switch_failed",
                reason = "runtime_install_failed",
                %error,
                "The admitted candidate's already-loaded runtime could not be installed"
            );
            return;
        }
        let previous_revision = self.profile.revision;
        self.profile.switched_to(&candidate);
        self.install_switch_speaker(template_key);
        tracing::info!(
            event = "session_profile_switched",
            template_key = %candidate.template_key,
            previous_revision,
            revision = self.profile.revision,
            "Session profile switched at a turn boundary"
        );
    }
}

fn install_candidate_runtimes(
    actor: &mut SessionActor,
    runtimes: &ResolvedAgentRuntimes,
) -> Result<(), crate::audio::AudioError> {
    let speech_output = SpeechOutput::with_worker(
        runtimes.tts.provider(),
        std::sync::Arc::clone(&runtimes.tts),
        actor.speech_output_config.clone(),
    )?;

    if !std::sync::Arc::ptr_eq(&actor.vad_runtime, &runtimes.vad) {
        actor.close_vad();
        actor.vad_session = None;
        actor.vad_cycle = None;
        actor.pending_vad_cycle = None;
        actor.vad_runtime.unregister_session(&actor.session_id);
        actor.vad_runtime = std::sync::Arc::clone(&runtimes.vad);
        actor.vad_events = actor.vad_runtime.register_session(&actor.session_id);
    }
    if !std::sync::Arc::ptr_eq(&actor.asr_runtime, &runtimes.asr) {
        actor.cancel_asr();
        actor.asr_cleanup_pending.clear();
        actor.asr_runtime.unregister_session(&actor.session_id);
        actor.asr_runtime = std::sync::Arc::clone(&runtimes.asr);
        actor.asr_events = actor.asr_runtime.register_session(&actor.session_id);
    }
    if !std::sync::Arc::ptr_eq(&actor.llm_runtime, &runtimes.llm) {
        actor.llm_runtime.unregister_session(&actor.session_id);
        actor.llm_runtime = std::sync::Arc::clone(&runtimes.llm);
        actor.llm_events = actor
            .llm_runtime
            .register_session(&actor.session_id, LLM_EVENT_CAPACITY);
    }
    if !std::sync::Arc::ptr_eq(&actor.tts_runtime, &runtimes.tts) {
        actor.speech_output.release();
        actor.tts_runtime = std::sync::Arc::clone(&runtimes.tts);
        actor.speech_output = speech_output;
    }
    actor.vad_segmenter = VadSegmenter::new(runtimes.vad_segmenter);
    actor.pre_roll_samples = runtimes.vad_pre_roll_samples;
    actor.auto_retention = AutoPcmRetention::new(auto_retention_capacity(
        runtimes.vad.runtime_config().command_capacity,
        runtimes.vad_segmenter.min_speech_samples,
        runtimes.vad_pre_roll_samples,
    ));
    Ok(())
}
