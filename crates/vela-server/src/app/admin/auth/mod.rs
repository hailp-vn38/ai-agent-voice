//! Admin Password login. A single Admin signs in with the Admin Password and receives an Admin
//! Web Token signed by the Admin Signing Secret from config. The token is pinned to the password
//! revision it was minted under, so a later revision bump invalidates it without a token list.

mod password;
mod token;

use super::*;
use crate::config::AppConfig;
use axum::{
    Router,
    routing::{get, post},
};
use ring::hmac;
use std::{
    collections::VecDeque,
    sync::{Mutex, PoisonError, RwLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// The default Admin Password of a fresh deployment. It is only ever seeded as a hash.
const DEFAULT_PASSWORD: &str = "admin";
const MIN_PASSWORD_CHARS: usize = 8;
const MAX_PASSWORD_BYTES: usize = 1024;
const FAILURE_LIMIT: usize = 5;
const FAILURE_WINDOW: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PasswordLoginStatus {
    pub password_login_enabled: bool,
    pub must_change_password: bool,
    pub revision: i64,
}

/// In-process login state. The revision and flags are mirrored here so the Admin middleware can
/// check a token without a SQLite read on every request.
pub(crate) struct AdminAuthState {
    key: Option<hmac::Key>,
    token_ttl_secs: i64,
    status: RwLock<PasswordLoginStatus>,
    /// Failed logins only.
    login_failures: FailureLimiter,
    /// Failed current-password checks on the password change and Password Login toggle routes.
    credential_failures: FailureLimiter,
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

/// A sliding-window failure budget: `FAILURE_LIMIT` failures per `FAILURE_WINDOW`.
struct FailureLimiter(Mutex<VecDeque<Instant>>);

impl FailureLimiter {
    fn new() -> Self {
        Self(Mutex::new(VecDeque::new()))
    }

    /// Reserves one slot of the budget before the password is checked, so attempts that run
    /// concurrently cannot exceed it. A spent budget returns the remaining lockout instead.
    fn reserve(&self) -> Result<Instant, Duration> {
        let mut failures = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        while failures
            .front()
            .is_some_and(|at| now.duration_since(*at) >= FAILURE_WINDOW)
        {
            failures.pop_front();
        }
        if failures.len() >= FAILURE_LIMIT {
            let oldest = failures.front().copied().unwrap_or(now);
            return Err(FAILURE_WINDOW.saturating_sub(now.duration_since(oldest)));
        }
        failures.push_back(now);
        Ok(now)
    }

    /// Returns the slot of an attempt whose password was right. Earlier failures stay counted.
    fn release(&self, reserved: Instant) {
        let mut failures = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(index) = failures.iter().position(|at| *at == reserved) {
            failures.remove(index);
        }
    }
}

impl AdminAuthState {
    pub(crate) fn from_config(config: &AppConfig) -> Self {
        let secret = config.api.jwt_secret.expose().trim();
        Self {
            key: (!secret.is_empty()).then(|| hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes())),
            token_ttl_secs: i64::try_from(config.api.token_ttl_secs).unwrap_or(i64::MAX),
            status: RwLock::new(PasswordLoginStatus {
                password_login_enabled: true,
                must_change_password: false,
                revision: 1,
            }),
            login_failures: FailureLimiter::new(),
            credential_failures: FailureLimiter::new(),
        }
    }

    pub(crate) fn status(&self) -> PasswordLoginStatus {
        *self.status.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn publish(&self, status: PasswordLoginStatus) {
        // Callers publish only after the SQLite transaction that produced the status commits.
        *self.status.write().unwrap_or_else(PoisonError::into_inner) = status;
    }

    /// True when the bearer is an unexpired Admin Web Token minted under the live revision.
    pub(crate) fn accepts(&self, bearer: &str) -> bool {
        let Some(key) = self.key.as_ref() else {
            return false;
        };
        token::verify(key, bearer, unix_now())
            .is_some_and(|claims| claims.rev == self.status().revision)
    }

    /// Whether a long-lived Admin SSE request would still pass the authenticate middleware.
    pub(super) fn stream_access_allowed(&self, bearer: Option<&str>) -> bool {
        let status = self.status();
        if !status.password_login_enabled {
            return true;
        }
        bearer.is_some_and(|value| self.accepts(value)) && !status.must_change_password
    }

    /// The signed token's expiry, used to stop an SSE response at its deadline.
    pub(super) fn token_expiry(&self, bearer: &str) -> Option<SystemTime> {
        let key = self.key.as_ref()?;
        let claims = token::verify(key, bearer, unix_now())?;
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(u64::try_from(claims.exp).ok()?))
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[derive(sqlx::FromRow)]
struct AdminAuthRow {
    password_login_enabled: i64,
    password_hash: String,
    must_change_password: i64,
    revision: i64,
}

async fn read_row(pool: &sqlx::SqlitePool) -> Result<AdminAuthRow, sqlx::Error> {
    sqlx::query_as::<_, AdminAuthRow>(
        "SELECT password_login_enabled, password_hash, must_change_password, revision
         FROM admin_auth WHERE id = 1",
    )
    .fetch_one(pool)
    .await
}

/// Seeds the default Admin Password when no hash exists yet, then loads the live status into
/// memory. Runs once at startup, after migrations.
pub(crate) async fn prepare(state: &AppState) -> Result<(), sqlx::Error> {
    let Some(database) = state.database.as_ref() else {
        return Ok(());
    };
    let pool = database.pool();
    let mut row = read_row(pool).await?;
    if row.password_hash.is_empty() {
        let seeded = tokio::task::spawn_blocking(|| password::hash(DEFAULT_PASSWORD))
            .await
            .map_err(|_| sqlx::Error::WorkerCrashed)?;
        sqlx::query(
            "UPDATE admin_auth
             SET password_login_enabled = 1, password_hash = ?, must_change_password = 1,
                 updated_at = ?
             WHERE id = 1 AND password_hash = ''",
        )
        .bind(seeded)
        .bind(unix_now())
        .execute(pool)
        .await?;
        row = read_row(pool).await?;
    }
    state.admin_auth.publish(PasswordLoginStatus {
        password_login_enabled: row.password_login_enabled != 0,
        must_change_password: row.must_change_password != 0,
        revision: row.revision,
    });
    Ok(())
}

/// Command-line recovery: puts the Admin Password back to the public default, turns Password Login
/// on and sets the forced-change flag. Writes SQLite directly and audits the reset in the same
/// transaction; a running server keeps its in-memory status until it restarts.
pub(crate) async fn reset_to_default(config: &crate::config::DatabaseConfig) -> anyhow::Result<()> {
    let database = crate::database::Database::connect(config).await?;
    let seeded = tokio::task::spawn_blocking(|| password::hash(DEFAULT_PASSWORD)).await?;
    let request_id = format!("admin-password-reset-{}", Uuid::new_v4());
    let mut tx = database.pool().begin().await?;
    let (prior, new): (i64, i64) = sqlx::query_as(
        "UPDATE admin_auth
         SET password_login_enabled = 1, password_hash = ?, must_change_password = 1,
             revision = revision + 1, updated_at = ?
         WHERE id = 1
         RETURNING revision - 1, revision",
    )
    .bind(seeded)
    .bind(unix_now())
    .fetch_one(&mut *tx)
    .await?;
    crate::database::audit::audit(
        &mut *tx,
        &request_id,
        "admin_auth",
        Some(1),
        "password_reset",
        Some(prior),
        Some(new),
        crate::database::audit::AuditOutcome::Success,
        1,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Public Admin routes that carry no bearer credential. They are mounted only when the signing
/// secret is configured; otherwise they answer 404.
pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/status", get(status))
        .route("/auth/login", post(login))
}

/// Admin routes behind the bearer check. The password change is the one route an Admin Web Token
/// may call while the forced-change flag is set, so the middleware exempts it by path.
pub(super) fn protected_routes() -> Router<AppState> {
    Router::new()
        .route("/auth/password", post(change_password))
        .route("/auth/logout-all", post(logout_everywhere))
        .route("/auth/password-login", post(set_password_login))
}

/// The path the middleware lets through while the forced-change flag is set.
pub(super) fn is_password_change_path(path: &str) -> bool {
    path.ends_with("/auth/password")
}

#[derive(Deserialize)]
struct ChangePasswordRequest {
    current_password: String,
    new_password: String,
}

/// Replaces the Admin Password. The hash, the cleared forced-change flag, the revision bump and
/// the audit event commit together; every token minted before is then rejected.
async fn change_password(State(state): State<AppState>, request: Request) -> Response {
    let (request, body): (Request, ChangePasswordRequest) = match json(request).await {
        Ok(parsed) => parsed,
        Err(response) => return response,
    };
    let ChangePasswordRequest {
        current_password,
        new_password,
    } = body;
    if new_password == DEFAULT_PASSWORD {
        return error(
            &request,
            StatusCode::UNPROCESSABLE_ENTITY,
            "password_is_default",
        );
    }
    if new_password.chars().count() < MIN_PASSWORD_CHARS {
        return error(
            &request,
            StatusCode::UNPROCESSABLE_ENTITY,
            "password_too_short",
        );
    }
    if new_password.len() > MAX_PASSWORD_BYTES {
        return error(
            &request,
            StatusCode::UNPROCESSABLE_ENTITY,
            "password_too_long",
        );
    }
    let Some(database) = state.database.as_ref() else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    };
    let pool = database.pool();
    let row = match read_row(pool).await {
        Ok(row) => row,
        Err(cause) => return sql_error(&request, &cause),
    };
    let attempt = match begin_attempt(&state.admin_auth.credential_failures, &request) {
        Ok(attempt) => attempt,
        Err(response) => return *response,
    };
    let stored = row.password_hash.clone();
    let candidate = current_password.clone();
    let verified = tokio::task::spawn_blocking(move || password::verify(&stored, &candidate))
        .await
        .unwrap_or(false);
    if !verified {
        return error(
            &request,
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_current_password",
        );
    }
    state.admin_auth.credential_failures.release(attempt);
    if new_password == current_password {
        return error(
            &request,
            StatusCode::UNPROCESSABLE_ENTITY,
            "password_unchanged",
        );
    }
    let Some(key) = state.admin_auth.key.as_ref() else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "admin_signing_unavailable",
        );
    };
    let new_hash = match tokio::task::spawn_blocking(move || password::hash(&new_password)).await {
        Ok(hash) => hash,
        Err(_) => {
            return error(
                &request,
                StatusCode::INTERNAL_SERVER_ERROR,
                "password_hash_failed",
            );
        }
    };
    let new_revision = row.revision.saturating_add(1);
    let issued_at = unix_now();
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(cause) => return sql_error(&request, &cause),
    };
    let updated = sqlx::query(
        "UPDATE admin_auth
         SET password_hash = ?, must_change_password = 0, revision = ?, updated_at = ?
         WHERE id = 1 AND revision = ?",
    )
    .bind(new_hash)
    .bind(new_revision)
    .bind(issued_at)
    .bind(row.revision)
    .execute(&mut *tx)
    .await;
    match updated {
        Ok(result) if result.rows_affected() == 1 => {}
        Ok(_) => {
            return error(&request, StatusCode::CONFLICT, "revision_conflict");
        }
        Err(cause) => return sql_error(&request, &cause),
    }
    if let Err(cause) = crate::database::audit::audit(
        &mut *tx,
        id(&request),
        "admin_auth",
        Some(1),
        "password_change",
        Some(row.revision),
        Some(new_revision),
        crate::database::audit::AuditOutcome::Success,
        1,
    )
    .await
    {
        return sql_error(&request, &cause);
    }
    if let Err(cause) = tx.commit().await {
        return sql_error(&request, &cause);
    }
    let enabled = row.password_login_enabled != 0;
    state.admin_auth.publish(PasswordLoginStatus {
        password_login_enabled: enabled,
        must_change_password: false,
        revision: new_revision,
    });
    let expires_at = issued_at.saturating_add(state.admin_auth.token_ttl_secs);
    let token = token::sign(
        key,
        &token::Claims {
            iat: issued_at,
            exp: expires_at,
            rev: new_revision,
        },
    );
    let mut response = Json(serde_json::json!({
        "token": token,
        "token_type": "Bearer",
        "expires_at": expires_at,
        "must_change_password": false,
    }))
    .into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

/// Reserves a password attempt, or answers 429 with `Retry-After` while the budget is spent. The
/// caller releases the reservation only when the password turns out to be right.
fn begin_attempt(limiter: &FailureLimiter, request: &Request) -> Result<Instant, Box<Response>> {
    let remaining = match limiter.reserve() {
        Ok(reserved) => return Ok(reserved),
        Err(remaining) => remaining,
    };
    let seconds = (remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0)).max(1);
    tracing::warn!(
        event = "admin_password_attempts_limited",
        request_id = id(request),
        retry_after_secs = seconds,
        "admin password attempt refused by the failure limiter"
    );
    let mut response = error(request, StatusCode::TOO_MANY_REQUESTS, "too_many_attempts");
    if let Ok(value) = axum::http::HeaderValue::from_str(&seconds.to_string()) {
        response
            .headers_mut()
            .insert(axum::http::header::RETRY_AFTER, value);
    }
    Err(Box::new(response))
}

fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

/// Revokes every Admin Web Token minted so far by bumping the revision, then hands the caller a
/// token under the new revision. The bump and its audit event commit together.
async fn logout_everywhere(State(state): State<AppState>, request: Request) -> Response {
    let Some(key) = state.admin_auth.key.as_ref() else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "admin_signing_unavailable",
        );
    };
    let Some(database) = state.database.as_ref() else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    };
    let pool = database.pool();
    let row = match read_row(pool).await {
        Ok(row) => row,
        Err(cause) => return sql_error(&request, &cause),
    };
    let new_revision = row.revision.saturating_add(1);
    let issued_at = unix_now();
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(cause) => return sql_error(&request, &cause),
    };
    let updated = sqlx::query(
        "UPDATE admin_auth SET revision = ?, updated_at = ? WHERE id = 1 AND revision = ?",
    )
    .bind(new_revision)
    .bind(issued_at)
    .bind(row.revision)
    .execute(&mut *tx)
    .await;
    match updated {
        Ok(result) if result.rows_affected() == 1 => {}
        Ok(_) => return error(&request, StatusCode::CONFLICT, "revision_conflict"),
        Err(cause) => return sql_error(&request, &cause),
    }
    if let Err(cause) = crate::database::audit::audit(
        &mut *tx,
        id(&request),
        "admin_auth",
        Some(1),
        "logout_everywhere",
        Some(row.revision),
        Some(new_revision),
        crate::database::audit::AuditOutcome::Success,
        1,
    )
    .await
    {
        return sql_error(&request, &cause);
    }
    if let Err(cause) = tx.commit().await {
        return sql_error(&request, &cause);
    }
    let must_change_password = row.must_change_password != 0;
    state.admin_auth.publish(PasswordLoginStatus {
        password_login_enabled: row.password_login_enabled != 0,
        must_change_password,
        revision: new_revision,
    });
    let expires_at = issued_at.saturating_add(state.admin_auth.token_ttl_secs);
    let token = token::sign(
        key,
        &token::Claims {
            iat: issued_at,
            exp: expires_at,
            rev: new_revision,
        },
    );
    no_store(
        Json(serde_json::json!({
            "token": token,
            "token_type": "Bearer",
            "expires_at": expires_at,
            "must_change_password": must_change_password,
        }))
        .into_response(),
    )
}

