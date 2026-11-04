use std::sync::{Arc, atomic::Ordering};
use std::time::Duration;

use futures_util::SinkExt;
use tokio_tungstenite::tungstenite::Message;
use vela_server::tools::builtin::SWITCH_TEMPLATE_TOOL_NAME;

use super::harness::*;

#[tokio::test]
async fn a_session_without_a_template_assignment_is_never_offered_the_switch_tool() {
    let (llm, offered_tools) = SwitchScriptLlm::new("default-llm", &["primary"]);
    let (base, url, task) = start_with_llm(
        with_server_default_prompt(config("127.0.0.1:0".parse().unwrap(), database_url())),
        Arc::new(llm),
        |state| state,
    )
    .await;
    let _pool = seed(&url).await;

    let mut socket = admit(&base).await;
    assert!(
        speak_once(&mut socket)
            .await
            .contains(DEFAULT_PROMPT_MARKER)
    );
    assert!(
        offered_tools
            .lock()
            .unwrap()
            .iter()
            .flatten()
            .all(|tool| tool.name != SWITCH_TEMPLATE_TOOL_NAME),
        "an Agent with no assignment must not be offered a switch it cannot honor"
    );
    task.abort();
}

#[tokio::test]
async fn the_switch_tool_offers_exactly_the_candidates_admission_validated() {
    let (base, url, task, offered_tools) = start_with_scripted_switch(&[]).await;
    let pool = seed(&url).await;
    seed_switchable_pair(&pool).await;
    // Bound to a provider this process never loaded, so admission must exclude it from the catalog.
    let broken = insert_template(&pool, "broken", TEMPLATE_PROMPT_V3, true).await;
    bind(
        &pool,
        broken,
        &[
            ("vad", "test"),
            ("asr", "test"),
            ("tts", "test"),
            ("llm", "never-loaded"),
        ],
    )
    .await;
    assign(&pool, broken, false, true).await;

    let mut socket = admit(&base).await;
    assert_eq!(
        speak_once(&mut socket).await,
        "default-llm[TEMPLATE-PROMPT-V1]"
    );
    let switch_tool = offered_tools
        .lock()
        .unwrap()
        .first()
        .and_then(|tools| {
            tools
                .iter()
                .find(|tool| tool.name == SWITCH_TEMPLATE_TOOL_NAME)
        })
        .cloned()
        .expect("an Agent with admitted candidates may switch");
    assert_eq!(
        switch_tool.parameters["properties"]["template"]["enum"],
        serde_json::json!(["primary", "sales"]),
        "the model is offered this session's own candidates and nothing else"
    );
    task.abort();
}

#[tokio::test]
async fn a_switch_applies_at_the_next_turn_boundary_on_the_candidates_loaded_runtime() {
    let (base, url, task, _) = start_with_scripted_switch(&["sales"]).await;
    let pool = seed(&url).await;
    seed_switchable_pair(&pool).await;

    let mut socket = admit(&base).await;
    assert_eq!(
        speak_once(&mut socket).await,
        "default-llm[TEMPLATE-PROMPT-V1]",
        "the turn that requested the switch must finish under the profile it started with"
    );
    assert_eq!(
        speak_once(&mut socket).await,
        "template-llm[TEMPLATE-PROMPT-V2]",
        "the next turn must run the switched prompt on the switched already-loaded runtime"
    );
    task.abort();
}

#[tokio::test]
async fn a_switch_uses_the_admission_snapshot_even_after_the_database_changes() {
    let (base, url, task, _) = start_with_scripted_switch(&["sales"]).await;
    let pool = seed(&url).await;
    seed_switchable_pair(&pool).await;
    let mut socket = admit(&base).await;
    assert_eq!(
        speak_once(&mut socket).await,
        "default-llm[TEMPLATE-PROMPT-V1]"
    );

    sqlx::query("UPDATE agent_templates SET prompt = ? WHERE key = 'sales'")
        .bind(TEMPLATE_PROMPT_V3)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE agent_template_assignments SET enabled = 0 WHERE template_id = 2")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE providers SET revision = revision + 1")
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(
        speak_once(&mut socket).await,
        "template-llm[TEMPLATE-PROMPT-V2]",
        "a switch must never read live database state, so the admission snapshot stays authoritative"
    );
    task.abort();
}

