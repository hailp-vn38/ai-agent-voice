//! Admin HTTP router wiring across all admin feature modules.

use super::{
    AppState, MAX_ASR_TEST_BODY, agents, authenticate, background, deletion, devices, history,
    knowledge, llm_gateway, mcp, notifications, providers, request_id, speakers, system, templates,
    transport,
};
use axum::{
    Router,
    routing::{get, put},
};

pub(in crate::app) fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .merge(knowledge_routes())
        .merge(background_routes())
        .merge(notification_routes())
        .merge(mcp_routes())
        .merge(system_routes())
        .merge(agent_and_template_routes())
        .merge(provider_and_gateway_routes())
        .merge(device_and_history_routes())
        .merge(speaker_routes())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            authenticate,
        ))
        .layer(axum::middleware::from_fn(transport))
        .layer(axum::middleware::from_fn(request_id))
        .with_state(state)
}

fn knowledge_routes() -> Router<AppState> {
    Router::new()
        // Knowledge and Memory are two authenticated Admin workspaces, not Plugin Store data.
        .route(
            "/knowledge/pages/{id}/draft-resolution",
            axum::routing::post(knowledge::processing::resolve_draft),
        )
        .route(
            "/knowledge/sources/{id}/analyze",
            axum::routing::post(knowledge::processing::analyze),
        )
        .route("/knowledge/reviews", get(knowledge::processing::reviews))
        .route("/knowledge/reviews/{id}", get(knowledge::processing::review))
        .route(
            "/knowledge/reviews/{id}/approve",
            axum::routing::post(knowledge::processing::approve),
        )
        .route(
            "/knowledge/reviews/{id}/reject",
            axum::routing::post(knowledge::processing::reject),
        )
        .route("/knowledge/tree", get(knowledge::knowledge_tree))
        .route("/knowledge/graph", get(knowledge::knowledge_graph))
        .route(
            "/knowledge/charts/{*path}",
            get(knowledge::knowledge_chart_read),
        )
        .route(
            "/knowledge/folders",
            axum::routing::post(knowledge::knowledge_folder_create),
        )
        .route(
            "/knowledge/raw/files",
            get(knowledge::knowledge_raw_read)
                .post(knowledge::knowledge_raw_create)
                .put(knowledge::knowledge_raw_update),
        )
        .route(
            "/knowledge/raw/import-source",
            axum::routing::post(knowledge::knowledge_raw_import),
        )
        .route(
            "/knowledge/raw/move",
            axum::routing::post(knowledge::knowledge_raw_move),
        )
        .route(
            "/knowledge/pages/{id}/move",
            axum::routing::post(knowledge::knowledge_page_move),
        )
        .route(
            "/knowledge/pages",
            get(knowledge::knowledge_search).post(knowledge::knowledge_create),
        )
        .route(
            "/knowledge/pages/{id}",
            get(knowledge::knowledge_read).put(knowledge::knowledge_update),
        )
        .route(
            "/knowledge/pages/{id}/publish",
            axum::routing::post(knowledge::knowledge_publish),
        )
        .route(
            "/knowledge/sources",
            get(knowledge::knowledge_sources).post(knowledge::knowledge_import_source),
        )
        .route(
            "/knowledge/sources/{id}",
            get(knowledge::knowledge_read_source),
        )
        .route("/knowledge/index", get(knowledge::knowledge_health))
        .route(
            "/knowledge/index/rebuild",
            axum::routing::post(knowledge::knowledge_rebuild),
        )
        .route(
            "/memory/facts",
            get(knowledge::memory_recall).post(knowledge::memory_create),
        )
        .route(
            "/memory/facts/{id}",
            get(knowledge::memory_read).put(knowledge::memory_update),
        )
        .route(
            "/memory/facts/{id}/forget",
            axum::routing::post(knowledge::memory_forget),
        )
        .route("/memory/summary", get(knowledge::memory_summary))
        .route("/memory/index", get(knowledge::memory_health))
        .route(
            "/memory/index/rebuild",
            axum::routing::post(knowledge::memory_rebuild),
        )
}

