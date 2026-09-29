use super::*;
use crate::{
    config::McpResultDelivery,
    providers::{ResolvedAgentRuntimes, llm::ToolDefinition},
    tools::{
        builtin::{
            BuiltinTool, EXIT_TOOL_NAME, SWITCH_TEMPLATE_TOOL_NAME, exit_tool_definition,
            parse_exit_args, parse_switch_template_args, switch_template_tool_definition,
        },
        device_mcp::LlmVisibleTool,
    },
};

/// A builtin tool must not be shadowed by, or shadow, a Device MCP tool. Sanitized MCP names can
/// never contain a dot, so the dotted server action is unambiguous while the exit tool is filtered.
pub(super) fn is_builtin_tool_name(name: &str) -> bool {
    name == EXIT_TOOL_NAME || name == SWITCH_TEMPLATE_TOOL_NAME
}

#[derive(Clone, Debug)]
enum ToolTarget {
    Builtin(BuiltinTool),
    DeviceMcp(LlmVisibleTool),
}

impl SessionActor {
    pub(super) fn available_llm_tools(&self) -> Vec<ToolDefinition> {
        let mut tools = vec![exit_tool_definition()];
        // A session whose admission catalog is empty never offers a capability it cannot honor.
        if !self.switch_catalog.is_empty() {
            tools.push(switch_template_tool_definition(
                &self.switch_catalog.template_keys(),
            ));
        }
        tools.extend(
            self.mcp
                .visible
                .iter()
                .filter(|tool| !is_builtin_tool_name(&tool.llm_name))
                .map(|tool| ToolDefinition {
                    name: tool.llm_name.clone(),
                    description: tool.description.clone(),
                    parameters: tool.input_schema.clone(),
                }),
        );
        tools
    }

    fn resolve_tool(&self, name: &str) -> Option<ToolTarget> {
        if name == EXIT_TOOL_NAME {
            return Some(ToolTarget::Builtin(BuiltinTool::EndConversation));
        }
        if name == SWITCH_TEMPLATE_TOOL_NAME {
            return Some(ToolTarget::Builtin(BuiltinTool::SwitchTemplate));
        }
        self.mcp
            .visible
            .iter()
            .find(|tool| tool.llm_name == name)
            .cloned()
            .map(ToolTarget::DeviceMcp)
    }

    pub(super) fn start_tool_batch(&mut self, calls: Vec<ToolCall>) {
        if calls.is_empty() {
            self.fail_speech_delivery();
            return;
        }
        self.tool_batch = Some(ToolBatchState {
            generation: self.generation,
            calls,
            completed_calls: Vec::new(),
            next: 0,
            results: Vec::new(),
            direct_response: None,
        });
        self.dispatch_next_tool();
    }

    fn dispatch_next_tool(&mut self) {
        let next = self.tool_batch.as_ref().and_then(|batch| {
            (batch.generation == self.generation)
                .then(|| batch.calls.get(batch.next).cloned())
                .flatten()
        });
        let Some(call) = next else {
            self.finish_tool_batch();
            return;
        };
        if let Some(batch) = self.tool_batch.as_mut() {
            batch.next += 1;
        }
        match self.resolve_tool(&call.name) {
            Some(ToolTarget::Builtin(tool)) => self.execute_builtin_tool(call, tool),
            Some(ToolTarget::DeviceMcp(tool)) => self.dispatch_device_mcp_tool(call, tool),
            None => self.complete_tool_call(call, Err("unknown_tool")),
        }
    }

