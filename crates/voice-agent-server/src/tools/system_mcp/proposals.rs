//! Single-process proposals: one claim, no retry; authority is checked at publication.
use super::*;
use crate::database::{mcp_servers::McpChanges, tool_allowlist::ToolReview, writes::WriteError};
use std::{collections::HashMap, sync::Mutex, time::Duration};

#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(tag = "tool", deny_unknown_fields)]
pub(crate) enum Command {
    #[serde(rename = "mcp_request_server_change")]
    Server {
        server_key: String,
        enabled: bool,
        expected_revision: i64,
    },
    #[serde(rename = "mcp_request_agent_binding_change")]
    Binding {
        agent_key: String,
        server_key: String,
        enabled: bool,
        expected_agent_revision: i64,
    },
    #[serde(rename = "mcp_request_tool_review")]
    Review {
        agent_key: String,
        server_key: String,
        tool_name: String,
        fingerprint: String,
        observed_revision: i64,
        allow: bool,
        sensitive: bool,
        expected_revision: i64,
    },
}
#[derive(Clone, serde::Serialize)]
pub(crate) struct ChangeRequest {
    pub id: String,
    pub command: Command,
    pub expires_at: i64,
    pub status: &'static str,
}
struct Entry {
    view: ChangeRequest,
    caller: SystemToolContext,
    expires: Instant,
}
#[derive(Default)]
pub(super) struct PendingRequests(Mutex<HashMap<String, Entry>>);
impl SystemMcp {
    pub(super) async fn authorize(
        &self,
        ctx: &SystemToolContext,
        name: &str,
    ) -> Result<(), SystemToolError> {
        if !self.list_tools(ctx).iter().any(|t| t.name == name) {
            return Err(SystemToolError::Denied);
        }
        let db = self
            .status
            .database
            .as_ref()
            .ok_or(SystemToolError::Unavailable)?;
        match &ctx.caller {
            SystemCaller::WebAssistant {
                device_db_id,
                agent_id,
            } => {
                if !db
                    .system_mcp_caller_allowed(*device_db_id, *agent_id)
                    .await?
                {
                    return Err(SystemToolError::Denied);
                }
                let live = db.system_mcp_grants(self.enabled).await?.grants;
                if !catalog::catalog(live).iter().any(|t| t.name == name) {
                    return Err(SystemToolError::Denied);
                }
            }
            SystemCaller::Remote {
                jti, expires_at, ..
            } => {
                if !db.system_mcp_token_allowed(jti, *expires_at).await? {
                    return Err(SystemToolError::Denied);
                }
            }
        }
        Ok(())
    }
    pub(super) fn propose(
        &self,
        ctx: &SystemToolContext,
        name: &str,
        mut arguments: Value,
    ) -> Result<Value, SystemToolError> {
        if !arguments.is_object() || arguments.get("tool").is_some() {
            return Err(SystemToolError::InvalidArguments);
        }
        arguments["tool"] = json!(name);
        let command: Command = parse(arguments)?;
        match &command {
            Command::Server {
                server_key,
                expected_revision,
                ..
            } => {
                validate_key(server_key)?;
                positive(*expected_revision)?;
            }
            Command::Binding {
                agent_key,
                server_key,
                expected_agent_revision,
                ..
            } => {
                validate_key(agent_key)?;
                validate_key(server_key)?;
                positive(*expected_agent_revision)?;
            }
            Command::Review {
                agent_key,
                server_key,
                tool_name,
                fingerprint,
                observed_revision,
                expected_revision,
                ..
            } => {
                validate_key(agent_key)?;
                validate_key(server_key)?;
                positive(*observed_revision)?;
                positive(*expected_revision)?;
                if tool_name.is_empty()
                    || tool_name.len() > 128
                    || !tool_name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
                    || fingerprint.len() != 64
                    || !fingerprint
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    return Err(SystemToolError::InvalidArguments);
                }
            }
        }
        let mut requests = self.pending.0.lock().unwrap_or_else(|e| e.into_inner());
        requests.retain(|_, entry| entry.expires > Instant::now());
        let identity = caller_key(&ctx.caller);
        if requests.len() >= 256
            || requests
                .values()
                .filter(|e| caller_key(&e.caller.caller) == identity)
                .count()
                >= 16
        {
            return Err(SystemToolError::Unavailable);
        }
        let id = uuid::Uuid::new_v4().to_string();
        let view = ChangeRequest {
            id: id.clone(),
            command,
            expires_at: crate::database::unix_seconds().unwrap_or_default() + 300,
            status: "pending",
        };
        requests.insert(
            id,
            Entry {
                view: view.clone(),
                caller: ctx.clone(),
                expires: Instant::now() + Duration::from_secs(300),
            },
        );
        Ok(json!(view))
    }
    pub(crate) fn change_requests(&self) -> Vec<ChangeRequest> {
        let mut entries = self.pending.0.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|_, e| e.expires > Instant::now());
        let mut views: Vec<_> = entries.values().map(|e| e.view.clone()).collect();
        views.sort_by(|a, b| a.id.cmp(&b.id));
        views
    }
    pub(crate) fn reject(&self, id: &str) -> Result<ChangeRequest, SystemToolError> {
        let mut entries = self.pending.0.lock().unwrap_or_else(|e| e.into_inner());
        let e = entries.get_mut(id).ok_or(SystemToolError::NotFound)?;
        if e.expires <= Instant::now() || e.view.status != "pending" {
            return Err(SystemToolError::Denied);
        }
        e.view.status = "rejected";
        Ok(e.view.clone())
    }
    pub(crate) async fn confirm(
        self: &Arc<Self>,
        id: &str,
        request_id: &str,
    ) -> Result<ChangeRequest, SystemToolError> {
        let (view, ctx, expires) = {
            let mut entries = self.pending.0.lock().unwrap_or_else(|e| e.into_inner());
            let e = entries.get_mut(id).ok_or(SystemToolError::NotFound)?;
            if e.expires <= Instant::now() || e.view.status != "pending" {
                return Err(SystemToolError::Denied);
            }
            e.view.status = "claimed";
            (e.view.clone(), e.caller.clone(), e.expires)
        };
        let core = self.clone();
        let request_id = request_id.to_owned();
        // Own dispatch independently of a dropped HTTP response; claimed IDs are never retryable.
        tokio::spawn(async move {
            let result = core.commit(&ctx, &view.command, expires, &request_id).await;
            let mut entries = core.pending.0.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(e) = entries.get_mut(&view.id) {
                e.view.status = if result.is_ok() { "applied" } else { "failed" };
                return Ok(e.view.clone());
            }
            Err(SystemToolError::NotFound)
        })
        .await
        .map_err(|_| SystemToolError::Unavailable)?
    }
    async fn commit(
        &self,
        ctx: &SystemToolContext,
        command: &Command,
        expires: Instant,
        request_id: &str,
    ) -> Result<(), WriteError> {
        let db = self
            .status
            .database
            .as_ref()
            .ok_or(WriteError::Unavailable)?;
        let _publication = db.tool_security.publication.write().await;
        if expires <= Instant::now() {
            return Err(WriteError::Gone("proposal_expired"));
        }
        self.authorize(ctx, "mcp_request_server_change")
            .await
            .map_err(|_| WriteError::Invalid("permission_revoked"))?;
        match command {
            Command::Server {
                server_key,
                enabled,
                expected_revision,
            } => {
                let old = mcp_by(db, server_key).await.map_err(WriteError::Sql)?;
                db.update_mcp_server_published(
                    &old,
                    *expected_revision,
                    McpChanges {
                        name: &old.name,
                        url: &old.url,
                        auth_type: &old.auth_type,
                        auth_header_name: old.auth_header_name.as_deref(),
                        credential: old.credential_json.as_deref(),
                        connect_timeout: old.connect_timeout_ms,
                        request_timeout: old.request_timeout_ms,
                        enabled: i64::from(*enabled),
                    },
                    request_id,
                )
                .await
            }
            Command::Binding {
                agent_key,
                server_key,
                enabled,
                expected_agent_revision,
            } => {
                db.put_agent_mcp_binding_published(
                    agent_key,
                    server_key,
                    *enabled,
                    *expected_agent_revision,
                    request_id,
                )
                .await
            }
            Command::Review {
                agent_key,
                server_key,
                tool_name,
                fingerprint,
                observed_revision,
                allow,
                sensitive,
                expected_revision,
            } => db
                .review_external_tool_published(
                    agent_key,
                    ToolReview {
                        server_key: server_key.clone(),
                        original_name: tool_name.clone(),
                        fingerprint: fingerprint.clone(),
                        observed_revision: *observed_revision,
                        allowed: *allow,
                        sensitive: *sensitive,
                    },
                    *expected_revision,
                    request_id,
                )
                .await
                .map(|_| ()),
        }
    }
}
fn positive(revision: i64) -> Result<(), SystemToolError> {
    if revision > 0 && revision < i64::MAX {
        Ok(())
    } else {
        Err(SystemToolError::InvalidArguments)
    }
}
fn caller_key(caller: &SystemCaller) -> String {
    match caller {
        SystemCaller::WebAssistant {
            device_db_id,
            agent_id,
        } => format!("web:{device_db_id}:{agent_id}"),
        SystemCaller::Remote { subject, .. } => format!("remote:{subject}"),
    }
}