fn background_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/background/settings",
            get(background::settings).put(background::save_settings),
        )
        .route(
            "/background/agents",
            axum::routing::post(background::create_agent),
        )
        .route(
            "/background/agents/{id}",
            axum::routing::delete(background::delete_agent),
        )
        .route(
            "/background/skills/{id}",
            get(background::get_skill)
                .put(background::save_skill)
                .delete(background::delete_skill),
        )
        .route("/artifacts", get(background::list_artifacts))
        .route("/jobs", get(background::list).post(background::create))
        .route(
            "/jobs/batch-delete",
            axum::routing::post(background::batch_delete),
        )
        .route("/jobs/stream", get(background::stream))
        .route(
            "/jobs/{id}",
            get(background::get).delete(background::delete),
        )
        .route("/jobs/{id}/events", get(background::events))
        .route("/jobs/{id}/history", get(background::history))
        .route(
            "/jobs/{id}/followup",
            axum::routing::post(background::followup),
        )
        .route(
            "/jobs/{id}/artifacts/{artifact_id}",
            get(background::artifact),
        )
        .route("/jobs/{id}/cancel", axum::routing::post(background::cancel))
        .route("/jobs/{id}/retry", axum::routing::post(background::retry))
}

fn notification_routes() -> Router<AppState> {
    Router::new()
        .route("/notifications", get(notifications::hub::list_inbox))
        .route(
            "/notifications/read",
            axum::routing::post(notifications::hub::mark_read),
        )
        .route(
            "/notifications/purge",
            axum::routing::post(notifications::hub::purge_inbox),
        )
        .route("/notifications/stream", get(notifications::hub::stream))
        .route(
            "/notifications/settings",
            get(notifications::hub::settings).put(notifications::hub::save_settings),
        )
        .route(
            "/notifications/channels/{channel}/test",
            axum::routing::post(notifications::hub::test_channel),
        )
        .route(
            "/notifications/{id}",
            axum::routing::delete(notifications::hub::delete_item),
        )
        .route(
            "/agents/{key}/reminders",
            get(notifications::reminders::agent_list),
        )
        .route(
            "/agents/{key}/devices/{device_id}/reminders",
            get(notifications::reminders::scoped_list)
                .post(notifications::reminders::scoped_create),
        )
        .route(
            "/agents/{key}/devices/{device_id}/reminders/{reminder_id}",
            axum::routing::patch(notifications::reminders::scoped_update),
        )
        .route(
            "/agents/{key}/devices/{device_id}/reminders/{reminder_id}/cancel",
            axum::routing::post(notifications::reminders::scoped_cancel),
        )
        .route(
            "/devices/{device_id}/reminders/{reminder_id}",
            axum::routing::patch(notifications::reminders::update),
        )
        .route(
            "/devices/{device_id}/reminders/{reminder_id}/cancel",
            axum::routing::post(notifications::reminders::cancel),
        )
        .route(
            "/devices/{device_id}/reminders",
            get(notifications::reminders::list).post(notifications::reminders::create),
        )
        .route(
            "/agents/{key}/reminder-policy",
            get(notifications::reminders::grant).patch(notifications::reminders::patch_grant),
        )
}

fn mcp_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/mcp-tests/connection",
            axum::routing::post(mcp::diagnostics::draft_connection),
        )
        .route(
            "/mcp-tests/discover",
            axum::routing::post(mcp::diagnostics::draft_discover),
        )
        .route(
            "/mcp-servers/{key}/test/connection",
            axum::routing::post(mcp::diagnostics::saved_connection),
        )
        .route(
            "/mcp-servers/{key}/test/discover",
            axum::routing::post(mcp::diagnostics::saved_discover),
        )
        .route(
            "/system-mcp/grants",
            get(mcp::system::grants).put(mcp::system::save_grants),
        )
        .route("/system-mcp/change-requests", get(mcp::system::requests))
        .route(
            "/system-mcp/change-requests/{id}/confirm",
            axum::routing::post(mcp::system::confirm),
        )
        .route(
            "/system-mcp/change-requests/{id}/reject",
            axum::routing::post(mcp::system::reject),
        )
        .route(
            "/system-mcp/token-revocations",
            axum::routing::post(mcp::system::revoke_token),
        )
        .route(
            "/system-mcp/knowledge",
            get(mcp::system::knowledge_settings).put(mcp::system::save_knowledge_settings),
        )
        .route(
            "/system-mcp/knowledge/tokens",
            axum::routing::post(mcp::system::create_knowledge_token),
        )
        .route(
            "/system-mcp/knowledge/tokens/{id}",
            axum::routing::delete(mcp::system::delete_knowledge_token),
        )
        .route(
            "/mcp-servers",
            get(mcp::list_mcp_servers).post(mcp::create_mcp_server),
        )
        .route(
            "/mcp-servers/{key}",
            get(mcp::get_mcp_server)
                .patch(mcp::patch_mcp_server)
                .delete(deletion::delete_mcp_server),
        )
        .route(
            "/agents/{key}/mcp-bindings",
            get(mcp::list_agent_mcp_bindings),
        )
        .route(
            "/agents/{key}/mcp-bindings/{server_key}",
            put(mcp::put_agent_mcp_binding).delete(mcp::unlink_agent_mcp_binding),
        )
        .route(
            "/agents/{key}/tool-allowlist",
            get(mcp::allowlist::list).put(mcp::allowlist::review),
        )
}

