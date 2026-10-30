use super::*;

#[test]
fn resolved_llm_provider_exposes_capability_tags_and_enforces_agent_compatibility() {
    use crate::services::task_flow::agent_settings::{
        AgentConfigOverride, ResolvedLlmProvider, find_provider_by_tag, validate_agent_overrides,
    };
    use std::collections::BTreeMap;

    let chat_provider = ResolvedLlmProvider {
        provider_ref: "llm_chat".into(),
        name: "GPT-4o".into(),
        source: "database",
        adapter: "openai".into(),
        model: "gpt-4o".into(),
        base_url: "https://api.example.com/v1".into(),
        api_key: None,
        gateway_binding: None,
        timeout_ms: 30_000,
        max_tokens: 4_096,
        context_limit_tokens: 128_000,
        reasoning_effort: None,
        temperature_percent: None,
        tags: vec!["chat".into(), "vision".into()],
        muse_options: None,
    };
    let image_only_provider = ResolvedLlmProvider {
        provider_ref: "llm_image".into(),
        name: "Flux Image".into(),
        source: "database",
        adapter: "openai".into(),
        model: "flux-1".into(),
        base_url: "https://api.example.com/v1".into(),
        api_key: None,
        gateway_binding: None,
        timeout_ms: 30_000,
        max_tokens: 4_096,
        context_limit_tokens: 32_000,
        reasoning_effort: None,
        temperature_percent: None,
        tags: vec!["image".into()],
        muse_options: None,
    };

    let summary = chat_provider.to_json_summary();
    assert_eq!(summary["tags"], serde_json::json!(["chat", "vision"]));

    let catalog = vec![chat_provider, image_only_provider];
    assert_eq!(
        find_provider_by_tag(&catalog, "image").map(|p| p.provider_ref.as_str()),
        Some("llm_image")
    );
    assert_eq!(
        find_provider_by_tag(&catalog, "vision").map(|p| p.provider_ref.as_str()),
        Some("llm_chat")
    );
    assert!(find_provider_by_tag(&catalog, "video").is_none());

    // Orchestrator provider must support `chat`
    assert_eq!(
        validate_agent_overrides("llm_image", "llm_chat", &BTreeMap::new(), &catalog),
        Err("provider_capability_mismatch")
    );

    // Media creator can use an `image`- or `video`-tagged provider, whereas `wiki_researcher` requires `chat`
    let video_only_provider = ResolvedLlmProvider {
        provider_ref: "llm_video".into(),
        name: "Sora Video".into(),
        source: "database",
        adapter: "openai".into(),
        model: "sora-1".into(),
        base_url: "https://api.example.com/v1".into(),
        api_key: None,
        gateway_binding: None,
        timeout_ms: 60_000,
        max_tokens: 4_096,
        context_limit_tokens: 32_000,
        reasoning_effort: None,
        temperature_percent: None,
        tags: vec!["video".into()],
        muse_options: None,
    };
    let audio_only_provider = ResolvedLlmProvider {
        provider_ref: "llm_audio".into(),
        name: "TTS Model".into(),
        source: "database",
        adapter: "openai".into(),
        model: "tts-1".into(),
        base_url: "https://api.example.com/v1".into(),
        api_key: None,
        gateway_binding: None,
        timeout_ms: 30_000,
        max_tokens: 4_096,
        context_limit_tokens: 32_000,
        reasoning_effort: None,
        temperature_percent: None,
        tags: vec!["audio".into()],
        muse_options: None,
    };
    let mut catalog = catalog;
    catalog.push(video_only_provider);
    catalog.push(audio_only_provider);

    let mut media_overrides = BTreeMap::new();
    media_overrides.insert(
        "media_creator".to_owned(),
        AgentConfigOverride {
            enabled: true,
            provider_ref: Some("llm_image".to_owned()),
            ..Default::default()
        },
    );
    assert!(validate_agent_overrides("llm_chat", "llm_chat", &media_overrides, &catalog).is_ok());

    media_overrides.insert(
        "media_creator".to_owned(),
        AgentConfigOverride {
            enabled: true,
            provider_ref: Some("llm_video".to_owned()),
            ..Default::default()
        },
    );
    assert!(validate_agent_overrides("llm_chat", "llm_chat", &media_overrides, &catalog).is_ok());

    // `media_creator` LLM provider only serves chat/reasoning/image/video; TTS uses `tts_synthesize` tool
    let mut audio_media_overrides = BTreeMap::new();
    audio_media_overrides.insert(
        "media_creator".to_owned(),
        AgentConfigOverride {
            enabled: true,
            provider_ref: Some("llm_audio".to_owned()),
            ..Default::default()
        },
    );
    assert_eq!(
        validate_agent_overrides("llm_chat", "llm_chat", &audio_media_overrides, &catalog),
        Err("provider_capability_mismatch")
    );

    // When `media_creator` is assigned a dedicated `video` (or `image`) provider without `chat`,
    // `project_agents_json` reports `llm_video`, while `build_agent_definitions` binds the
    // Orchestrator subagent's tool-calling session to the worker `chat` provider (`llm_chat`).
    let settings = crate::services::task_flow::PersistedBackgroundSettings {
        orchestrator_llm_provider: "llm_chat".into(),
        worker_llm_provider: "llm_chat".into(),
        agents: media_overrides,
        ..Default::default()
    };
    let projected_agents = crate::services::task_flow::project_agents_json(
        &crate::config::OrchestratorSectionConfig::default(),
        &settings,
        &catalog,
    );
    let media_json = projected_agents
        .iter()
        .find(|a| a["id"] == "media_creator")
        .unwrap();
    assert_eq!(media_json["effective_provider_ref"], "llm_video");
    assert_eq!(media_json["effective_model"], "sora-1");
    assert_eq!(
        media_json["allowed_capability_tags"],
        serde_json::json!(["chat", "reasoning", "image", "video"])
    );

    let (_, defs) = crate::services::task_flow::build_agent_definitions(
        &crate::config::OrchestratorSectionConfig::default(),
        &settings,
        &catalog,
    );
    let media_def = defs.iter().find(|d| d.id == "media_creator").unwrap();
    assert_eq!(
        media_def.provider_id, "llm_chat",
        "non-chat media provider must fall back to worker chat provider for Orchestrator tool-calling loop"
    );

    let mut wiki_overrides = BTreeMap::new();
    wiki_overrides.insert(
        "wiki_researcher".to_owned(),
        AgentConfigOverride {
            enabled: true,
            provider_ref: Some("llm_image".to_owned()),
            ..Default::default()
        },
    );
    assert_eq!(
        validate_agent_overrides("llm_chat", "llm_chat", &wiki_overrides, &catalog),
        Err("provider_capability_mismatch")
    );

    // Per-tool provider configuration (`tool_providers`) across `media_creator` and `media_processor`:
    // allows binding `media_creator` (`image_create` -> `llm_image`, `video_create` -> `llm_video`)
    // and `media_processor` (`tts_synthesize` -> `llm_audio`).
    let mut per_tool_overrides = BTreeMap::new();
    per_tool_overrides.insert(
        "media_creator".to_owned(),
        AgentConfigOverride {
            enabled: true,
            provider_ref: Some("llm_chat".to_owned()),
            tool_providers: Some(BTreeMap::from([
                ("image_create".to_owned(), "llm_image".to_owned()),
                ("video_create".to_owned(), "llm_video".to_owned()),
            ])),
            ..Default::default()
        },
    );
    per_tool_overrides.insert(
        "media_processor".to_owned(),
        AgentConfigOverride {
            enabled: true,
            provider_ref: Some("llm_chat".to_owned()),
            tool_providers: Some(BTreeMap::from([(
                "tts_synthesize".to_owned(),
                "llm_audio".to_owned(),
            )])),
            ..Default::default()
        },
    );
    assert!(
        validate_agent_overrides("llm_chat", "llm_chat", &per_tool_overrides, &catalog).is_ok()
    );

    let per_tool_settings = crate::services::task_flow::PersistedBackgroundSettings {
        orchestrator_llm_provider: "llm_chat".into(),
        worker_llm_provider: "llm_chat".into(),
        agents: per_tool_overrides,
        ..Default::default()
    };
    let projected_per_tool = crate::services::task_flow::project_agents_json(
        &crate::config::OrchestratorSectionConfig::default(),
        &per_tool_settings,
        &catalog,
    );
    let media_per_tool_json = projected_per_tool
        .iter()
        .find(|a| a["id"] == "media_creator")
        .unwrap();
    assert_eq!(
        media_per_tool_json["tool_capability_tags"],
        serde_json::json!({
            "image_create": "image",
            "video_create": "video"
        })
    );
    assert_eq!(
        media_per_tool_json["tool_providers"],
        serde_json::json!({
            "image_create": "llm_image",
            "video_create": "llm_video"
        })
    );
    assert_eq!(
        media_per_tool_json["effective_tool_providers"],
        serde_json::json!({
            "image_create": "llm_image",
            "video_create": "llm_video"
        })
    );
    let proc_per_tool_json = projected_per_tool
        .iter()
        .find(|a| a["id"] == "media_processor")
        .unwrap();
    assert_eq!(
        proc_per_tool_json["allowed_capability_tags"],
        serde_json::json!(["chat", "reasoning", "tts", "asr", "audio"])
    );
    assert_eq!(
        proc_per_tool_json["tool_capability_tags"],
        serde_json::json!({
            "tts_synthesize": "tts",
            "asr_transcribe": "asr"
        })
    );
    assert_eq!(
        proc_per_tool_json["tool_providers"],
        serde_json::json!({
            "tts_synthesize": "llm_audio"
        })
    );
    assert_eq!(
        proc_per_tool_json["effective_tool_providers"]["tts_synthesize"],
        "llm_audio"
    );

    // Verify legacy `agent_settings.json` migration: `tts_synthesize` and `asr_transcribe`
    // under `media_creator.tool_providers` are automatically migrated to `media_processor`
    // with `media_processor.enabled == true`.
    let mig_dir = std::env::temp_dir().join(format!("legacy-media-mig-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&mig_dir).unwrap();
    std::fs::write(
        mig_dir.join("agent_settings.json"),
        serde_json::json!({
            "revision": 3,
            "orchestrator_llm_provider": "llm_chat",
            "worker_llm_provider": "llm_chat",
            "agents": {
                "media_creator": {
                    "enabled": true,
                    "tool_providers": {
                        "image_create": "llm_image",
                        "tts_synthesize": "llm_audio",
                        "asr_transcribe": "config:asr:zipformer_vi"
                    }
                }
            }
        })
        .to_string(),
    )
    .unwrap();
    let migrated =
        crate::services::task_flow::load_persisted_settings(&mig_dir).expect("migrated settings");
    let mig_creator = migrated.agents.get("media_creator").unwrap();
    assert_eq!(
        mig_creator.tool_providers.as_ref().unwrap(),
        &BTreeMap::from([("image_create".to_owned(), "llm_image".to_owned())])
    );
    let mig_proc = migrated
        .agents
        .get("media_processor")
        .expect("media_processor created during migration");
    assert!(
        mig_proc.enabled,
        "migrated media_processor must remain enabled (true)"
    );
    assert_eq!(
        mig_proc.tool_providers.as_ref().unwrap(),
        &BTreeMap::from([
            (
                "asr_transcribe".to_owned(),
                "config:asr:zipformer_vi".to_owned()
            ),
            ("tts_synthesize".to_owned(), "llm_audio".to_owned()),
        ])
    );
    let _ = std::fs::remove_dir_all(&mig_dir);

    // Assigning a non-image provider (`llm_video`) to `image_create` must fail with `provider_capability_mismatch`
    let mut bad_tool_overrides = BTreeMap::new();
    bad_tool_overrides.insert(
        "media_creator".to_owned(),
        AgentConfigOverride {
            enabled: true,
            tool_providers: Some(BTreeMap::from([(
                "image_create".to_owned(),
                "llm_video".to_owned(),
            )])),
            ..Default::default()
        },
    );
    assert_eq!(
        validate_agent_overrides("llm_chat", "llm_chat", &bad_tool_overrides, &catalog),
        Err("provider_capability_mismatch")
    );

    assert!(crate::providers::llm::valid_llm_tags(Some(&[
        "tts".to_owned()
    ])));
    assert!(crate::providers::llm::valid_llm_tags(Some(&[
        "TTS".to_owned()
    ])));
    assert_eq!(
        crate::providers::llm::llm_config_tags(r#"{"tags":["tts"]}"#),
        vec!["tts".to_owned()]
    );
    assert!(crate::providers::llm::llm_config_supports_tag(
        r#"{"tags":["tts"]}"#,
        "audio"
    ));
}

#[test]
fn muse_gateway_provider_in_catalog_supports_media_and_custom_executor() {
    use std::collections::BTreeMap;

    // 1. Standalone `muse` adapter is removed; Muse is configured via `llm_gateway` (`pool:muse`)
    let valid_json = r#"{"gateway_binding":"pool:muse","model":"muse-image","account_concurrency":2,"cdp_port":19222,"headless":true,"tags":["image","video"]}"#;
    assert!(crate::database::provider_config::validate_raw("muse", valid_json).is_err());
    let canonical = crate::database::provider_config::validate_raw("llm_gateway", valid_json)
        .expect("valid llm_gateway muse config_json");
    assert!(canonical.contains("pool:muse"));
    assert!(canonical.contains("muse-image"));

    // Rejects embedded credentials in `config_json`
    let with_secret =
        r#"{"gateway_binding":"pool:muse","model":"muse-image","api_key":"secret_token"}"#;
    assert!(crate::database::provider_config::validate_raw("llm_gateway", with_secret).is_err());

    // 2. Verify `llm_gateway` bound to `pool:muse` in Task Flow catalog exposes `image` & `video`
    // and uses `ProviderAuth::CustomExecutor` (never `ProviderAuth::ApiKey` or plain OpenAI).
    let cfg = OrchestratorSectionConfig::default();
    let chat_prov = default_orchestrator_provider(&cfg);
    let muse_prov = ResolvedLlmProvider {
        provider_ref: "muse_media".into(),
        name: "Muse Media".into(),
        source: "database",
        adapter: "llm_gateway".into(),
        model: "muse-image".into(),
        base_url: "https://muse.ai".into(),
        api_key: None,
        gateway_binding: Some(crate::services::llm_gateway::GatewayBinding::Pool(
            crate::services::llm_gateway::GatewayVendor::Muse,
        )),
        timeout_ms: 300_000,
        max_tokens: 4_096,
        context_limit_tokens: 128_000,
        reasoning_effort: None,
        temperature_percent: None,
        tags: vec!["image".into(), "video".into()],
        muse_options: Some(
            crate::services::task_flow::agent_settings::MuseCatalogOptions {
                account_concurrency: 2,
                cdp_port: 19222,
                headless: true,
            },
        ),
    };

    assert!(muse_prov.supports_tag("image"));
    assert!(muse_prov.supports_tag("video"));
    assert!(!muse_prov.supports_tag("chat"));
    assert!(muse_prov.is_muse_provider());
    let muse_vc = muse_prov.video_constraints();
    assert_eq!(muse_vc.max_clip_duration_secs, 10);
    assert_eq!(muse_vc.max_clips_per_call, 6);
    assert_eq!(muse_vc.max_total_duration_secs, 60);
    assert!(muse_vc.provider_guidance.contains("automatically merges"));

    let openai_vc = chat_prov.video_constraints();
    assert_eq!(openai_vc.max_clip_duration_secs, 10);
    assert_eq!(openai_vc.max_clips_per_call, 1);
    assert_eq!(openai_vc.max_total_duration_secs, 10);

    let catalog = vec![chat_prov, muse_prov];
    let mut agents = BTreeMap::new();
    agents.insert(
        "media_creator".to_owned(),
        AgentConfigOverride {
            enabled: true,
            provider_ref: Some("muse_media".to_owned()),
            tool_providers: Some(BTreeMap::from([
                ("image_create".to_owned(), "muse_media".to_owned()),
                ("video_create".to_owned(), "muse_media".to_owned()),
            ])),
            ..Default::default()
        },
    );
    assert!(
        validate_agent_overrides(
            DEFAULT_ORCHESTRATOR_PROVIDER_REF,
            DEFAULT_ORCHESTRATOR_PROVIDER_REF,
            &agents,
            &catalog,
        )
        .is_ok()
    );

    let settings = PersistedBackgroundSettings {
        orchestrator_llm_provider: DEFAULT_ORCHESTRATOR_PROVIDER_REF.into(),
        worker_llm_provider: DEFAULT_ORCHESTRATOR_PROVIDER_REF.into(),
        agents,
        ..Default::default()
    };
    let (orch_providers, defs) = build_agent_definitions(&cfg, &settings, &catalog);
    let muse_orch = orch_providers
        .iter()
        .find(|p| p.id == "muse_media")
        .expect("muse_media registered with CustomExecutor");
    assert!(matches!(
        muse_orch.auth,
        orchestrator_agent_rs::ProviderAuth::CustomExecutor(_)
    ));
    let media_def = defs.iter().find(|d| d.id == "media_creator").unwrap();
    assert_eq!(media_def.provider_id, DEFAULT_ORCHESTRATOR_PROVIDER_REF);
    assert!(
        media_def
            .description
            .contains("[Active video_create provider: Muse Media (model: muse-image); max 10s/clip, up to 6 clip(s) = 60s per call; auto-merges multi-clip outputs into a single primary MP4]")
    );
    let proc_spec = crate::services::task_flow::find_agent_spec("media_processor").unwrap();
    assert!(proc_spec.default_allowed_tools.contains(&"video_concat"));
    assert!(proc_spec.available_tools.contains(&"video_concat"));

    let backend = CatalogMediaBackend::from_catalog(catalog).with_settings(settings);
    let backend_vc = orchestrator_flow_rs::VideoGeneratorBackend::video_constraints(&backend);
    assert_eq!(backend_vc.max_clip_duration_secs, 10);
    assert_eq!(backend_vc.max_clips_per_call, 6);
    assert_eq!(backend_vc.max_total_duration_secs, 60);

    // 3. Verify `build_generated_video_payload` converts multi-part `LlmOutput::Media` + `assistant_message`
    // into `GeneratedVideoPayload` with accurate clip durations parsed via `inspect_mp4_duration_secs`.
    let mut mvhd_10s = Vec::new();
    let mvhd_body_len = 4 + 16 + 4; // version/flags + 4 u32s + duration
    let mvhd_box_len = (8 + mvhd_body_len) as u32;
    let moov_box_len = 8 + mvhd_box_len;
    mvhd_10s.extend_from_slice(&20u32.to_be_bytes());
    mvhd_10s.extend_from_slice(b"ftypisom\x00\x00\x02\x00isom");
    mvhd_10s.extend_from_slice(&moov_box_len.to_be_bytes());
    mvhd_10s.extend_from_slice(b"moov");
    mvhd_10s.extend_from_slice(&mvhd_box_len.to_be_bytes());
    mvhd_10s.extend_from_slice(b"mvhd");
    mvhd_10s.extend_from_slice(&[0, 0, 0, 0]); // version 0
    mvhd_10s.extend_from_slice(&[0u8; 8]); // creation + modification time
    mvhd_10s.extend_from_slice(&1_000u32.to_be_bytes()); // timescale = 1000
    mvhd_10s.extend_from_slice(&10_000u32.to_be_bytes()); // duration = 10_000 -> 10s
    let multi_media = crate::providers::LlmOutput::from_media_parts(
        vec![
            crate::providers::MediaPart {
                bytes: mvhd_10s.clone(),
                mime_type: "video/mp4".into(),
            },
            crate::providers::MediaPart {
                bytes: mvhd_10s,
                mime_type: "video/mp4".into(),
            },
        ],
        Some("Scene 1 (0-10s)\n\nScene 2 (10-20s)".into()),
    )
    .unwrap();
    let multi_payload = crate::services::task_flow::agent_settings::build_generated_video_payload(
        &orchestrator_flow_rs::VideoGenerationRequest {
            prompt: "Two scenes".into(),
            title: None,
            filename: "two-scenes.mp4".into(),
            width: 1280,
            height: 720,
            duration_secs: 20,
            first_frame: None,
        },
        "muse_media",
        &backend_vc,
        1280,
        720,
        20,
        multi_media,
    )
    .unwrap();
    assert_eq!(multi_payload.clips.len(), 2);
    assert_eq!(multi_payload.clips[0].filename, "two-scenes-part-1.mp4");
    assert_eq!(multi_payload.clips[1].filename, "two-scenes-part-2.mp4");
    assert_eq!(multi_payload.clips[0].duration_secs, 10);
    assert_eq!(multi_payload.clips[1].duration_secs, 10);
    assert_eq!(multi_payload.duration_secs, 20);
    assert_eq!(
        multi_payload.provider_message.as_deref(),
        Some("Scene 1 (0-10s)\n\nScene 2 (10-20s)")
    );
}

#[test]
fn load_persisted_settings_migrates_legacy_media_creator_tts_and_asr_tool_providers_to_media_processor()
 {
    let temp_dir = std::env::temp_dir().join(format!(
        "voice-agent-legacy-media-settings-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let settings_path = temp_dir.join("agent_settings.json");

    // 1. Legacy settings without `media_processor`, where `media_creator` holds `tts_synthesize` & `asr_transcribe`
    let legacy_json = serde_json::json!({
        "enabled": true,
        "orchestrator_llm_provider": "orchestrator:default",
        "worker_llm_provider": "orchestrator:default",
        "revision": 3,
        "agents": {
            "media_creator": {
                "enabled": true,
                "tool_providers": {
                    "image_create": "img_prov",
                    "tts_synthesize": "tts_prov",
                    "asr_transcribe": "asr_prov"
                }
            }
        }
    });
    std::fs::write(
        &settings_path,
        serde_json::to_string_pretty(&legacy_json).unwrap(),
    )
    .unwrap();

    let loaded = load_persisted_settings(&temp_dir).expect("loaded legacy settings");

    let mc = loaded.agents.get("media_creator").expect("media_creator");
    let mc_tools = mc.tool_providers.as_ref().expect("mc tool_providers");
    assert_eq!(
        mc_tools.get("image_create").map(String::as_str),
        Some("img_prov")
    );
    assert!(!mc_tools.contains_key("tts_synthesize"));
    assert!(!mc_tools.contains_key("asr_transcribe"));

    let mp = loaded
        .agents
        .get("media_processor")
        .expect("migrated media_processor");
    assert!(
        mp.enabled,
        "migrated media_processor must default to enabled: true"
    );
    let mp_tools = mp.tool_providers.as_ref().expect("mp tool_providers");
    assert_eq!(
        mp_tools.get("tts_synthesize").map(String::as_str),
        Some("tts_prov")
    );
    assert_eq!(
        mp_tools.get("asr_transcribe").map(String::as_str),
        Some("asr_prov")
    );

    // 2. When `media_processor` already has an explicit override, legacy keys on `media_creator` do not overwrite it
    let mixed_json = serde_json::json!({
        "enabled": true,
        "orchestrator_llm_provider": "orchestrator:default",
        "worker_llm_provider": "orchestrator:default",
        "revision": 4,
        "agents": {
            "media_creator": {
                "enabled": true,
                "tool_providers": {
                    "tts_synthesize": "old_tts_prov"
                }
            },
            "media_processor": {
                "enabled": true,
                "tool_providers": {
                    "tts_synthesize": "new_tts_prov"
                }
            }
        }
    });
    std::fs::write(
        &settings_path,
        serde_json::to_string_pretty(&mixed_json).unwrap(),
    )
    .unwrap();
    let loaded_mixed = load_persisted_settings(&temp_dir).expect("loaded mixed settings");
    assert!(
        loaded_mixed
            .agents
            .get("media_creator")
            .unwrap()
            .tool_providers
            .as_ref()
            .is_none_or(|m| m.is_empty())
    );
    assert_eq!(
        loaded_mixed
            .agents
            .get("media_processor")
            .unwrap()
            .tool_providers
            .as_ref()
            .unwrap()
            .get("tts_synthesize")
            .map(String::as_str),
        Some("new_tts_prov")
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}
