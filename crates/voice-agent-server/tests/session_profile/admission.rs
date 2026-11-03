use std::sync::Arc;

use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::http::StatusCode};
use voice_agent_server::{app::AppState, database::Database, providers::ProviderSet};

use super::harness::*;

#[tokio::test]
async fn an_agent_without_any_assignment_row_uses_server_defaults() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let _pool = seed(&url).await;
    let mut socket = admit(&base).await;
    assert!(
        speak_once(&mut socket)
            .await
            .contains(DEFAULT_PROMPT_MARKER),
        "an unassigned agent keeps the deployment prompt"
    );
    task.abort();
}

#[tokio::test]
async fn an_agent_with_only_a_disabled_assignment_uses_server_defaults() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    let template = insert_template(&pool, "primary", TEMPLATE_PROMPT_V1, true).await;
    bind(&pool, template, &full_bindings("test")).await;
    assign(&pool, template, true, false).await;

    let mut socket = admit(&base).await;
    assert!(
        speak_once(&mut socket)
            .await
            .contains(DEFAULT_PROMPT_MARKER),
        "a soft-unlinked assignment does not block the deployment profile"
    );
    task.abort();
}

#[tokio::test]
async fn an_agent_without_an_enabled_default_assignment_returns_coarse_503() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    let template = insert_template(&pool, "secondary", TEMPLATE_PROMPT_V1, true).await;
    bind(&pool, template, &full_bindings("test")).await;
    assign(&pool, template, false, true).await;

    let error = connect_async(request(&base)).await.unwrap_err();
    assert_eq!(rejected_status(error), StatusCode::SERVICE_UNAVAILABLE);
    task.abort();
}

#[tokio::test]
async fn a_disabled_default_template_fails_closed() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    let template = insert_template(&pool, "primary", TEMPLATE_PROMPT_V1, false).await;
    bind(&pool, template, &full_bindings("test")).await;
    assign(&pool, template, true, true).await;

    let error = connect_async(request(&base)).await.unwrap_err();
    assert_eq!(rejected_status(error), StatusCode::SERVICE_UNAVAILABLE);
    task.abort();
}

#[tokio::test]
async fn a_default_template_missing_a_binding_uses_the_server_default_for_that_slot() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    let template = insert_template(&pool, "primary", TEMPLATE_PROMPT_V1, true).await;
    bind(
        &pool,
        template,
        &[("vad", "test"), ("asr", "test"), ("llm", "test")],
    )
    .await;
    assign(&pool, template, true, true).await;

    let mut socket = admit(&base).await;
    assert_eq!(
        speak_once(&mut socket).await,
        format!("default-llm[{TEMPLATE_PROMPT_V1}]")
    );
    task.abort();
}

#[tokio::test]
async fn a_default_template_bound_to_an_unloaded_provider_fails_closed() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    let template = insert_template(&pool, "primary", TEMPLATE_PROMPT_V1, true).await;
    bind(
        &pool,
        template,
        &[
            ("vad", "test"),
            ("asr", "test"),
            ("tts", "test"),
            ("llm", "never-loaded"),
        ],
    )
    .await;
    assign(&pool, template, true, true).await;

    let error = connect_async(request(&base)).await.unwrap_err();
    assert_eq!(rejected_status(error), StatusCode::SERVICE_UNAVAILABLE);
    task.abort();
}

#[tokio::test]
async fn a_default_template_bound_to_a_disabled_provider_fails_closed() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    let template = insert_template(&pool, "primary", TEMPLATE_PROMPT_V1, true).await;
    bind(&pool, template, &full_bindings("test")).await;
    assign(&pool, template, true, true).await;
    sqlx::query("UPDATE providers SET enabled = 0 WHERE key = 'test'")
        .execute(&pool)
        .await
        .unwrap();

    let error = connect_async(request(&base)).await.unwrap_err();
    assert_eq!(rejected_status(error), StatusCode::SERVICE_UNAVAILABLE);
    task.abort();
}

#[tokio::test]
async fn database_backed_admission_uses_the_default_template_prompt_and_language() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    let template = insert_template(&pool, "primary", TEMPLATE_PROMPT_V1, true).await;
    bind(&pool, template, &full_bindings("test")).await;
    assign(&pool, template, true, true).await;

    let mut socket = admit(&base).await;
    assert_eq!(
        speak_once(&mut socket).await,
        format!("default-llm[{TEMPLATE_PROMPT_V1}]")
    );
    task.abort();
}