fn system_routes() -> Router<AppState> {
    Router::new()
        .route("/system", get(system::get_system))
        .route("/system/resources", get(system::get_resources))
        .route("/system/logs", get(system::get_logs))
        .route("/system/logs/stream", get(system::stream_logs))
        .route("/system/audit-logs", get(system::get_audit_logs))
        .route(
            "/system/assistant-device",
            get(devices::assistant::get).put(devices::assistant::put),
        )
        .route(
            "/system/assistant-device/reminder-target",
            put(devices::assistant::put_reminder_target),
        )
        .route(
            "/assistant/ws-ticket",
            axum::routing::post(devices::assistant::ticket),
        )
}

fn agent_and_template_routes() -> Router<AppState> {
    Router::new()
        .route("/agents", get(agents::list_agents).post(agents::create_agent))
        .route(
            "/agents/{key}",
            get(agents::get_agent)
                .patch(agents::patch_agent)
                .delete(deletion::delete_agent),
        )
        .route(
            "/agents/{key}/default-template/{template_key}",
            put(templates::set_default_template),
        )
        .route(
            "/agents/{key}/templates",
            get(templates::list_agent_templates),
        )
        .route(
            "/agents/{key}/templates/{template_key}",
            put(templates::assign_template).delete(templates::unlink_agent_template),
        )
        .route(
            "/templates",
            get(templates::list_templates).post(templates::create_template),
        )
        .route(
            "/templates/{key}",
            get(templates::get_template)
                .patch(templates::patch_template)
                .delete(deletion::delete_template),
        )
        .route("/templates/{key}/agents", get(templates::list_template_agents))
        .route(
            "/templates/{key}/providers",
            get(templates::list_template_providers),
        )
        .route(
            "/templates/{key}/providers/{provider_type}",
            put(templates::bind_template_provider).delete(templates::unlink_template_provider),
        )
}

