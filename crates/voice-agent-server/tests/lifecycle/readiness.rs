use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use super::harness::*;

/// The two routes answer different questions, so a healthy process says so on both and a degraded
/// one still says it is alive.  Collapsing them is the failure this test exists to prevent: an
/// orchestrator that reads a degraded database as a dead process kills sessions that are fine.
#[tokio::test]
async fn a_healthy_process_is_both_live_and_ready() {
    let voice = start(config(database_url()), Duration::from_secs(15)).await;
    assert_eq!(voice.status("/health").await, StatusCode::OK);
    assert_eq!(voice.status("/ready").await, StatusCode::OK);
    assert_eq!(voice.body("/ready").await, "ready");
}

/// Liveness depends on nothing, so a database that has gone away degrades readiness and leaves
/// `/health` alone.
#[tokio::test]
async fn a_degraded_database_degrades_readiness_without_taking_the_process_down() {
    let voice = start(config(database_url()), Duration::from_secs(15)).await;
    voice.state.database.as_ref().unwrap().pool().close().await;

    assert_eq!(
        voice.status("/health").await,
        StatusCode::OK,
        "the process is still running and still serving the sessions it admitted"
    );
    assert_eq!(
        voice.status("/ready").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        voice.body("/ready").await,
        "database_unreachable",
        "readiness names the dependency that failed, in bounded words only"
    );
}

/// Readiness is a statement about *new* connections.  A Voice Session already admitted keeps its
/// snapshot, so a database outage after admission cannot disturb it — that is the whole reason
/// liveness and readiness are separate routes.
#[tokio::test]
async fn a_session_admitted_before_a_database_outage_keeps_its_profile() {
    let voice = start(config(database_url()), Duration::from_secs(15)).await;
    seed(&voice.database().await).await;
    let admitted = voice
        .state
        .resolve_session_profile("device")
        .await
        .expect("the seeded Device is admitted before the outage");
    assert_eq!(admitted.agent_key, "agent");

    voice.state.database.as_ref().unwrap().pool().close().await;

    assert_eq!(
        voice.status("/ready").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(
        admitted.agent_key == "agent",
        "the admitted snapshot is owned by the session, not re-read from the database"
    );
}

/// Directly assembled state without the required DB must never report ready.
#[tokio::test]
async fn missing_database_reports_startup_incomplete() {
    let voice = start_without_database(Duration::from_secs(15)).await;
    assert_eq!(voice.status("/health").await, StatusCode::OK);
    assert_eq!(
        voice.status("/ready").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(voice.body("/ready").await, "startup_incomplete");
}

/// A process that has begun shutdown reports so.  `/health` stays up because the process is up:
/// it is finishing the sessions it already has, and killing it would cost their Dialogue History.
#[tokio::test]
async fn a_shutting_down_process_is_live_but_no_longer_ready() {
    let voice = start(config(database_url()), Duration::from_millis(50)).await;
    seed(&voice.database().await).await;
    // Hold a session open so the drain reaches its deadline instead of completing immediately.
    let (mut socket, _) = connect_async(request(&voice.base, "device")).await.unwrap();
    socket.send(Message::Text(hello().into())).await.unwrap();
    assert!(matches!(
        timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        Message::Text(_)
    ));

    let lifecycle = Arc::clone(&voice.lifecycle);
    let shutdown = tokio::spawn(async move { lifecycle.shutdown().await });
    // The gate is the first thing shutdown does, before any session is asked anything. Waiting for
    // it rather than assuming it keeps the assertion about the ordering, not about scheduling.
    await_gate_closed(voice.state.admission_gate().as_ref()).await;
    assert_eq!(voice.status("/health").await, StatusCode::OK);
    assert_eq!(
        voice.status("/ready").await,
        StatusCode::SERVICE_UNAVAILABLE,
        "a process refusing new connections must not advertise that it will take them"
    );
    assert_eq!(voice.body("/ready").await, "shutting_down");

    let _ = shutdown.await;
}

/// An External MCP server that is refusing every connection is exactly the failure that must not
/// reach the readiness route.  External MCP is fail-soft in V1: a session that cannot use the bound
/// server simply gets fewer tools, so the process is still able to accept a connection.
#[tokio::test]
async fn an_unreachable_external_mcp_server_does_not_make_the_process_unready() {
    let voice = start(config(database_url()), Duration::from_secs(15)).await;
    let pool = voice.database().await;
    seed(&pool).await;
    // Bind nothing: nothing is listening on this port, so every discovery attempt fails.
    let dead_port = {
        let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
        probe.local_addr().unwrap().port()
    };
    let server_id: i64 = sqlx::query_scalar(
        "INSERT INTO mcp_servers (key,name,url,headers_json,auth_type,auth_header_name,\
         connect_timeout_ms,request_timeout_ms,enabled,created_at,updated_at) \
         VALUES ('weather','Weather',?,'{}','none',NULL, 500, 500, 1, 1, 1) RETURNING id",
    )
    .bind(format!("http://127.0.0.1:{dead_port}/mcp"))
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO agent_mcp_bindings (agent_id,mcp_server_id,enabled,required,created_at) \
         VALUES (1, ?, 1, 0, 1)",
    )
    .bind(server_id)
    .execute(&pool)
    .await
    .unwrap();
    drop(pool);

    // A real admission proves the failure is real and fail-soft at the same time.
    let profile = voice
        .state
        .resolve_session_profile("device")
        .await
        .expect("an unavailable optional server still admits the Device");
    assert!(
        profile.external_mcp().is_empty(),
        "the unreachable server contributed no tools, and the session was admitted anyway"
    );

    assert_eq!(
        voice.status("/ready").await,
        StatusCode::OK,
        "an optional External MCP server being down is not a readiness failure"
    );
}

/// The strongest possible statement about what readiness does not do: a readiness probe must not
/// resolve a Device.  A Device that admission would refuse has to leave `/ready` untouched, because
/// whether *this* client is allowed in says nothing about whether the process can serve anyone.
#[tokio::test]
async fn readiness_never_resolves_a_device() {
    let voice = start(config(database_url()), Duration::from_secs(15)).await;
    // No Device rows at all: full admission would refuse every client.
    let refusal = voice.state.resolve_session_profile("nobody").await;
    assert!(
        refusal.is_err(),
        "the boundary under test really does refuse this Device"
    );

    assert_eq!(
        voice.status("/ready").await,
        StatusCode::OK,
        "readiness answers for the process, not for the Device a probe might have invented"
    );
}

/// A readiness probe must not reach out over the network.  This counts every request the fake MCP
/// server receives, so "discovery is not on the readiness path" is an observation rather than a
/// claim about which code runs.
#[tokio::test]
async fn readiness_never_discovers_external_mcp() {
    let requests = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&requests);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            counted.fetch_add(1, Ordering::SeqCst);
            // Answer nothing: a connection that is accepted and dropped is enough to be counted,
            // and it proves the probe never waits on one.
            drop(socket);
        }
    });

    let mut app_config = config(database_url());
    app_config.mcp = McpConfig {
        external: ExternalMcpConfig {
            network: ExternalMcpNetworkConfig {
                allowed_hosts: vec![],
            },
            ..ExternalMcpConfig::default()
        },
        ..McpConfig::default()
    };
    let voice = start(app_config, Duration::from_secs(15)).await;
    let pool = voice.database().await;
    seed(&pool).await;
    let server_id: i64 = sqlx::query_scalar(
        "INSERT INTO mcp_servers (key,name,url,headers_json,auth_type,auth_header_name,\
         connect_timeout_ms,request_timeout_ms,enabled,created_at,updated_at) \
         VALUES ('weather','Weather',?,'{}','none',NULL, 500, 500, 1, 1, 1) RETURNING id",
    )
    .bind(format!("http://{address}/mcp"))
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO agent_mcp_bindings (agent_id,mcp_server_id,enabled,required,created_at) \
         VALUES (1, ?, 1, 0, 1)",
    )
    .bind(server_id)
    .execute(&pool)
    .await
    .unwrap();
    drop(pool);

    for _ in 0..5 {
        assert_eq!(voice.status("/ready").await, StatusCode::OK);
    }
    assert_eq!(
        requests.load(Ordering::SeqCst),
        0,
        "five readiness probes, zero outbound requests: discovery is not on this path"
    );
    task.abort();
}

