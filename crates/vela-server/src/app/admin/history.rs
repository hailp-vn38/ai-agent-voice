//! Admin Persistent Transcript surfaces: the bounded archive read, and the one destructive
//! operation in the whole Admin API.
//!
//! Both stay available whenever the Admin API is, including while capture is off: turning capture
//! off stops new records, it never strands the archive an operator already owns.  Neither surface
//! reaches into a running Voice Session, and neither is a conversational input — Dialogue History
//! stays in RAM whatever an administrator does here.
use super::*;

use crate::database::history::{
    HistoryRole,
    queries::{HistoryFilters, PurgeScope},
};

/// The literal an all-history purge must carry.  Archive deletion is irreversible, so the widest
/// scope is the one an operator spells out rather than the one a request reaches by omission.
const PURGE_ALL_HISTORY: &str = "PURGE_ALL_HISTORY";

/// The keyword that names the widest scope, kept apart from the confirmation so that naming it
/// without confirming is a rejected request rather than a second way to ask for it.
const ALL_SCOPE: &str = "all";

/// The page window this surface accepts, declared apart from the filters so no query parameter can
/// mean nothing here.
#[derive(Default, Deserialize)]
pub(super) struct HistoryPageQuery {
    #[serde(default)]
    page: Option<u32>,
    #[serde(default)]
    page_size: Option<u32>,
    #[serde(default)]
    sort: Option<String>,
}

/// Typed filters.  Every value is bounded before it reaches a predicate and the sort is an
/// allowlist, so a request can neither name a column nor smuggle an expression into one.
#[derive(Deserialize)]
pub(super) struct HistoryQuery {
    #[serde(flatten)]
    page: HistoryPageQuery,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    device_id: Option<i64>,
    #[serde(default)]
    agent_id: Option<i64>,
    #[serde(default)]
    template_id: Option<i64>,
    #[serde(default)]
    role: Option<String>,
}

/// The filter set, or why the request named something this archive cannot answer.
///
/// A rejected filter never degrades into an absent one: `device_id=0` matched by nothing would be
/// answered with the *whole archive*, which is the one answer an administrator must never be handed
/// in place of the one they asked for.
impl HistoryFilters {
    fn of(query: &HistoryQuery) -> Result<Self, &'static str> {
        if let Some(session_id) = query.session_id.as_deref()
            && !valid_identity(session_id)
        {
            return Err("invalid_query");
        }
        for id in [query.device_id, query.agent_id, query.template_id] {
            if id.is_some_and(|id| id <= 0) {
                return Err("invalid_query");
            }
        }
        Ok(Self {
            session_id: query.session_id.clone(),
            device_id: query.device_id,
            agent_id: query.agent_id,
            template_id: query.template_id,
            role: role_filter(query.role.as_deref())?,
        })
    }
}

/// The only two roles the archive stores, so no other role can be asked for.
fn role_filter(role: Option<&str>) -> Result<Option<&'static str>, &'static str> {
    match role {
        Some(value) => match HistoryRole::parse(value) {
            Some(role) => Ok(Some(role.as_str())),
            None => Err("invalid_query"),
        },
        None => Ok(None),
    }
}

pub(super) async fn list_history(
    State(state): State<AppState>,
    Query(query): Query<HistoryQuery>,
    request: Request,
) -> Response {
    let (page, page_size) = match page_bounds(query.page.page, query.page.page_size) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    if query.page.sort.as_deref().is_some_and(|sort| {
        !matches!(
            sort,
            "created_at" | "-created_at" | "sequence" | "-sequence"
        )
    }) {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_query");
    }
    let filters = match HistoryFilters::of(&query) {
        Ok(filters) => filters,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let database = match database(&state) {
        Ok(value) => value,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    match database
        .list_history(filters, query.page.sort.as_deref(), page, page_size)
        .await
    {
        Ok(items) => Json(serde_json::json!({
            "items": items,
            "page": page,
            "page_size": page_size,
            "max_page_size": PAGE_MAX
        }))
        .into_response(),
        Err(error_value) => sql_error(&request, &error_value),
    }
}

/// Exactly one scope, so a request naming none or several can never be read as a wider purge than
/// the operator meant.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PurgeHistory {
    #[serde(default)]
    device_id: Option<i64>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    all: Option<String>,
    #[serde(default)]
    confirm: Option<String>,
}

/// The single deletion a purge request is allowed to become.
impl PurgeHistory {
    fn scope(&self) -> Result<PurgeScope, &'static str> {
        let named = usize::from(self.device_id.is_some())
            + usize::from(self.session_id.is_some())
            + usize::from(self.all.is_some());
        if named != 1 {
            return Err("invalid_purge_scope");
        }
        if let Some(all) = self.all.as_deref() {
            if all != ALL_SCOPE {
                return Err("invalid_purge_scope");
            }
            return if self.confirm.as_deref() == Some(PURGE_ALL_HISTORY) {
                Ok(PurgeScope::All)
            } else {
                Err("confirmation_required")
            };
        }
        // A confirmation beside a narrower scope asks for more than that scope names, so it is
        // rejected rather than ignored: ignoring it would let a request look confirmed and not be.
        if self.confirm.is_some() {
            return Err("invalid_purge_scope");
        }
        if let Some(device_id) = self.device_id {
            return if device_id > 0 {
                Ok(PurgeScope::Device(device_id))
            } else {
                Err("invalid_purge_scope")
            };
        }
        let session_id = self.session_id.as_deref().unwrap_or_default();
        if valid_identity(session_id) {
            Ok(PurgeScope::Session(session_id.to_owned()))
        } else {
            Err("invalid_purge_scope")
        }
    }
}

pub(super) async fn purge_history(State(state): State<AppState>, request: Request) -> Response {
    let (request, body): (_, PurgeHistory) = match json(request).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let scope = match body.scope() {
        Ok(scope) => scope,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let database = match database(&state) {
        Ok(value) => value,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    match database.purge_history(scope, id(&request)).await {
        Ok(deleted) => Json(serde_json::json!({"deleted":deleted})).into_response(),
        Err(cause) => write_error(&request, cause),
    }
}
