use super::*;

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
    let admitted = profile.into_admitted_profile();
    let (active, switch_catalog) = (admitted.active, admitted.switch_catalog);
    let bound = catalog
        .resolve(&EffectiveProviderBindings {
            vad: "vad".to_owned(),
            asr: "asr".to_owned(),
            llm: "llm".to_owned(),
            tts: "tts".to_owned(),
            vision: None,
            speaker: None,
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
            job_submit: None,
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
    .with_effective_profile(active, switch_catalog, admitted.external_mcp, 4_096)
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