#[tokio::test]
async fn a_default_template_binding_selects_a_loaded_runtime_other_than_the_server_default() {
    let (base, url, task) = start_with_template_llm(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    let template = insert_template(&pool, "primary", TEMPLATE_PROMPT_V1, true).await;
    bind(&pool, template, &full_bindings("db-llm")).await;
    assign(&pool, template, true, true).await;

    let mut socket = admit(&base).await;
    assert_eq!(
        speak_once(&mut socket).await,
        "template-llm[TEMPLATE-PROMPT-V1]",
        "the stored binding must select the runtime it names, not the server default"
    );
    task.abort();
}

#[tokio::test]
async fn an_admitted_profile_stays_immutable_while_a_new_session_sees_the_mutation() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    let template = insert_template(&pool, "primary", TEMPLATE_PROMPT_V1, true).await;
    bind(&pool, template, &full_bindings("test")).await;
    assign(&pool, template, true, true).await;

    let mut admitted = admit(&base).await;
    assert_eq!(
        speak_once(&mut admitted).await,
        "default-llm[TEMPLATE-PROMPT-V1]"
    );

    sqlx::query("UPDATE agent_templates SET prompt = ? WHERE key = 'primary'")
        .bind(TEMPLATE_PROMPT_V2)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE providers SET revision = revision + 1 WHERE key = 'test'")
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(
        speak_once(&mut admitted).await,
        "default-llm[TEMPLATE-PROMPT-V1]",
        "an admitted session must never be reconfigured by a database mutation"
    );
    let mut reconnected = admit(&base).await;
    assert_eq!(
        speak_once(&mut reconnected).await,
        "default-llm[TEMPLATE-PROMPT-V2]",
        "a new session resolves the mutated desired state"
    );
    task.abort();
}

#[tokio::test]
async fn an_invalid_non_default_candidate_is_excluded_while_a_valid_default_still_starts() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    let default_template = insert_template(&pool, "primary", TEMPLATE_PROMPT_V1, true).await;
    bind(&pool, default_template, &full_bindings("test")).await;
    assign(&pool, default_template, true, true).await;
    // A non-default candidate bound to a provider this process never loaded must be excluded,
    // not allowed to fail the session that already has a valid default.
    let broken = insert_template(&pool, "secondary", TEMPLATE_PROMPT_V2, true).await;
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
    task.abort();
}

#[tokio::test]
async fn a_disabled_device_is_still_denied_before_profile_resolution() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    sqlx::query("UPDATE devices SET enabled = 0")
        .execute(&pool)
        .await
        .unwrap();

    let error = connect_async(request(&base)).await.unwrap_err();
    assert_eq!(rejected_status(error), StatusCode::FORBIDDEN);
    task.abort();
}

#[tokio::test]
async fn a_closed_database_pool_degrades_admission_to_coarse_503() {
    let url = database_url();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app_config = with_server_default_prompt(config(address, url.clone()));
    let database = Database::connect(&app_config.database).await.unwrap();
    database.pool().close().await;
    let state = AppState::from_provider_set_with_database(
        app_config,
        Arc::new(ProviderSet::with_all(
            Arc::new(SilentVad),
            Arc::new(FinalAsr),
            Arc::new(EchoLlm {
                label: "default-llm",
            }),
            Arc::new(ShortTts),
        )),
        Some(database),
    );
    let task = tokio::spawn(async move {
        axum::serve(listener, voice_agent_server::app::router_with_state(state))
            .await
            .unwrap()
    });
    let error = connect_async(request(&format!("http://{address}")))
        .await
        .unwrap_err();
    assert_eq!(rejected_status(error), StatusCode::SERVICE_UNAVAILABLE);
    task.abort();
}

#[tokio::test]
async fn concurrent_sessions_each_resolve_their_own_immutable_profile() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    sqlx::query("INSERT INTO devices (device_id,agent_id,enabled,created_at,updated_at) VALUES ('second',1,1,1,1)")
        .execute(&pool)
        .await
        .unwrap();
    let template = insert_template(&pool, "primary", TEMPLATE_PROMPT_V1, true).await;
    bind(&pool, template, &full_bindings("test")).await;
    assign(&pool, template, true, true).await;

    let mut first = admit(&base).await;
    let mut second = admit_device(&base, "second", false).await;
    let (left, right) = tokio::join!(speak_once(&mut first), speak_once(&mut second));
    assert_eq!(left, "default-llm[TEMPLATE-PROMPT-V1]");
    assert_eq!(right, "default-llm[TEMPLATE-PROMPT-V1]");
    task.abort();
}
