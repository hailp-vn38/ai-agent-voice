use super::tests::fixture;
use super::*;
use crate::database::{Database, mcp_servers::McpInput};
use std::time::Duration;

async fn grant_management(db: &Database, ctx: &mut SystemToolContext) {
    ctx.grants.mcp_manage_request = true;
    ctx.grants.mcp_discover = true;
    db.save_system_mcp_grants(true, ctx.grants, 1, "grant")
        .await
        .unwrap();
}
async fn server(db: &Database) {
    db.create_mcp_server(
        McpInput {
            key: "weather",
            name: "Weather",
            url: "http://localhost/mcp",
            auth_type: "none",
            auth_header_name: None,
            credential: None,
            connect_timeout_ms: 1000,
            request_timeout_ms: 1000,
        },
        "create",
    )
    .await
    .unwrap();
}
async fn proposal(core: &SystemMcp, ctx: &SystemToolContext, revision: i64) -> String {
    core.call_tool(
        ctx,
        "mcp_request_server_change",
        json!({"server_key":"weather","enabled":false,"expected_revision":revision}),
    )
    .await
    .unwrap()["id"]
        .as_str()
        .unwrap()
        .into()
}
#[tokio::test]
async fn grants_seed_once_catalog_additions_wait_and_live_revoke_denies() {
    let (core, mut ctx, db) = fixture(true, 8192).await;
    let initial = db.system_mcp_grants(true).await.unwrap();
    assert!(initial.grants.system_read && initial.grants.mcp_read);
    assert!(!initial.grants.mcp_discover && !initial.grants.mcp_manage_request);
    let mut next = initial.grants;
    next.mcp_discover = true;
    db.save_system_mcp_grants(true, next, 1, "add")
        .await
        .unwrap();
    assert!(
        !core
            .list_tools(&ctx)
            .iter()
            .any(|t| t.name == "mcp_discover_tools")
    );
    assert!(
        db.save_system_mcp_grants(true, next, 1, "stale")
            .await
            .is_err()
    );
    let conflicts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM admin_audit_events WHERE request_id='stale' AND outcome='conflict'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(conflicts, 1);
    next.system_read = false;
    db.save_system_mcp_grants(true, next, 2, "revoke")
        .await
        .unwrap();
    assert!(
        !db.system_mcp_grants(false)
            .await
            .unwrap()
            .grants
            .system_read
    );
    assert_eq!(
        core.call_tool(&ctx, "system_get_status", json!({})).await,
        Err(SystemToolError::Denied)
    );
    ctx.grants.mcp_discover = true;
    assert_eq!(
        core.call_tool(
            &ctx,
            "mcp_discover_tools",
            json!({"server_key":"weather","approved":true})
        )
        .await,
        Err(SystemToolError::InvalidArguments)
    );
}
#[tokio::test]
async fn proposals_never_mutate_and_concurrent_confirmation_dispatches_once() {
    let (core, mut ctx, db) = fixture(true, 8192).await;
    grant_management(&db, &mut ctx).await;
    server(&db).await;
    let old = mcp_by(&db, "weather").await.unwrap();
    let id = proposal(&core, &ctx, old.revision).await;
    assert_eq!(mcp_by(&db, "weather").await.unwrap().enabled, old.enabled);
    assert_eq!(core.change_requests()[0].status, "pending");
    assert_eq!(core.call_tool(&ctx,"mcp_request_server_change",json!({"server_key":"weather","enabled":false,"expected_revision":old.revision,"confirmation":true})).await,Err(SystemToolError::InvalidArguments));
    let (one, two) = tokio::join!(
        core.confirm(&id, "confirm-a"),
        core.confirm(&id, "confirm-b")
    );
    assert_ne!(one.is_ok(), two.is_ok());
    assert_eq!(one.or(two).unwrap().status, "applied");
    let updated = mcp_by(&db, "weather").await.unwrap();
    assert_eq!(updated.enabled, 0);
    assert_eq!(updated.revision, old.revision + 1);
    assert!(core.confirm(&id, "retry").await.is_err());
}
#[tokio::test]
async fn stale_revision_revoked_grant_and_audit_failure_are_terminal_without_mutation() {
    for mode in ["stale", "revoked", "audit"] {
        let (core, mut ctx, db) = fixture(true, 8192).await;
        grant_management(&db, &mut ctx).await;
        server(&db).await;
        let old = mcp_by(&db, "weather").await.unwrap();
        let id = proposal(
            &core,
            &ctx,
            if mode == "stale" {
                old.revision + 1
            } else {
                old.revision
            },
        )
        .await;
        if mode == "revoked" {
            let mut grants = ctx.grants;
            grants.mcp_manage_request = false;
            db.save_system_mcp_grants(true, grants, 2, "revoke")
                .await
                .unwrap();
        }
        if mode == "audit" {
            sqlx::query("CREATE TRIGGER fail_system_audit BEFORE INSERT ON admin_audit_events BEGIN SELECT RAISE(ABORT,'audit failed'); END").execute(db.pool()).await.unwrap();
        }
        assert_eq!(
            core.confirm(&id, "confirm").await.unwrap().status,
            "failed",
            "{mode}"
        );
        let unchanged = mcp_by(&db, "weather").await.unwrap();
        assert_eq!(unchanged.revision, old.revision);
        assert_eq!(unchanged.enabled, old.enabled);
        assert!(core.confirm(&id, "retry").await.is_err());
    }
}
#[tokio::test]
async fn expired_and_rejected_requests_cannot_be_claimed_and_registry_is_bounded() {
    let (core, mut ctx, db) = fixture(true, 8192).await;
    grant_management(&db, &mut ctx).await;
    let id = proposal(&core, &ctx, 1).await;
    core.reject(&id).unwrap();
    assert!(core.confirm(&id, "confirm").await.is_err());
    for _ in 1..16 {
        proposal(&core, &ctx, 1).await;
    }
    assert_eq!(
        core.call_tool(
            &ctx,
            "mcp_request_server_change",
            json!({"server_key":"weather","enabled":false,"expected_revision":1})
        )
        .await,
        Err(SystemToolError::Unavailable)
    );
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(301)).await;
    assert!(core.confirm(&id, "expired").await.is_err());
    assert!(core.change_requests().is_empty());
    tokio::time::resume();
    let (fresh, _, _) = fixture(true, 8192).await;
    assert!(fresh.change_requests().is_empty());
}
#[tokio::test]
async fn remote_revocation_is_live_and_does_not_accept_admin_authority() {
    let (core, mut ctx, db) = fixture(true, 8192).await;
    let expires_at = crate::database::unix_seconds().unwrap() + 300;
    ctx.caller = SystemCaller::Remote {
        subject: "client".into(),
        jti: "first".into(),
        expires_at,
    };
    assert!(
        core.call_tool(&ctx, "system_get_status", json!({}))
            .await
            .is_ok()
    );
    assert_eq!(
        core.call_tool(&ctx, "mcp_list_tools", json!({})).await,
        Err(SystemToolError::InvalidArguments)
    );
    db.revoke_system_mcp_token("first", expires_at, "revoke")
        .await
        .unwrap();
    assert_eq!(
        core.call_tool(&ctx, "system_get_status", json!({})).await,
        Err(SystemToolError::Denied)
    );
}