#[tokio::test]
async fn a_candidate_excluded_at_admission_can_never_be_switched_to() {
    let (base, url, task, offered_tools) = start_with_scripted_switch(&["broken"]).await;
    let pool = seed(&url).await;
    seed_switchable_pair(&pool).await;
    let broken = insert_template(&pool, "broken", TEMPLATE_PROMPT_V3, true).await;
    bind(
        &pool,
        broken,
        &[
            ("vad", "test"),
            ("asr", "test"),
            ("tts", "test"),
            ("llm", "never-loaded"),
        ],
    )
    .await;
    assign(&pool, broken, false, true).await;

    let mut socket = admit(&base).await;
    assert_eq!(
        speak_once(&mut socket).await,
        "default-llm[TEMPLATE-PROMPT-V1]",
        "a rejected switch is answered by the ordinary tool continuation"
    );
    assert_eq!(
        speak_once(&mut socket).await,
        "default-llm[TEMPLATE-PROMPT-V1]",
        "a rejected switch must leave the active profile untouched"
    );
    let switch_tool = offered_tools
        .lock()
        .unwrap()
        .first()
        .and_then(|tools| {
            tools
                .iter()
                .find(|tool| tool.name == SWITCH_TEMPLATE_TOOL_NAME)
        })
        .cloned()
        .expect("an Agent with admitted candidates may switch");
    assert_eq!(
        switch_tool.parameters["properties"]["template"]["enum"],
        serde_json::json!(["primary", "sales"]),
        "an excluded candidate is never advertised, let alone switchable"
    );
    task.abort();
}