    fn execute_builtin_tool(&mut self, call: ToolCall, tool: BuiltinTool) {
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

    /// Schedules a Template the session admitted at its own admission.  Nothing here reads
    /// configuration: the catalog is an immutable snapshot, so a candidate that was never admitted
    /// — or was excluded at admission — simply does not resolve, and the model gets one coarse
    /// error that says nothing about which Templates exist.
    fn execute_template_switch(&mut self, call: ToolCall) {
        let Ok(args) = parse_switch_template_args(&call.arguments) else {
            self.complete_tool_call(call, Err("invalid_arguments"));
            return;
        };
        let Some(turn_id) = self.current_turn_id() else {
            self.complete_tool_call(call, Err("no_active_turn"));
            return;
        };
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
            tool = SWITCH_TEMPLATE_TOOL_NAME,
            template_key = %args.template,
            turn_id = turn_id.get(),
            "Builtin template switch tool called"
        );
        self.pending_actions.switch_template_after_turn = Some(PendingTemplateSwitch {
            turn_id,
            template_key: args.template.clone(),
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

    /// The only place a Template change becomes effective.  Prompt, language, providers and the
    /// already-loaded runtime handles are replaced together, then the Session Profile Revision
    /// advances; a candidate that no longer resolves leaves the active profile untouched.
    pub(super) fn apply_template_switch(&mut self, template_key: &str) {
        let Some(candidate) = self.switch_catalog.find(template_key).cloned() else {
            tracing::warn!(
                event = "session_profile_switch_dropped",
                reason = "candidate_not_admitted",
                "A scheduled template switch no longer names an admitted candidate"
            );
            return;
        };
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
        tracing::info!(
            event = "session_profile_switched",
            template_key = %candidate.template_key,
            previous_revision,
            revision = self.profile.revision,
            "Session profile switched at a turn boundary"
        );
    }

    pub(super) fn complete_tool_call(
        &mut self,
        call: ToolCall,
        result: Result<serde_json::Value, &str>,
    ) {
        let content = match result {
            Ok(value) => {
                crate::session::actor::mcp::log_action_envelope_shape(&value, &call.name);
                if let Some(response) =
                    crate::session::actor::mcp::parse_xiaozhi_direct_response(&value)
                {
                    let content = crate::session::actor::mcp::normalize_tool_result(
                        serde_json::json!({"content":[{"text":response}]}),
                        self.max_tool_result_chars,
                    );
                    let direct_response = serde_json::from_str::<serde_json::Value>(&content)
                        .ok()
                        .and_then(|normalized| normalized["content"].as_str().map(str::to_owned))
                        .filter(|text| !text.trim().is_empty());
                    tracing::info!(
                        event = "mcp_action_response_detected",
                        tool = %call.name,
                        response_chars = response.chars().count(),
                        "MCP action response detected"
                    );
                    self.record_tool_call(call, content);
                    if let (Some(batch), Some(response)) =
                        (self.tool_batch.as_mut(), direct_response)
                    {
                        batch.direct_response.get_or_insert(response);
                    }
                    self.dispatch_next_tool();
                    return;
                }
                tracing::info!(
                    event = "mcp_action_response_not_detected",
                    tool = %call.name,
                    "MCP result remains on generic tool-result path"
                );
                crate::session::actor::mcp::normalize_tool_result(
                    crate::session::actor::mcp::redact_photo_data_from_tool_result(value),
                    self.max_tool_result_chars,
                )
            }
            Err(code) => tool_error_content(code),
        };
        self.record_tool_call(call, content);
        self.dispatch_next_tool();
    }

    fn complete_builtin_tool_call(&mut self, call: ToolCall, content: String, response: String) {
        self.record_tool_call(call, content);
        if let Some(batch) = self.tool_batch.as_mut() {
            batch.direct_response.get_or_insert(response);
        }
        self.dispatch_next_tool();
    }

    fn record_tool_call(&mut self, call: ToolCall, content: String) {
        if let Some(batch) = self.tool_batch.as_mut() {
            batch.completed_calls.push(call.clone());
            batch.results.push(ChatMessage::ToolResult {
                tool_call_id: call.id,
                content,
            });
        }
    }

    fn finish_tool_batch(&mut self) {
        let Some(batch) = self.tool_batch.take() else {
            return;
        };
        self.commit_tool_exchange(&batch);
        if let Some(goodbye) = batch.direct_response {
            self.begin_direct_tool_speech(goodbye);
            return;
        }
        match self.batch_result_delivery(&batch) {
            McpResultDelivery::LlmThenTts => self.begin_tool_continuation(),
            McpResultDelivery::DirectTts => {
                if let Some(text) = self.direct_tts_text(&batch) {
                    self.begin_direct_tool_speech(text);
                } else {
                    self.finish_tool_turn_without_speech();
                }
            }
            McpResultDelivery::Silent => self.finish_tool_turn_without_speech(),
        }
    }

    pub(super) fn commit_tool_exchange(&mut self, batch: &ToolBatchState) {
        if batch.completed_calls.is_empty() {
            return;
        }
        let Some(turn_id) = self.current_turn_id() else {
            return;
        };
        if self.dialogue_history.append_completed_round(
            turn_id,
            batch.completed_calls.clone(),
            batch.results.clone(),
        ) {
            crate::session::prompt::append_completed_round(
                &mut self.llm_messages,
                batch.completed_calls.clone(),
                batch.results.clone(),
            );
        }
    }

    fn batch_result_delivery(&self, batch: &ToolBatchState) -> McpResultDelivery {
        batch
            .calls
            .iter()
            .fold(McpResultDelivery::Silent, |selected, call| {
                let delivery = self
                    .mcp
                    .visible
                    .iter()
                    .find(|tool| tool.llm_name == call.name)
                    .and_then(|tool| self.mcp.tool_delivery.get(&tool.original_name))
                    .copied()
                    .unwrap_or(self.mcp.result_delivery);
                match (selected, delivery) {
                    (McpResultDelivery::LlmThenTts, _) | (_, McpResultDelivery::LlmThenTts) => {
                        McpResultDelivery::LlmThenTts
                    }
                    (McpResultDelivery::DirectTts, _) | (_, McpResultDelivery::DirectTts) => {
                        McpResultDelivery::DirectTts
                    }
                    _ => McpResultDelivery::Silent,
                }
            })
    }

    fn direct_tts_text(&self, batch: &ToolBatchState) -> Option<String> {
        let text = batch
            .results
            .iter()
            .filter_map(|result| match result {
                ChatMessage::ToolResult { content, .. } => {
                    serde_json::from_str::<serde_json::Value>(content)
                        .ok()
                        .filter(|result| result["ok"].as_bool() == Some(true))
                        .and_then(|result| {
                            result["content"].as_str().map(str::trim).map(str::to_owned)
                        })
                }
                _ => None,
            })
            .filter(|text| !text.is_empty() && !text.starts_with('{') && !text.starts_with('['))
            .collect::<Vec<_>>();
        (!text.is_empty()).then(|| text.join("\n"))
    }

    fn finish_tool_turn_without_speech(&mut self) {
        self.generated_response.clear();
        self.complete_recognition();
    }

    pub(super) fn cancel_tool_turn(&mut self) {
        self.cancel_pending_mcp_turn();
        if let Some(batch) = self.tool_batch.take() {
            self.commit_tool_exchange(&batch);
        }
        self.llm_round = None;
    }
}

/// Installs a candidate's already-loaded runtime handles.
///
/// No Provider is constructed here: every handle was resolved against the process-wide Loaded
/// Runtime catalog at admission, so this only re-points the session at runtimes that already
/// exist.
///
/// A worker lease belongs to the runtime that granted it, so an open VAD or ASR lease is closed on
/// its own runtime before any pointer moves and the capture lifecycle re-arms afterwards.  A
/// runtime the candidate does not actually change is left completely untouched: re-registering an
/// unchanged runtime would strand the cleanup acknowledgement a cancelled stream still owes.
///
/// The fallible part is built first, so a rejected install leaves the active profile and its
/// runtimes exactly as they were.
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
        // Close on the runtime that owns the lease: the candidate's VAD never granted it and would
        // refuse it as an unknown lease.
        actor.close_vad();
        actor.vad_session = None;
        actor.vad_cycle = None;
        actor.pending_vad_cycle = None;
        actor.vad_runtime.unregister_session(&actor.session_id);
        actor.vad_runtime = std::sync::Arc::clone(&runtimes.vad);
        actor.vad_events = actor.vad_runtime.register_session(&actor.session_id);
    }
    if !std::sync::Arc::ptr_eq(&actor.asr_runtime, &runtimes.asr) {
        // Cancel on the runtime that owns the lease.  No cleanup acknowledgement is outstanding at
        // a turn boundary, so nothing the departing runtime still owes this session can be lost.
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
    // Segmentation timing belongs to the VAD instance, so it is rebound with the VAD runtime.
    actor.vad_segmenter = VadSegmenter::new(runtimes.vad_segmenter);
    actor.pre_roll_samples = runtimes.vad_pre_roll_samples;
    actor.auto_retention = AutoPcmRetention::new(auto_retention_capacity(
        runtimes.vad.runtime_config().command_capacity,
        runtimes.vad_segmenter.min_speech_samples,
        runtimes.vad_pre_roll_samples,
    ));
    Ok(())
}

fn tool_error_content(code: &str) -> String {
    serde_json::json!({
        "ok": false,
        "code": code,
        "content": "",
        "truncated": false,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        audio::VadSegmenterConfig,
        config::AppConfig,
        config::EffectiveProviderBindings,
        database::{AdmittedAssignment, AdmittedProviderBinding},
        providers::{
            RuntimeCatalog, asr::UnavailableAsr, llm::UnavailableLlm, tts::UnavailableTts,
            vad::UnavailableVad,
        },
        session::profile::resolve_effective_session_profile,
        workers::{
            AsrWorkerRuntime, LlmRuntime, TtsWorkerRuntime, VadWorkerRuntime, WorkerRuntimeConfig,
        },
    };
    use std::{collections::HashMap, sync::Arc};

    fn deployment() -> AppConfig {
        toml::from_str(
            r#"
            [server]
            bind = "127.0.0.1:0"
            public_ws_url = "ws://127.0.0.1:0/voice/v1/"

            [provider_defaults]
            vad = "vad"
            asr = "asr"
            llm = "llm"
            tts = "tts"
            "#,
        )
        .expect("the fixture configuration is valid")
    }

    fn worker() -> WorkerRuntimeConfig {
        WorkerRuntimeConfig {
            max_workers: 1,
            command_capacity: 1,
            final_timeout: std::time::Duration::from_secs(1),
            cleanup_grace: std::time::Duration::from_secs(1),
        }
    }

    /// Carries a second LLM and VAD instance so a switch is observable as a runtime change and not
    /// only as a prompt change.
    fn catalog() -> RuntimeCatalog {
        let segmenter = VadSegmenterConfig::default();
        RuntimeCatalog {
            vad: HashMap::from([
                (
                    "vad".to_owned(),
                    crate::providers::LoadedVad {
                        runtime: Arc::new(VadWorkerRuntime::new(
                            Arc::new(UnavailableVad),
                            worker(),
                        )),
                        segmenter,
                        pre_roll_samples: 4_800,
                    },
                ),
                (
                    "alternate-vad".to_owned(),
                    crate::providers::LoadedVad {
                        runtime: Arc::new(VadWorkerRuntime::new(
                            Arc::new(UnavailableVad),
                            worker(),
                        )),
                        segmenter,
                        pre_roll_samples: 9_600,
                    },
                ),
            ]),
            asr: HashMap::from([(
                "asr".to_owned(),
                Arc::new(AsrWorkerRuntime::new(Arc::new(UnavailableAsr), worker())),
            )]),
            llm: HashMap::from([
                (
                    "llm".to_owned(),
                    Arc::new(LlmRuntime::new(
                        Arc::new(UnavailableLlm),
                        1,
                        std::time::Duration::from_secs(1),
                    )),
                ),
                (
                    "alternate".to_owned(),
                    Arc::new(LlmRuntime::new(
                        Arc::new(UnavailableLlm),
                        1,
                        std::time::Duration::from_secs(1),
                    )),
                ),
            ]),
            tts: HashMap::from([(
                "tts".to_owned(),
                Arc::new(TtsWorkerRuntime::new(Arc::new(UnavailableTts), worker())),
            )]),
            vision: HashMap::new(),
        }
    }

    fn candidate(
        template_id: i64,
        key: &str,
        prompt: &str,
        llm: &str,
        vad: &str,
    ) -> AdmittedAssignment {
        AdmittedAssignment {
            template_id,
            template_key: key.to_owned(),
            template_name: key.to_owned(),
            language: "vi-VN".to_owned(),
            prompt: prompt.to_owned(),
            template_enabled: true,
            template_revision: 2,
            is_default: template_id == 1,
            assignment_enabled: true,
            bindings: [("vad", vad), ("asr", "asr"), ("llm", llm), ("tts", "tts")]
                .into_iter()
                .map(|(provider_type, provider_key)| AdmittedProviderBinding {
                    provider_type: provider_type.to_owned(),
                    provider_key: provider_key.to_owned(),
                    provider_enabled: true,
                })
                .collect(),
        }
    }

    /// A session admitted to the default Template and one switchable candidate.
    fn admitted_actor(catalog: &RuntimeCatalog) -> SessionActor {
        let profile = resolve_effective_session_profile(
            1,
            2,
            "agent",
            &[
                candidate(1, "primary", "primary prompt", "llm", "vad"),
                candidate(7, "usable", "usable prompt", "alternate", "alternate-vad"),
            ],
            &deployment(),
            catalog,
        )
        .expect("the default template resolves against the loaded catalog");
        let (active, switch_catalog) = profile.into_active_profile();
        let bound = catalog
            .resolve(&EffectiveProviderBindings {
                vad: "vad".to_owned(),
                asr: "asr".to_owned(),
                llm: "llm".to_owned(),
                tts: "tts".to_owned(),
                vision: None,
            })
            .expect("the default bindings resolve");
        let (control_tx, _control_rx) = mpsc::channel(4);
        let (audio_tx, _audio_rx) = mpsc::channel(4);
        SessionActor::new_with_runtimes_and_limiter(
            "session".to_owned(),
            control_tx,
            audio_tx,
            16,
            20,
            SessionRuntimes {
                asr: bound.asr,
                vad: bound.vad,
                llm: bound.llm,
                tts: bound.tts,
                active_turn_limiter: Arc::new(ActiveTurnLimiter::new(1)),
                vad_segmenter_config: bound.vad_segmenter,
                pre_roll_samples: bound.vad_pre_roll_samples,
            },
        )
        .expect("the session audio runtime initializes")
        .with_effective_profile(active, switch_catalog, 4_096)
        .expect("the admitted prompt is within the bound")
    }

    #[test]
    fn a_successful_switch_installs_the_candidate_runetimes_and_advances_the_revision() {
        let catalog = catalog();
        let mut actor = admitted_actor(&catalog);
        let initial_llm = Arc::clone(&actor.llm_runtime);
        let alternate_llm = Arc::clone(&catalog.llm["alternate"]);
        assert_eq!(actor.profile_revision(), 1);

        actor.apply_template_switch("usable");

        assert_eq!(actor.profile_revision(), 2);
        assert_eq!(actor.profile.system_prompt, "usable prompt");
        assert_eq!(actor.profile.language, "vi-VN");
        assert!(
            Arc::ptr_eq(&actor.llm_runtime, &alternate_llm)
                && !Arc::ptr_eq(&actor.llm_runtime, &initial_llm),
            "a switch installs the candidate's already-loaded runtime and never reuses the old one"
        );
    }

    #[test]
    fn a_switch_to_a_candidate_this_session_never_admitted_changes_nothing() {
        let catalog = catalog();
        let mut actor = admitted_actor(&catalog);
        let initial_llm = Arc::clone(&actor.llm_runtime);

        actor.apply_template_switch("never-admitted");

        assert_eq!(actor.profile_revision(), 1);
        assert_eq!(actor.profile.system_prompt, "primary prompt");
        assert!(
            Arc::ptr_eq(&actor.llm_runtime, &initial_llm),
            "a rejected switch must not disturb the active runtimes"
        );
    }

    /// A worker lease belongs to the runtime that granted it, so a switch that rebases capture
    /// onto another already-loaded VAD must close the old lease there and re-arm, never hand it
    /// to the new runtime or leave an Auto client without capture.
    #[test]
    fn a_switch_that_rebases_the_vad_runtime_rearms_capture_instead_of_failing_closed() {
        let catalog = catalog();
        let mut actor = admitted_actor(&catalog);
        actor.start_listening(crate::protocol::ListenMode::Auto);
        assert!(
            actor.vad_session.is_some(),
            "Auto mode arms a VAD capture cycle"
        );

        actor.apply_template_switch("usable");
        assert_eq!(actor.phase, SessionPhase::Listening);

        actor.complete_recognition();

        assert_eq!(
            actor.phase,
            SessionPhase::Listening,
            "an Auto session must keep capturing after a VAD rebasing"
        );
        assert!(
            actor.vad_session.is_some(),
            "capture must be re-armed, not left holding a lease the candidate's runtime never granted"
        );
        assert!(
            Arc::ptr_eq(
                &actor.vad_runtime,
                &catalog.vad["alternate-vad"].runtime.clone()
            ),
            "the re-armed cycle must belong to the candidate's own runtime"
        );
        assert_eq!(actor.pre_roll_samples, 9_600);
    }

    /// The overwhelmingly common switch keeps VAD, ASR and TTS on the same instances. Their leases
    /// and mailboxes must then be left completely alone.
    #[test]
    fn a_switch_that_keeps_the_capture_runtimes_leaves_their_leases_open() {
        let catalog = catalog();
        let profile = resolve_effective_session_profile(
            1,
            2,
            "agent",
            &[
                candidate(1, "primary", "primary prompt", "llm", "vad"),
                candidate(7, "usable", "usable prompt", "llm", "vad"),
            ],
            &deployment(),
            &catalog,
        )
        .expect("the default template resolves against the loaded catalog");
        let (active, switch_catalog) = profile.into_active_profile();
        let bound = catalog
            .resolve(&EffectiveProviderBindings {
                vad: "vad".to_owned(),
                asr: "asr".to_owned(),
                llm: "llm".to_owned(),
                tts: "tts".to_owned(),
                vision: None,
            })
            .expect("the default bindings resolve");
        let (control_tx, _control_rx) = mpsc::channel(4);
        let (audio_tx, _audio_rx) = mpsc::channel(4);
        let mut actor = SessionActor::new_with_runtimes_and_limiter(
            "session".to_owned(),
            control_tx,
            audio_tx,
            16,
            20,
            SessionRuntimes {
                asr: bound.asr,
                vad: bound.vad,
                llm: bound.llm,
                tts: bound.tts,
                active_turn_limiter: Arc::new(ActiveTurnLimiter::new(1)),
                vad_segmenter_config: bound.vad_segmenter,
                pre_roll_samples: bound.vad_pre_roll_samples,
            },
        )
        .expect("the session audio runtime initializes")
        .with_effective_profile(active, switch_catalog, 4_096)
        .expect("the admitted prompt is within the bound");
        actor.start_listening(crate::protocol::ListenMode::Auto);
        let lease = actor.vad_session.as_ref().map(|(lease, _)| *lease);

        actor.apply_template_switch("usable");

        assert_eq!(actor.profile.revision, 2);
        assert_eq!(
            actor.vad_session.as_ref().map(|(open, _)| *open),
            lease,
            "an unchanged capture runtime must keep the lease the session already holds"
        );
    }

    #[test]
    fn a_session_without_a_catalog_is_never_offered_the_switch_tool() {
        let catalog = catalog();
        let bound = catalog
            .resolve(&EffectiveProviderBindings {
                vad: "vad".to_owned(),
                asr: "asr".to_owned(),
                llm: "llm".to_owned(),
                tts: "tts".to_owned(),
                vision: None,
            })
            .expect("the default bindings resolve");
        let (control_tx, _control_rx) = mpsc::channel(4);
        let (audio_tx, _audio_rx) = mpsc::channel(4);
        let actor = SessionActor::new_with_runtimes_and_limiter(
            "session".to_owned(),
            control_tx,
            audio_tx,
            16,
            20,
            SessionRuntimes {
                asr: bound.asr,
                vad: bound.vad,
                llm: bound.llm,
                tts: bound.tts,
                active_turn_limiter: Arc::new(ActiveTurnLimiter::new(1)),
                vad_segmenter_config: bound.vad_segmenter,
                pre_roll_samples: bound.vad_pre_roll_samples,
            },
        )
        .expect("the session audio runtime initializes");

        assert!(
            actor
                .available_llm_tools()
                .iter()
                .all(|tool| tool.name != SWITCH_TEMPLATE_TOOL_NAME),
            "a session with no admission catalog cannot offer a switch"
        );
    }
}