fn provider_and_gateway_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/provider-tests/llm",
            axum::routing::post(providers::draft_tests::llm),
        )
        .route(
            "/provider-tests/tts",
            axum::routing::post(providers::draft_tests::tts),
        )
        .route(
            "/provider-tests/asr",
            axum::routing::post(providers::draft_tests::asr).layer(
                axum::extract::DefaultBodyLimit::max(MAX_ASR_TEST_BODY + 80 * 1024),
            ),
        )
        .route(
            "/providers",
            get(providers::list_providers).post(providers::create_provider),
        )
        .route(
            "/providers/{key}",
            get(providers::get_provider)
                .patch(providers::patch_provider)
                .delete(deletion::delete_provider),
        )
        .route(
            "/providers/{key}/templates",
            get(providers::list_provider_templates),
        )
        .route(
            "/providers/{key}/prepare",
            axum::routing::post(providers::prepare_provider),
        )
        .route(
            "/providers/{key}/test/llm",
            axum::routing::post(providers::test_llm_provider),
        )
        .route(
            "/providers/{key}/test/tts",
            axum::routing::post(providers::test_tts_provider),
        )
        .route(
            "/providers/{key}/test/asr",
            axum::routing::post(providers::test_asr_provider),
        )
        .route(
            "/providers/{key}/test/vad",
            axum::routing::post(providers::test_vad_provider),
        )
        .route(
            "/providers/{key}/capabilities",
            get(providers::get_provider_capabilities),
        )
        .route(
            "/provider-adapters",
            get(providers::list_provider_adapters),
        )
        .route(
            "/provider-adapters/{adapter}",
            get(providers::get_provider_adapter),
        )
        .route(
            "/provider-adapters/{adapter}/capabilities/discover",
            axum::routing::post(providers::discover_provider_capabilities),
        )
        .route("/llm-gateway/vendors", get(llm_gateway::list_vendors))
        .route(
            "/llm-gateway/pools/{vendor}",
            get(llm_gateway::get_pool_detail),
        )
        .route(
            "/llm-gateway/pools/{vendor}/models",
            axum::routing::post(llm_gateway::create_pool_model),
        )
        .route(
            "/llm-gateway/pools/{vendor}/models/{provider_key}",
            axum::routing::patch(llm_gateway::patch_pool_model)
                .delete(llm_gateway::delete_pool_model),
        )
        .route("/llm-gateway/accounts", get(llm_gateway::list_accounts))
        .route("/llm-gateway/quotas", get(llm_gateway::list_quotas))
        .route(
            "/llm-gateway/oauth/start",
            axum::routing::post(llm_gateway::start_oauth),
        )
        .route(
            "/llm-gateway/oauth/callback",
            get(llm_gateway::oauth_callback),
        )
        .route(
            "/llm-gateway/oauth/sessions/{session_id}/complete",
            axum::routing::post(llm_gateway::complete_oauth_session),
        )
        .route(
            "/llm-gateway/oauth/sessions/{session_id}/poll",
            axum::routing::post(llm_gateway::poll_oauth_session),
        )
        .route(
            "/llm-gateway/accounts/{account_key}/refresh",
            axum::routing::post(llm_gateway::refresh_account),
        )
        .route(
            "/llm-gateway/accounts/{account_key}/quota",
            get(llm_gateway::get_account_quota).post(llm_gateway::refresh_account_quota),
        )
        .route(
            "/llm-gateway/accounts/{account_key}/clear-cooldown",
            axum::routing::post(llm_gateway::clear_account_cooldown),
        )
        .route(
            "/llm-gateway/accounts/{account_key}",
            axum::routing::patch(llm_gateway::patch_account).delete(llm_gateway::delete_account),
        )
}

fn device_and_history_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/devices/{device_id}/speak",
            axum::routing::post(devices::speak),
        )
        .route(
            "/devices",
            get(devices::list_devices).post(devices::create_device),
        )
        .route(
            "/device-enrollments/claim",
            axum::routing::post(devices::enrollments::claim),
        )
        .route(
            "/devices/{device_id}",
            get(devices::get_device)
                .patch(devices::patch_device)
                .delete(deletion::delete_device),
        )
        .route("/history", get(history::list_history))
        .route("/history/purge", axum::routing::post(history::purge_history))
}

fn speaker_routes() -> Router<AppState> {
    Router::new()
        .route("/speaker-recognition", get(speakers::summary))
        .route("/speakers", get(speakers::list).post(speakers::create))
        .route(
            "/speakers/captures",
            axum::routing::post(speakers::quick::create_capture),
        )
        .route(
            "/speakers/from-capture",
            axum::routing::post(speakers::quick::create_speaker_from_capture),
        )
        .route(
            "/speakers/{key}",
            get(speakers::get)
                .patch(speakers::patch)
                .delete(speakers::delete),
        )
        .route(
            "/speakers/{key}/voiceprint",
            put(speakers::quick::replace_voiceprint),
        )
        .route(
            "/speakers/{key}/voiceprint/purge",
            axum::routing::post(speakers::purge),
        )
        .route(
            "/agents/{key}/speaker-policy",
            get(speakers::policy::get_policy).put(speakers::policy::put_policy),
        )
        .route(
            "/agents/{key}/speakers",
            get(speakers::policy::list_agent_speakers),
        )
        .route(
            "/agents/{key}/speakers/{speaker_key}",
            put(speakers::policy::put_agent_speaker)
                .delete(speakers::policy::delete_agent_speaker),
        )
        .route(
            "/speakers/{key}/bindings",
            get(speakers::policy::list_speaker_bindings),
        )
}