#[derive(Deserialize)]
struct SetPasswordLoginRequest {
    enabled: bool,
    current_password: String,
}

/// Switches Password Login on or off. The current Admin Password is required in both directions,
/// and it is also what lets a caller with no credential switch the login back on. Shares the
/// credential failure budget with the password change, never the login one, because while
/// Password Login is off these routes need no bearer. Turning it off is refused while the
/// forced-change flag is set.
async fn set_password_login(State(state): State<AppState>, request: Request) -> Response {
    let (request, body): (Request, SetPasswordLoginRequest) = match json(request).await {
        Ok(parsed) => parsed,
        Err(response) => return response,
    };
    if !body.enabled && state.admin_auth.status().must_change_password {
        return error(&request, StatusCode::FORBIDDEN, "password_change_required");
    }
    let Some(database) = state.database.as_ref() else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    };
    let pool = database.pool();
    let row = match read_row(pool).await {
        Ok(row) => row,
        Err(cause) => return sql_error(&request, &cause),
    };
    let attempt = match begin_attempt(&state.admin_auth.credential_failures, &request) {
        Ok(attempt) => attempt,
        Err(response) => return *response,
    };
    let stored = row.password_hash.clone();
    let candidate = body.current_password;
    let verified = tokio::task::spawn_blocking(move || password::verify(&stored, &candidate))
        .await
        .unwrap_or(false);
    if !verified {
        return error(
            &request,
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_current_password",
        );
    }
    state.admin_auth.credential_failures.release(attempt);
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(cause) => return sql_error(&request, &cause),
    };
    let updated = sqlx::query(
        "UPDATE admin_auth SET password_login_enabled = ?, updated_at = ?
         WHERE id = 1 AND revision = ?",
    )
    .bind(i64::from(body.enabled))
    .bind(unix_now())
    .bind(row.revision)
    .execute(&mut *tx)
    .await;
    match updated {
        Ok(result) if result.rows_affected() == 1 => {}
        Ok(_) => return error(&request, StatusCode::CONFLICT, "revision_conflict"),
        Err(cause) => return sql_error(&request, &cause),
    }
    if let Err(cause) = crate::database::audit::audit(
        &mut *tx,
        id(&request),
        "admin_auth",
        Some(1),
        "password_login_set",
        Some(row.revision),
        Some(row.revision),
        crate::database::audit::AuditOutcome::Success,
        1,
    )
    .await
    {
        return sql_error(&request, &cause);
    }
    if let Err(cause) = tx.commit().await {
        return sql_error(&request, &cause);
    }
    let must_change_password = row.must_change_password != 0;
    state.admin_auth.publish(PasswordLoginStatus {
        password_login_enabled: body.enabled,
        must_change_password,
        revision: row.revision,
    });
    no_store(
        Json(serde_json::json!({
            "password_login_enabled": body.enabled,
            "must_change_password": must_change_password,
        }))
        .into_response(),
    )
}