/// A switch the session armed is discarded when its turn is interrupted before the boundary, so a
/// later turn still runs the profile it started with and the revision never advances.
#[tokio::test]
async fn an_interrupted_turn_drops_the_switch_it_armed() {
    let (base, url, task, _) =
        start_with_stalled_switch(&["sales"], Duration::from_millis(750)).await;
    let pool = seed(&url).await;
    seed_switchable_pair(&pool).await;
    let mut socket = admit(&base).await;

    socket
        .send(Message::Text(
            serde_json::json!({"type": "listen", "state": "start", "mode": "manual"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::Text(
            serde_json::json!({"type": "listen", "state": "detect", "text": "chuyển sang sales"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    // The switch is armed and the turn is now parked in its continuation, well inside the stall.
    tokio::time::sleep(Duration::from_millis(150)).await;
    socket
        .send(Message::Text(
            serde_json::json!({"type": "abort"}).to_string().into(),
        ))
        .await
        .unwrap();

    assert_eq!(
        speak_once(&mut socket).await,
        "default-llm[TEMPLATE-PROMPT-V1]",
        "a turn interrupted before its boundary must not carry its switch forward"
    );
    task.abort();
}

/// Regression for the boundary ordering a switch depends on.
///
/// The writer reports `TurnClosed(Normal)` immediately after the client sees `tts:stop`, so the
/// turns below are sent back-to-back with no delay in between. Each must reach the new profile and
/// keep its captured audio intact — a client that starts a turn the instant a turn closes is
/// entitled to that.
///
/// There is no client-visible synchronization point between `tts:stop` and the writer's own report,
/// so a single turn is only a probabilistic probe. Repeating the back-to-back boundary several
/// times multiplies the opportunities to overtake it, which is what makes this a regression rather
/// than a formality.
#[tokio::test]
async fn the_turn_after_a_switch_keeps_every_audio_frame_and_runs_the_new_profile() {
    const BACK_TO_BACK_TURNS: usize = 4;

    let (base, url, task, _) = start_with_scripted_switch(&["sales"]).await;
    let pool = seed(&url).await;
    seed_switchable_pair(&pool).await;
    let mut socket = admit(&base).await;

    let switching = speak_and_observe(&mut socket).await;
    assert_eq!(
        switching.answer, "default-llm[TEMPLATE-PROMPT-V1]",
        "the turn that requested the switch must finish under the profile it started with"
    );
    assert!(switching.audio_frames > 0);

    for turn in 0..BACK_TO_BACK_TURNS {
        let switched = speak_and_observe(&mut socket).await;
        assert_eq!(
            switched.transcript.as_deref(),
            Some("utterance"),
            "turn {turn} started the instant the previous one closed, so its capture frames must \
             reach ASR rather than be dropped against a still-closing turn"
        );
        assert!(switched.playback_started, "turn {turn} must reach playback");
        assert_eq!(
            switched.audio_frames, switching.audio_frames,
            "turn {turn} must deliver the same audio as the turn before it"
        );
        assert_eq!(
            switched.answer, "template-llm[TEMPLATE-PROMPT-V2]",
            "turn {turn} must run the switched prompt on the switched already-loaded runtime"
        );
    }
    task.abort();
}

/// Deterministic proof of AC2 at the WebSocket boundary.
///
/// The outcome is provably in the actor's mailbox and provably not applied, so the only way the
/// next turn can be captured and answered under the switched profile is that client ingress applies
/// the boundary before interpreting it. Removing that step fails this test every single run.
#[tokio::test]
async fn a_turn_arriving_after_a_reported_boundary_keeps_its_frames_and_uses_the_switched_profile()
{
    let (base, url, task, _, mut boundary) = start_controlled_switch(&["sales"]).await;
    let pool = seed(&url).await;
    seed_switchable_pair(&pool).await;
    let mut socket = admit(&base).await;

    let switching = boundary
        .run_turn_and_hold_at_its_boundary(&mut socket)
        .await;
    assert_eq!(
        switching.answer, "default-llm[TEMPLATE-PROMPT-V1]",
        "the turn that requested the switch must finish under the profile it started with"
    );

    let switched = speak_and_observe(&mut socket).await;

    assert_eq!(
        switched.transcript.as_deref(),
        Some("utterance"),
        "the reported boundary must be applied before this turn is interpreted, or its capture \
         frames are dropped against a turn that has already closed"
    );
    assert!(switched.playback_started);
    assert_eq!(switched.audio_frames, switching.audio_frames);
    assert_eq!(
        switched.answer, "template-llm[TEMPLATE-PROMPT-V2]",
        "the reported boundary must be applied before this turn is interpreted, so the switch it \
         armed takes effect exactly now"
    );
    boundary.hold.store(false, Ordering::SeqCst);
    task.abort();
}

/// Deterministic proof that a normal boundary stays normal.
///
/// An `abort` arriving after the writer reported `TurnClosed(Normal)` must not retroactively turn
/// that turn into an aborted one: the boundary commits first, switch included, and the abort then
/// applies to the fresh session.
#[tokio::test]
async fn an_abort_arriving_after_a_reported_boundary_cannot_undo_the_committed_switch() {
    let (base, url, task, _, mut boundary) = start_controlled_switch(&["sales"]).await;
    let pool = seed(&url).await;
    seed_switchable_pair(&pool).await;
    let mut socket = admit(&base).await;

    let switching = boundary
        .run_turn_and_hold_at_its_boundary(&mut socket)
        .await;
    assert_eq!(switching.answer, "default-llm[TEMPLATE-PROMPT-V1]");

    socket
        .send(Message::Text(
            serde_json::json!({"type": "abort"}).to_string().into(),
        ))
        .await
        .unwrap();
    let after_abort = speak_and_observe(&mut socket).await;

    assert_eq!(
        after_abort.answer, "template-llm[TEMPLATE-PROMPT-V2]",
        "the normal boundary commits its switch before the abort is interpreted; treating the abort \
         first would discard the switch as an interrupted turn"
    );
    assert_eq!(after_abort.transcript.as_deref(), Some("utterance"));
    boundary.hold.store(false, Ordering::SeqCst);
    task.abort();
}
