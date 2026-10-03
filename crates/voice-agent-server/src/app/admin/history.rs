//! Admin Persistent Transcript surfaces: the bounded archive read, and the one destructive
//! operation in the whole Admin API.
//!
//! Both stay available whenever the Admin API is, including while capture is off: turning capture
//! off stops new records, it never strands the archive an operator already owns.  Neither surface
//! reaches into a running Voice Session, and neither is a conversational input — Dialogue History
//! stays in RAM whatever an administrator does here.
use super::*;

use crate::database::history::HistoryRole;

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

#[derive(Serialize, FromRow)]
struct HistoryMessage {
    id: i64,
    session_id: String,
    device_id: i64,
    agent_id: i64,
    template_id: Option<i64>,
    sequence: i64,
    turn_id: Option<String>,
    role: String,
    text: String,
    /// Unix milliseconds UTC, matching the retention cutoff this archive is pruned by.
    created_at: i64,
}

/// The filter set, or why the request named something this archive cannot answer.
///
/// A rejected filter never degrades into an absent one: `device_id=0` matched by nothing would be
/// answered with the *whole archive*, which is the one answer an administrator must never be handed
/// in place of the one they asked for.
struct HistoryFilters {
    session_id: Option<String>,
    device_id: Option<i64>,
    agent_id: Option<i64>,
    template_id: Option<i64>,
    role: Option<&'static str>,
}

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

    /// Adds each present filter as one clause.  Absent filters contribute nothing, so the query is
    /// the whole archive only when the request asked for the whole archive.
    ///
    /// The set is consumed because every value it holds is bound into the query it builds.
    fn push(self, query: &mut sqlx::QueryBuilder<'_, Sqlite>) {
        if let Some(session_id) = self.session_id {
            query.push(" AND session_id = ").push_bind(session_id);
        }
        for (column, id) in [
            ("device_id", self.device_id),
            ("agent_id", self.agent_id),
            ("template_id", self.template_id),
        ] {
            if let Some(id) = id {
                query.push(" AND ").push(column).push(" = ").push_bind(id);
            }
        }
        if let Some(role) = self.role {
            query.push(" AND role = ").push_bind(role);
        }
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
    let order = match query.page.sort.as_deref().unwrap_or("-created_at") {
        "created_at" => "created_at ASC",
        "sequence" => "sequence ASC",
        "-sequence" => "sequence DESC",
        "-created_at" => "created_at DESC",
        _ => return error(&request, StatusCode::BAD_REQUEST, "invalid_query"),
    };
    let filters = match HistoryFilters::of(&query) {
        Ok(filters) => filters,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let pool = match db(&state) {
        Ok(value) => value,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let mut builder = sqlx::QueryBuilder::<Sqlite>::new(
        "SELECT id, session_id, device_id, agent_id, template_id, sequence, turn_id, role, text, \
         created_at FROM history_messages WHERE 1 = 1",
    );
    filters.push(&mut builder);
    builder
        .push(" ORDER BY ")
        .push(order)
        .push(" LIMIT ")
        .push_bind(i64::from(page_size))
        .push(" OFFSET ")
        .push_bind(i64::from((page - 1) * page_size));
    match builder
        .build_query_as::<HistoryMessage>()
        .fetch_all(pool)
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
enum PurgeScope {
    Device(i64),
    Session(String),
    All,
}

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

impl PurgeScope {
    /// Adds this scope's predicate, or nothing at all for the confirmed all-history delete.
    fn push(self, query: &mut sqlx::QueryBuilder<'_, Sqlite>) {
        match self {
            Self::All => {}
            Self::Device(id) => {
                query.push(" AND device_id = ").push_bind(id);
            }
            Self::Session(id) => {
                query.push(" AND session_id = ").push_bind(id);
            }
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
    let pool = match db(&state) {
        Ok(value) => value,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let mut transaction = match pool.begin().await {
        Ok(value) => value,
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let mut builder = sqlx::QueryBuilder::<Sqlite>::new("DELETE FROM history_messages WHERE 1 = 1");
    scope.push(&mut builder);
    let deleted = match builder.build().execute(&mut *transaction).await {
        Ok(result) => result.rows_affected(),
        Err(error_value) => {
            let _ = transaction.rollback().await;
            return sql_error(&request, &error_value);
        }
    };
    // The deletion and the record of it are one transaction: an archive an operator cannot account
    // for is the one outcome worse than a failed purge.
    if audit(
        &mut *transaction,
        id(&request),
        "history",
        None,
        "purge",
        None,
        None,
        AuditOutcome::Success,
        deleted,
    )
    .await
    .is_err()
        || transaction.commit().await.is_err()
    {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    }
    Json(serde_json::json!({"deleted": deleted})).into_response()
}