#[derive(Deserialize)]
struct LoginRequest {
    password: String,
}

async fn status(State(state): State<AppState>) -> Response {
    let status = state.admin_auth.status();
    let mut response = Json(serde_json::json!({
        "password_login_enabled": status.password_login_enabled,
        "must_change_password": status.must_change_password,
    }))
    .into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

async fn login(State(state): State<AppState>, request: Request) -> Response {
    let (request, body): (Request, LoginRequest) = match json(request).await {
        Ok(parsed) => parsed,
        Err(response) => return response,
    };
    let Some(database) = state.database.as_ref() else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    };
    let row = match read_row(database.pool()).await {
        Ok(row) => row,
        Err(cause) => return sql_error(&request, &cause),
    };
    if row.password_login_enabled == 0 {
        return error(&request, StatusCode::CONFLICT, "password_login_disabled");
    }
    let attempt = match begin_attempt(&state.admin_auth.login_failures, &request) {
        Ok(attempt) => attempt,
        Err(response) => return *response,
    };
    // The token is minted from the row that held the verified hash, never from the in-memory
    // mirror: a reset written by the recovery command must force the change it asked for.
    let status = PasswordLoginStatus {
        password_login_enabled: true,
        must_change_password: row.must_change_password != 0,
        revision: row.revision,
    };
    let stored = row.password_hash;
    let candidate = body.password;
    let verified = tokio::task::spawn_blocking(move || password::verify(&stored, &candidate))
        .await
        .unwrap_or(false);
    if !verified {
        tracing::warn!(
            event = "admin_password_login_failed",
            request_id = id(&request),
            "admin password login rejected"
        );
        return error(&request, StatusCode::UNAUTHORIZED, "invalid_password");
    }
    let Some(key) = state.admin_auth.key.as_ref() else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "admin_signing_unavailable",
        );
    };
    state.admin_auth.login_failures.release(attempt);
    // SQLite moved ahead of this process (recovery command): adopt it so the token is accepted.
    if status.revision > state.admin_auth.status().revision {
        state.admin_auth.publish(status);
    }
    let issued_at = unix_now();
    let expires_at = issued_at.saturating_add(state.admin_auth.token_ttl_secs);
    let token = token::sign(
        key,
        &token::Claims {
            iat: issued_at,
            exp: expires_at,
            rev: status.revision,
        },
    );
    let mut response = Json(serde_json::json!({
        "token": token,
        "token_type": "Bearer",
        "expires_at": expires_at,
        "must_change_password": status.must_change_password,
    }))
    .into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}
