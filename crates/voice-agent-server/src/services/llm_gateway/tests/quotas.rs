use super::*;

#[test]
fn vendor_quota_payload_parsers_match_9router_formats() {
    use super::super::vendors::{
        anthropic::parse_anthropic_usage_payload, gemini::parse_gemini_quota_payload,
        github_copilot::parse_github_copilot_usage_payload,
        openai_codex::parse_codex_usage_payload,
    };
    use serde_json::json;

    // 1. OpenAI Codex (`wham/usage`)
    let codex_json = json!({
        "plan_type": "plus",
        "rate_limit": {
            "limit_reached": false,
            "primary_window": {
                "used_percent": 25.0,
                "reset_at": 1760003600
            },
            "secondary_window": {
                "used_percent": 60.0,
                "reset_at": "2026-10-30T00:00:00Z"
            }
        }
    });
    let codex_quota = parse_codex_usage_payload(&codex_json);
    assert_eq!(codex_quota.plan.as_deref(), Some("plus"));
    assert!(!codex_quota.limit_reached);
    assert_eq!(codex_quota.windows.len(), 2);
    assert_eq!(codex_quota.windows[0].name, "5h");
    assert_eq!(codex_quota.windows[0].remaining_percentage, 75.0);
    assert_eq!(codex_quota.windows[0].reset_at, Some(1760003600));
    assert_eq!(codex_quota.windows[1].name, "Weekly");
    assert_eq!(codex_quota.windows[1].remaining_percentage, 40.0);

    // 2. Anthropic Claude (`/api/oauth/usage?cedar_ember=1`)
    let claude_json = json!({
        "five_hour": {
            "utilization": 42.5,
            "resets_at": "2026-10-28T18:00:00Z"
        },
        "seven_day": {
            "utilization": 100.0,
            "resets_at": "2026-11-01T00:00:00Z"
        },
        "seven_day_opus": {
            "utilization": 80.0,
            "resets_at": "2026-11-01T00:00:00Z"
        }
    });
    let claude_quota = parse_anthropic_usage_payload(&claude_json);
    assert_eq!(claude_quota.plan.as_deref(), Some("Claude Code"));
    assert!(claude_quota.limit_reached);
    assert_eq!(claude_quota.windows.len(), 3);
    assert_eq!(claude_quota.windows[0].name, "Session (5h)");
    assert_eq!(claude_quota.windows[0].remaining_percentage, 57.5);
    assert_eq!(claude_quota.windows[1].name, "Weekly (7d)");
    assert_eq!(claude_quota.windows[1].remaining_percentage, 0.0);

    // 3. Google Gemini (`retrieveUserQuota`)
    let gemini_json = json!({
        "buckets": [
            {
                "modelId": "gemini-2.5-pro",
                "remainingFraction": 0.85,
                "resetTime": "2026-10-29T00:00:00Z"
            },
            {
                "modelId": "gemini-2.5-flash",
                "remainingFraction": 0.5,
                "resetTime": "2026-10-29T00:00:00Z"
            }
        ]
    });
    let gemini_quota = parse_gemini_quota_payload(
        &gemini_json,
        Some("Standard".to_owned()),
        Some("proj-gemini-123".to_owned()),
    );
    assert_eq!(gemini_quota.plan.as_deref(), Some("Standard"));
    assert!(!gemini_quota.limit_reached);
    assert_eq!(gemini_quota.windows.len(), 2);
    assert_eq!(gemini_quota.windows[0].name, "gemini-2.5-pro");
    assert_eq!(gemini_quota.windows[0].used, 150.0);
    assert_eq!(gemini_quota.windows[0].total, 1000.0);
    assert_eq!(gemini_quota.windows[0].remaining_percentage, 85.0);
    assert_eq!(
        gemini_quota.discovered_account_context_id.as_deref(),
        Some("proj-gemini-123")
    );

    // 4. GitHub Copilot (`copilot_internal/user`)
    let copilot_json = json!({
        "copilot_plan": "individual_pro",
        "quota_reset_date": "2026-11-01",
        "quota_snapshots": {
            "chat": {
                "entitlement": 0,
                "remaining": 0,
                "unlimited": true
            },
            "premium_interactions": {
                "entitlement": 300,
                "remaining": 210,
                "unlimited": false
            }
        }
    });
    let copilot_quota = parse_github_copilot_usage_payload(&copilot_json);
    assert_eq!(copilot_quota.plan.as_deref(), Some("individual_pro"));
    assert!(!copilot_quota.limit_reached);
    assert_eq!(copilot_quota.windows.len(), 2);
    assert!(copilot_quota.windows[0].unlimited);
    assert_eq!(copilot_quota.windows[0].remaining_percentage, 100.0);
    assert_eq!(copilot_quota.windows[1].name, "Premium Requests");
    assert_eq!(copilot_quota.windows[1].used, 90.0);
    assert_eq!(copilot_quota.windows[1].total, 300.0);
    assert_eq!(copilot_quota.windows[1].remaining_percentage, 70.0);
}

#[tokio::test]
async fn hub_fetches_caches_and_routes_around_exhausted_account_quotas() {
    let (hub, drivers, _dir) = test_hub_with_fakes().await;
    let codex_fake = drivers.get(&GatewayVendor::OpenaiCodex).unwrap();

    // Create 2 OpenAI Codex accounts in the pool
    let ch1 = hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::OpenaiCodex,
            label: "Codex A".to_owned(),
            redirect_uri: None,
        })
        .await
        .unwrap();
    let st1 = url::Url::parse(ch1.authorize_url.as_deref().unwrap())
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    let (_, acc1) = hub.complete_oauth_by_state(&st1, "code_q1").await.unwrap();

    let ch2 = hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::OpenaiCodex,
            label: "Codex B".to_owned(),
            redirect_uri: None,
        })
        .await
        .unwrap();
    let st2 = url::Url::parse(ch2.authorize_url.as_deref().unwrap())
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    let (_, acc2) = hub.complete_oauth_by_state(&st2, "code_q2").await.unwrap();

    // Mark acc1 as quota limit_reached = true, acc2 as limit_reached = false
    codex_fake.set_next_quota(Some(
        crate::services::llm_gateway::types::VendorQuotaSnapshot {
            plan: Some("plus".to_owned()),
            limit_reached: true,
            windows: Vec::new(),
            message: Some("5h limit reached".to_owned()),
            discovered_account_context_id: None,
        },
    ));
    let q1 = hub
        .get_account_quota(&acc1.account_key, true)
        .await
        .unwrap();
    assert!(q1.limit_reached);
    assert_eq!(codex_fake.quota_call_count(), 1);

    // Cached call (`force = false`) does not hit upstream driver again
    let _ = hub
        .get_account_quota(&acc1.account_key, false)
        .await
        .unwrap();
    assert_eq!(codex_fake.quota_call_count(), 1);

    codex_fake.set_next_quota(None);
    let q2 = hub
        .get_account_quota(&acc2.account_key, true)
        .await
        .unwrap();
    assert!(!q2.limit_reached);
    assert_eq!(q2.windows.len(), 2);

    // Pool routing (`pool:openai_codex`) automatically skips `acc1` (limit_reached) and picks `acc2`
    let lease_a = hub
        .acquire_lease(&GatewayBinding::Pool(GatewayVendor::OpenaiCodex))
        .unwrap();
    let lease_b = hub
        .acquire_lease(&GatewayBinding::Pool(GatewayVendor::OpenaiCodex))
        .unwrap();
    assert_eq!(lease_a.account_key, acc2.account_key);
    assert_eq!(lease_b.account_key, acc2.account_key);
}