#[tokio::test]
async fn binding_and_review_proposals_revalidate_revisions_and_fingerprints() {
    let (core, mut ctx, db) = fixture(true, 8192).await;
    grant_management(&db, &mut ctx).await;
    server(&db).await;
    let binding=core.call_tool(&ctx,"mcp_request_agent_binding_change",json!({"agent_key":"studio","server_key":"weather","enabled":true,"expected_agent_revision":1})).await.unwrap();
    let agent = get_agent_by(&db, "studio").await.unwrap();
    assert!(db.system_mcp_bindings(agent.id).await.unwrap().is_empty());
    assert_eq!(
        core.confirm(binding["id"].as_str().unwrap(), "confirm-binding")
            .await
            .unwrap()
            .status,
        "applied"
    );
    let source = mcp_by(&db, "weather").await.unwrap();
    let fingerprint = "a".repeat(64);
    sqlx::query("INSERT INTO external_tool_observations(server_id,original_name,description,input_schema,fingerprint,server_revision,observed_at) VALUES(?,'forecast','safe','{}',?,1,unixepoch())").bind(source.id).bind(&fingerprint).execute(db.pool()).await.unwrap();
    let review=core.call_tool(&ctx,"mcp_request_tool_review",json!({"agent_key":"studio","server_key":"weather","tool_name":"forecast","fingerprint":fingerprint,"observed_revision":1,"allow":true,"sensitive":false,"expected_revision":1})).await.unwrap();
    assert_eq!(
        db.system_mcp_tools(agent.id, Some("weather"), 1)
            .await
            .unwrap()[0]["allowed"],
        false
    );
    sqlx::query(
        "UPDATE external_tool_observations SET fingerprint=?,revision=revision+1 WHERE server_id=?",
    )
    .bind("b".repeat(64))
    .bind(source.id)
    .execute(db.pool())
    .await
    .unwrap();
    assert_eq!(
        core.confirm(review["id"].as_str().unwrap(), "confirm-stale-review")
            .await
            .unwrap()
            .status,
        "failed"
    );
    let new_review=core.call_tool(&ctx,"mcp_request_tool_review",json!({"agent_key":"studio","server_key":"weather","tool_name":"forecast","fingerprint":"b".repeat(64),"observed_revision":2,"allow":true,"sensitive":false,"expected_revision":1})).await.unwrap();
    assert_eq!(
        core.confirm(new_review["id"].as_str().unwrap(), "confirm-review")
            .await
            .unwrap()
            .status,
        "applied"
    );
    assert_eq!(
        db.system_mcp_tools(agent.id, Some("weather"), 1)
            .await
            .unwrap()[0]["allowed"],
        true
    );
}

#[tokio::test]
async fn proposals_reject_non_object_arguments_without_panicking() {
    let (core, mut ctx, db) = fixture(true, 8192).await;
    grant_management(&db, &mut ctx).await;
    for value in [json!([]), json!("text"), json!(true), json!(1), Value::Null] {
        assert_eq!(
            core.call_tool(&ctx, "mcp_request_server_change", value)
                .await,
            Err(SystemToolError::InvalidArguments)
        );
    }
}
#[tokio::test]
async fn short_supplied_expiry_cannot_shorten_remote_revocation() {
    let (_, _, db) = fixture(true, 8192).await;
    let now = crate::database::unix_seconds().unwrap();
    db.revoke_system_mcp_token("short", now + 1, "revoke")
        .await
        .unwrap();
    let retained: i64 =
        sqlx::query_scalar("SELECT expires_at FROM system_mcp_token_revocations WHERE jti='short'")
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert!(retained >= now + 3600);
}
