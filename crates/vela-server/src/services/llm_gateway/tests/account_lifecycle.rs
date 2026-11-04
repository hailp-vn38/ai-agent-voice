use super::*;

#[tokio::test]
async fn llm_gateway_proactive_refresh_and_zero_retry_cooldown_rotation() {
    let (hub, fakes, _temp) = test_hub_with_fakes().await;
    let anthropic_fake = fakes.get(&GatewayVendor::Anthropic).expect("anthropic");

    // Set near-expiry timestamp so proactive refresh triggers
    anthropic_fake.set_next_expires_at(Some(100));

    let ch1 = hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::Anthropic,
            label: "Claude #1".to_owned(),
            redirect_uri: None,
        })
        .await
        .expect("ch1");
    let acc1 = hub
        .complete_oauth_login(
            &ch1.session_id,
            OAuthCallbackSubmission {
                callback_url_or_code: "code_acc_1".to_owned(),
                state: None,
            },
        )
        .await
        .expect("acc1");

    let ch2 = hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::Anthropic,
            label: "Claude #2".to_owned(),
            redirect_uri: None,
        })
        .await
        .expect("ch2");
    let acc2 = hub
        .complete_oauth_login(
            &ch2.session_id,
            OAuthCallbackSubmission {
                callback_url_or_code: "code_acc_2".to_owned(),
                state: None,
            },
        )
        .await
        .expect("acc2");

    // Fire 4 concurrent proactive refreshes for acc1 -> single-flight lock ensures refresh_token runs once
    let mut handles = Vec::new();
    for _ in 0..4 {
        let h = Arc::clone(&hub);
        handles.push(tokio::spawn(async move {
            h.proactive_refresh_expiring(300).await
        }));
    }
    for handle in handles {
        let _ = handle.await.expect("join");
    }
    // Both acc1 and acc2 were expiring, so each should be refreshed exactly once (total = 2)
    assert_eq!(anthropic_fake.refresh_call_count(), 2);

    // Round-robin across the 2 active accounts in Pool(Anthropic)
    let pool_binding = GatewayBinding::Pool(GatewayVendor::Anthropic);
    let lease_a = hub.acquire_lease(&pool_binding).expect("lease a");
    let lease_b = hub.acquire_lease(&pool_binding).expect("lease b");
    assert_ne!(lease_a.account_key, lease_b.account_key);

    // Report 429 RateLimited on acc1 -> next pool acquisitions must skip acc1 and route to acc2
    hub.report_outcome(
        &acc1.account_key,
        GatewayCallOutcome::RateLimited {
            retry_after_secs: Some(60),
        },
    );
    assert_eq!(
        hub.acquire_lease(&GatewayBinding::Account(acc1.account_key.clone()))
            .unwrap_err(),
        GatewayError::AccountCoolingDown
    );
    for _ in 0..3 {
        let next_lease = hub.acquire_lease(&pool_binding).expect("pool lease");
        assert_eq!(next_lease.account_key, acc2.account_key);
    }

    // Clear cooldown on acc1 -> acc1 becomes available again
    let cleared = hub
        .clear_cooldown(&acc1.account_key)
        .await
        .expect("clear_cooldown");
    assert_eq!(cleared.cooldown_remaining_secs, 0);

    // Disable acc2 via update_account -> only acc1 remains active
    let updated = hub
        .update_account(
            &acc2.account_key,
            UpdateGatewayAccountPatch {
                label: Some("Claude #2 Paused".to_owned()),
                enabled: Some(false),
                priority: Some(200),
            },
        )
        .await
        .expect("update_account");
    assert!(!updated.enabled);
    assert_eq!(updated.status, GatewayAccountStatus::Disabled);
}