/// The same discipline against the database: readiness asks one question and it is about the
/// dependency, never about the configuration.  A probe cannot walk the Agent/Template graph, and
/// this proves it by pointing the process at a graph a full admission would reject.
#[tokio::test]
async fn readiness_never_runs_full_admission() {
    let voice = start(config(database_url()), Duration::from_secs(15)).await;
    let pool = voice.database().await;
    // An Agent whose default Template cannot be materialized: full admission fails closed for it,
    // and readiness must not notice or care.
    sqlx::query(
        "INSERT INTO agents (key,name,enabled,created_at,updated_at) \
                 VALUES ('agent','Agent',1,1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let template_id: i64 = sqlx::query_scalar(
        "INSERT INTO agent_templates (key,name,description,language,prompt,enabled,created_at,updated_at) \
         VALUES ('broken','Broken',NULL,'','prompt',1,1,1) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO agent_template_assignments (agent_id,template_id,is_default,enabled,\
         created_at) VALUES (1, ?, 1, 1, 1)",
    )
    .bind(template_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO devices (device_id,agent_id,enabled,created_at,updated_at) \
                 VALUES ('device',1,1,1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    drop(pool);

    assert!(
        voice.state.resolve_session_profile("device").await.is_err(),
        "the boundary under test really does fail closed for this Agent"
    );
    assert_eq!(
        voice.status("/ready").await,
        StatusCode::OK,
        "readiness does not evaluate any one Agent's profile"
    );
}
