use super::*;

use crate::services::llm_gateway::vendors::refresh_failure_for_status;

async fn login_expiring_anthropic(hub: &Arc<LlmGatewayHub>, fake: &FakeOAuthVendorDriver) -> String {
    fake.set_next_expires_at(Some(100));
    let challenge = hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::Anthropic,
            label: "Claude".to_owned(),
            redirect_uri: None,
        })
        .await
        .expect("start login");
    hub.complete_oauth_login(
        &challenge.session_id,
        OAuthCallbackSubmission {
            callback_url_or_code: "code".to_owned(),
            state: None,
        },
    )
    .await
    .expect("complete login")
    .account_key
}

async fn stored_status(db: &Database, account_key: &str) -> (String, Option<String>) {
    sqlx::query_as("SELECT status, last_error FROM llm_gateway_accounts WHERE account_key = ?")
        .bind(account_key)
        .fetch_one(db.pool())
        .await
        .expect("account row")
}

#[test]
fn refresh_status_classification_separates_rejected_grants_from_outages() {
    for status in [400, 401, 403] {
        assert_eq!(
            refresh_failure_for_status(status),
            GatewayError::AccountReauthRequired
        );
    }
    for status in [408, 429, 500, 502, 503] {
        assert!(matches!(
            refresh_failure_for_status(status),
            GatewayError::OAuthExchangeFailed(_)
        ));
    }
}

#[tokio::test]
async fn transient_refresh_failure_keeps_account_active_and_recovers() {
    let (hub, fakes, db) = test_hub_with_fakes().await;
    let fake = fakes.get(&GatewayVendor::Anthropic).expect("anthropic");
    let key = login_expiring_anthropic(&hub, fake).await;

    // The first pass after a restart runs with the network still down.
    fake.set_next_refresh_error(Some(GatewayError::OAuthExchangeFailed(
        "connection refused".to_owned(),
    )));
    assert_eq!(hub.proactive_refresh_expiring(300).await.expect("pass"), 0);
    assert_eq!(stored_status(&db, &key).await.0, "active");

    // A restart reads the row back as still active, and the next pass renews it.
    fake.set_next_refresh_error(None);
    hub.clear_cooldown(&key).await.expect("clear cooldown");
    hub.reload_snapshot().await.expect("reload");
    assert_eq!(hub.proactive_refresh_expiring(300).await.expect("pass"), 1);
    hub.acquire_lease(&GatewayBinding::Account(key.clone()))
        .expect("lease after recovery");
}

#[tokio::test]
async fn rejected_grant_requires_reauth_and_is_not_retried() {
    let (hub, fakes, db) = test_hub_with_fakes().await;
    let fake = fakes.get(&GatewayVendor::Anthropic).expect("anthropic");
    let key = login_expiring_anthropic(&hub, fake).await;

    fake.set_next_refresh_error(Some(GatewayError::AccountReauthRequired));
    assert_eq!(hub.proactive_refresh_expiring(300).await.expect("pass"), 0);
    assert_eq!(
        stored_status(&db, &key).await,
        (
            "reauth_required".to_owned(),
            Some("gateway_account_reauth_required".to_owned())
        )
    );

    let calls = fake.refresh_call_count();
    fake.set_next_refresh_error(None);
    assert_eq!(hub.proactive_refresh_expiring(300).await.expect("pass"), 0);
    assert_eq!(fake.refresh_call_count(), calls);
}

#[tokio::test]
async fn account_parked_by_an_earlier_outage_heals_on_the_next_pass() {
    let (hub, fakes, db) = test_hub_with_fakes().await;
    let fake = fakes.get(&GatewayVendor::Anthropic).expect("anthropic");
    let key = login_expiring_anthropic(&hub, fake).await;

    sqlx::query(
        "UPDATE llm_gateway_accounts \
         SET status = 'reauth_required', last_error = 'oauth_exchange_failed' \
         WHERE account_key = ?",
    )
    .bind(&key)
    .execute(db.pool())
    .await
    .expect("park account");
    hub.reload_snapshot().await.expect("reload");
    assert_eq!(
        hub.acquire_lease(&GatewayBinding::Account(key.clone()))
            .unwrap_err(),
        GatewayError::AccountReauthRequired
    );

    assert_eq!(hub.proactive_refresh_expiring(300).await.expect("pass"), 1);
    assert_eq!(stored_status(&db, &key).await, ("active".to_owned(), None));
    hub.acquire_lease(&GatewayBinding::Account(key))
        .expect("lease after heal");
}
