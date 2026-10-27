//! Voice Session admission: the whole pre-upgrade decision for one WebSocket upgrade request.
//!
//! [`admit`] turns a request into either an admitted connection (a Voice Session's runtimes or an
//! Enrollment Session) or exactly one [`Rejection`]. Its caller learns nothing about tickets,
//! bearer tokens, credential publication, enrollment routing, profile resolution or the optional
//! per-session capabilities; it maps the outcome onto HTTP and logs the single `ws_auth` decision.
use super::*;

/// What an admitted upgrade request becomes once the socket is upgraded.
pub(super) enum Admission {
    /// A provisioned Device or the Web Assistant: everything the Voice Session needs, resolved
    /// and pinned before the upgrade.
    Voice(Box<SocketRuntimes>),
    /// An unregistered Device's control-plane connection; it has no profile or providers.
    Enrollment(crate::app::enrollment::PendingConnection),
}

/// The one reason an upgrade request was refused. Each variant owns its HTTP response and its
/// `ws_auth` telemetry, so no refusal can be returned without being logged the same way.
pub(super) enum Rejection {
    ShuttingDown,
    DuplicateProtocolHeader,
    AmbiguousIdentity,
    InvalidProtocolHeaders,
    InvalidDeviceId,
    InvalidClientId,
    DeviceLookupUnavailable,
    AmbiguousAuthentication,
    TicketRequired,
    UntrustedOrigin,
    AssistantCapacityExceeded,
    InvalidTicket,
    TicketRequiresManagedDevice,
    InvalidBearerToken,
    /// Enrollment routing refused the Device and chose its own response.
    DeviceRouteRejected(Box<Response>),
    ProfileRuntimeUnavailable(String),
    DeviceNotAdmitted,
    DeviceAdmissionUnavailable,
    ProfileUnavailable,
    ConfiguredRuntimeUnavailable,
}

impl Rejection {
    pub(super) fn reason_code(&self) -> &'static str {
        match self {
            Self::ShuttingDown => "shutting_down",
            Self::DuplicateProtocolHeader => "duplicate_protocol_header",
            Self::AmbiguousIdentity => "ambiguous_protocol_identity",
            Self::InvalidProtocolHeaders => "invalid_protocol_headers",
            Self::InvalidDeviceId => "invalid_device_id",
            Self::InvalidClientId => "invalid_client_id",
            Self::DeviceLookupUnavailable => "device_lookup_unavailable",
            Self::AmbiguousAuthentication => "ambiguous_authentication",
            Self::TicketRequired => "ticket_required",
            Self::UntrustedOrigin => "untrusted_origin",
            Self::AssistantCapacityExceeded => "assistant_capacity_exceeded",
            Self::InvalidTicket => "invalid_ticket",
            Self::TicketRequiresManagedDevice => "ticket_requires_managed_device",
            Self::InvalidBearerToken => "invalid_bearer_token",
            Self::DeviceRouteRejected(_) => "device_route_rejected",
            Self::ProfileRuntimeUnavailable(_) => "profile_runtime_unavailable",
            Self::DeviceNotAdmitted => "device_not_admitted",
            Self::DeviceAdmissionUnavailable => "device_admission_unavailable",
            Self::ProfileUnavailable => "profile_unavailable",
            Self::ConfiguredRuntimeUnavailable => "configured_runtime_unavailable",
        }
    }

    /// Bearer outcome at the point of refusal: refusals before the credential step never claim a
    /// bearer result.
    fn bearer_result(&self, bearer_auth_enabled: bool) -> &'static str {
        match self {
            Self::ShuttingDown
            | Self::DuplicateProtocolHeader
            | Self::AmbiguousIdentity
            | Self::InvalidProtocolHeaders
            | Self::InvalidDeviceId
            | Self::InvalidClientId
            | Self::DeviceLookupUnavailable
            | Self::AmbiguousAuthentication
            | Self::TicketRequired
            | Self::UntrustedOrigin
            | Self::AssistantCapacityExceeded
            | Self::InvalidTicket
            | Self::TicketRequiresManagedDevice => "not_checked",
            Self::InvalidBearerToken => "denied",
            Self::DeviceRouteRejected(_)
            | Self::ProfileRuntimeUnavailable(_)
            | Self::DeviceNotAdmitted
            | Self::DeviceAdmissionUnavailable
            | Self::ProfileUnavailable
            | Self::ConfiguredRuntimeUnavailable => bearer_auth_result(bearer_auth_enabled),
        }
    }

    fn device_admission_result(&self) -> &'static str {
        match self {
            Self::DuplicateProtocolHeader
            | Self::AmbiguousIdentity
            | Self::InvalidProtocolHeaders
            | Self::AmbiguousAuthentication
            | Self::TicketRequired
            | Self::UntrustedOrigin
            | Self::AssistantCapacityExceeded
            | Self::InvalidTicket
            | Self::TicketRequiresManagedDevice
            | Self::InvalidBearerToken => "not_checked",
            Self::InvalidDeviceId
            | Self::InvalidClientId
            | Self::DeviceRouteRejected(_)
            | Self::DeviceNotAdmitted => "denied",
            Self::ShuttingDown
            | Self::DeviceLookupUnavailable
            | Self::ProfileRuntimeUnavailable(_)
            | Self::DeviceAdmissionUnavailable
            | Self::ProfileUnavailable
            | Self::ConfiguredRuntimeUnavailable => "unavailable",
        }
    }

    /// Records the refusal once, without raw headers, token, Device ID or Client ID.
    pub(super) fn log(&self, bearer_auth_enabled: bool) {
        log_ws_auth(
            bearer_auth_enabled,
            self.bearer_result(bearer_auth_enabled),
            self.device_admission_result(),
            "denied",
            self.reason_code(),
        );
    }
}

impl IntoResponse for Rejection {
    fn into_response(self) -> Response {
        let (status, body): (StatusCode, std::borrow::Cow<'static, str>) = match self {
            Self::DeviceRouteRejected(response) => return *response,
            Self::ProfileRuntimeUnavailable(message) => {
                (StatusCode::SERVICE_UNAVAILABLE, message.into())
            }
            Self::ShuttingDown => (
                StatusCode::SERVICE_UNAVAILABLE,
                "server is shutting down".into(),
            ),
            Self::DuplicateProtocolHeader => {
                (StatusCode::BAD_REQUEST, "duplicate protocol header".into())
            }
            Self::AmbiguousIdentity => (
                StatusCode::BAD_REQUEST,
                "ambiguous protocol identity".into(),
            ),
            Self::InvalidProtocolHeaders => (
                StatusCode::BAD_REQUEST,
                "Protocol-Version, Device-Id and Client-Id are required".into(),
            ),
            Self::InvalidDeviceId => (StatusCode::BAD_REQUEST, "invalid Device-Id".into()),
            Self::InvalidClientId => (StatusCode::BAD_REQUEST, "invalid Client-Id".into()),
            Self::DeviceLookupUnavailable | Self::DeviceAdmissionUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "device admission unavailable".into(),
            ),
            Self::AmbiguousAuthentication => {
                (StatusCode::BAD_REQUEST, "ambiguous authentication".into())
            }
            Self::TicketRequired => (StatusCode::UNAUTHORIZED, "ticket required".into()),
            Self::UntrustedOrigin => (StatusCode::FORBIDDEN, "untrusted origin".into()),
            Self::AssistantCapacityExceeded => (
                StatusCode::TOO_MANY_REQUESTS,
                "assistant capacity exceeded".into(),
            ),
            Self::InvalidTicket => (StatusCode::UNAUTHORIZED, "invalid ticket".into()),
            Self::TicketRequiresManagedDevice => (
                StatusCode::UNAUTHORIZED,
                "ticket requires managed Device".into(),
            ),
            Self::InvalidBearerToken => (StatusCode::UNAUTHORIZED, "invalid bearer token".into()),
            Self::DeviceNotAdmitted => (StatusCode::FORBIDDEN, "device not admitted".into()),
            Self::ProfileUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "agent profile unavailable".into(),
            ),
            Self::ConfiguredRuntimeUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "configured provider runtime is unavailable".into(),
            ),
        };
        (status, body).into_response()
    }
}

/// Decides one upgrade request. The Admission Gate is consulted first; nothing past it starts for
/// a refused request.
pub(super) async fn admit(
    state: &AppState,
    headers: &HeaderMap,
    query: &WebsocketQuery,
    connect_started: Instant,
    metrics: Option<Arc<RuntimeMetrics>>,
) -> Result<Admission, Rejection> {
    if !state.admission_gate().is_open() {
        return Err(Rejection::ShuttingDown);
    }
    let config = &state.config;
    let (device_id, client_id) = protocol_identity(headers, query)?;
    let assistant_auth = state.web_assistant.clone();
    let mut device = lookup_device(state, device_id).await?;
    // Only the Web Assistant takes part in credential publication. A physical Device never waits
    // on it, so a System save (or a slow Web Assistant admission queued ahead of one) cannot
    // stall Device reconnects.
    //
    // For the Web Assistant, publication is held through graph resolution and connection
    // registration; System saves cannot rebind between credential validation and the immutable
    // profile snapshot. The row is re-read under the guard so validation never uses a row a save
    // replaced while this connection waited.
    let publication = if device.as_ref().is_some_and(|d| d.kind == "web_assistant") {
        let guard = assistant_auth.publication.read().await;
        device = lookup_device(state, device_id).await?;
        Some(guard)
    } else {
        None
    };
    let assistant_connection =
        if let Some(device) = device.as_ref().filter(|d| d.kind == "web_assistant") {
            if headers.contains_key(header::AUTHORIZATION) || query.authorization.is_some() {
                return Err(Rejection::AmbiguousAuthentication);
            }
            let ticket = query.ticket.as_deref().ok_or(Rejection::TicketRequired)?;
            let origin = crate::app::assistant::trusted_origin(state, headers)
                .ok_or(Rejection::UntrustedOrigin)?;
            match assistant_auth.consume(ticket, device, client_id, &origin) {
                Ok(connection) => Some(connection),
                Err("assistant_capacity_exceeded") => {
                    return Err(Rejection::AssistantCapacityExceeded);
                }
                Err(_) => return Err(Rejection::InvalidTicket),
            }
        } else {
            if query.ticket.is_some() {
                return Err(Rejection::TicketRequiresManagedDevice);
            }
            if !config.auth.token.is_empty()
                && !bearer_matches(headers, query.authorization.as_deref(), &config.auth.token)
            {
                return Err(Rejection::InvalidBearerToken);
            }
            None
        };
    // A row re-read under the guard that is no longer the Web Assistant releases it here.
    let publication = assistant_connection.as_ref().and(publication);
    let routing_timer = metrics
        .as_ref()
        .map(|metrics| metrics.timer(RuntimePhase::WsRouting));
    match crate::app::enrollment::route(state, device_id, client_id).await {
        Ok(crate::app::enrollment::Route::Voice) => {}
        Ok(crate::app::enrollment::Route::Pending(connection)) => {
            return Ok(Admission::Enrollment(connection));
        }
        Err(response) => return Err(Rejection::DeviceRouteRejected(response)),
    }
    drop(routing_timer);
    // One immutable Effective Session Profile per connection.  Resolution is fail-closed: a
    // database-backed Agent whose default Template cannot be materialized is never silently
    // downgraded to the deployment's server defaults.
    let profile = state
        .resolve_session_profile(device_id)
        .await
        .map_err(|error| match error {
            SessionProfileAdmissionError::Runtime(error) => {
                Rejection::ProfileRuntimeUnavailable(error.to_string())
            }
            SessionProfileAdmissionError::Denied => Rejection::DeviceNotAdmitted,
            SessionProfileAdmissionError::AdmissionUnavailable => {
                Rejection::DeviceAdmissionUnavailable
            }
            SessionProfileAdmissionError::ProfileUnavailable => {
                debug!(
                    device_id = %device_id,
                    "the admitted agent has no usable effective session profile"
                );
                Rejection::ProfileUnavailable
            }
            SessionProfileAdmissionError::ShuttingDown => Rejection::ShuttingDown,
        })?;
    if assistant_connection.is_some()
        && let Some(guard) = &profile.external_mcp().guard
    {
        assistant_auth.register_close(&guard.close);
    }
    let system_mcp = match &assistant_connection {
        Some(_) => resolve_system_mcp(state, &assistant_auth, &profile).await,
        None => None,
    };
    let resolved_runtimes = profile
        .selected_runtimes
        .clone()
        .map(Ok)
        .unwrap_or_else(|| state.runtimes.resolve(&profile.providers))
        .map_err(|error| {
            debug!(%error, "configured provider runtime is unavailable");
            Rejection::ConfiguredRuntimeUnavailable
        })?;
    // One session identity, used both as the Voice Session's own id and as the archive's session
    // key: a transcript row groups by the same WebSocket connection, so a second database-side
    // session identity would be a second thing to keep in step.
    let session_id = Uuid::new_v4().to_string();
    if let Some(ref conn) = assistant_connection
        && !profile.agent_key.is_empty()
    {
        assistant_auth.register_session(session_id.clone(), profile.agent_key.clone(), conn);
    }
    let reminder_tools =
        resolve_reminder_tools(state, &profile, device_id, assistant_connection.as_ref()).await;
    let transcript = state.transcript_capture(&session_id, &profile);
    // Resolve the immutable, Agent-scoped candidate snapshot before the upgrade closure owns state.
    let speaker_timer = metrics
        .as_ref()
        .map(|metrics| metrics.timer(RuntimePhase::WsSpeaker));
    let speaker_observe = resolve_speaker_observe(state, &profile, config).await;
    drop(speaker_timer);
    // Publication protected credential validation through connection registration; the snapshot
    // is complete, so a System save may proceed.
    drop(publication);
    Ok(Admission::Voice(Box::new(SocketRuntimes {
        job_submit: crate::services::task_flow::VoiceTaskSubmit::admitted(
            state.task_manager.as_ref(),
            profile.device_db_id,
            profile.agent_id,
        ),
        asr: resolved_runtimes.asr,
        vad: resolved_runtimes.vad,
        llm: resolved_runtimes.llm,
        tts: resolved_runtimes.tts,
        vad_segmenter: resolved_runtimes.vad_segmenter,
        vad_pre_roll_samples: resolved_runtimes.vad_pre_roll_samples,
        active_turn_limiter: state.active_turn_limiter.clone(),
        pilot_admission: state.pilot_admission.clone(),
        writer_probe: state.writer_outcome_probe.clone(),
        profile,
        session_id,
        transcript,
        speaker_observe,
        admission_gate: Arc::clone(state.admission_gate()),
        connect_started,
        metrics,
        assistant_connection,
        system_mcp,
        reminder_tools,
        active_device_sessions: state.active_device_sessions.clone(),
        database: state.database.clone(),
    })))
}

/// Validates the protocol identity headers (or their browser query fallbacks) and returns the
/// exact Device ID and Client ID.
fn protocol_identity<'a>(
    headers: &'a HeaderMap,
    query: &'a WebsocketQuery,
) -> Result<(&'a str, &'a str), Rejection> {
    for name in [
        "protocol-version",
        "device-id",
        "client-id",
        "authorization",
    ] {
        if headers.get_all(name).iter().count() > 1 {
            return Err(Rejection::DuplicateProtocolHeader);
        }
    }
    for (name, value) in [
        ("protocol-version", &query.protocol_version),
        ("device-id", &query.device_id),
        ("client-id", &query.client_id),
        ("authorization", &query.authorization),
    ] {
        if headers.contains_key(name) && value.is_some() {
            return Err(Rejection::AmbiguousIdentity);
        }
    }
    if !header_or_query_is_or_absent(
        headers,
        "protocol-version",
        query.protocol_version.as_deref(),
        "1",
    ) || !header_or_query_present(headers, "device-id", query.device_id.as_deref())
        || !header_or_query_present(headers, "client-id", query.client_id.as_deref())
    {
        return Err(Rejection::InvalidProtocolHeaders);
    }
    let device_id = header_or_query_value(headers, "device-id", query.device_id.as_deref())
        .expect("validated above");
    if !valid_device_identity(device_id) {
        return Err(Rejection::InvalidDeviceId);
    }
    let client_id = header_or_query_value(headers, "client-id", query.client_id.as_deref())
        .expect("validated above");
    if !valid_device_identity(client_id) {
        return Err(Rejection::InvalidClientId);
    }
    Ok((device_id, client_id))
}

async fn lookup_device(
    state: &AppState,
    device_id: &str,
) -> Result<Option<crate::database::devices::Device>, Rejection> {
    let Some(db) = &state.database else {
        return Ok(None);
    };
    match crate::database::devices::get_device_by(db, device_id).await {
        Ok(device) => Ok(Some(device)),
        Err(sqlx::Error::RowNotFound) => Ok(None),
        Err(_) => Err(Rejection::DeviceLookupUnavailable),
    }
}

/// Only the consumed, origin-bound ticket plus managed admission can create this capability.
async fn resolve_system_mcp(
    state: &AppState,
    assistant_auth: &crate::app::assistant::WebAssistantAuth,
    profile: &crate::session::EffectiveSessionProfile,
) -> Option<crate::tools::system_mcp::SessionSystemMcp> {
    if profile.device_db_id <= 0 || profile.agent_id <= 0 {
        return None;
    }
    let (Some(core), Some(database)) = (&state.system_mcp, &state.database) else {
        return None;
    };
    let _publication = database.tool_security.publication.read().await;
    let settings = database
        .system_mcp_grants(state.config.system_mcp.enabled)
        .await
        .ok()?;
    let context = crate::tools::system_mcp::SystemToolContext {
        caller: crate::tools::system_mcp::SystemCaller::WebAssistant {
            device_db_id: profile.device_db_id,
            agent_id: profile.agent_id,
        },
        grants: settings.grants,
        revoked: database.tool_security.register_system_mcp(profile.agent_id),
        cancellation: CancellationToken::new(),
        deadline: tokio::time::Instant::now(),
    };
    assistant_auth.register_close(&context.revoked);
    Some(crate::tools::system_mcp::SessionSystemMcp {
        core: core.clone(),
        tools: core.list_tools(&context),
        context,
    })
}

/// The Voice Task reminder tools, when the Agent holds the reminder grant.
async fn resolve_reminder_tools(
    state: &AppState,
    profile: &crate::session::EffectiveSessionProfile,
    device_id: &str,
    assistant_connection: Option<&crate::app::assistant::Connection>,
) -> Option<crate::services::notifications::SessionReminderTools> {
    let database = state.database.as_ref()?;
    if profile.device_db_id <= 0 || profile.agent_id <= 0 {
        return None;
    }
    let Ok((true, _)) =
        crate::database::notifications::DeviceNotifications::new(database.as_ref().clone())
            .grant(&profile.agent_key)
            .await
    else {
        return None;
    };
    let coordinator = crate::services::notifications::ReminderCoordinator::from_database(
        database.as_ref().clone(),
        Arc::clone(&state.active_device_sessions),
    );
    let target = match assistant_connection {
        Some(conn) => crate::services::notifications::SessionReminderTarget::WebAssistant {
            assistant_device_id: device_id.to_owned(),
            bound_target: Arc::clone(&conn.reminder_target),
        },
        None => crate::services::notifications::SessionReminderTarget::Physical {
            device_row_id: profile.device_db_id,
            device_id: device_id.to_owned(),
        },
    };
    Some(crate::services::notifications::SessionReminderTools {
        coordinator,
        agent_id: profile.agent_id,
        agent_key: profile.agent_key.clone(),
        default_timezone: state.config.orchestrator.default_timezone.clone(),
        target,
    })
}

/// Install optional, Agent-scoped speaker identification. No voice-based admission
/// or tool gate exists: inference and resolution failures never refuse the session.
async fn resolve_speaker_observe(
    state: &AppState,
    profile: &crate::session::EffectiveSessionProfile,
    config: &AppConfig,
) -> Option<Arc<SpeakerObserve>> {
    let database = state.database.as_ref()?;
    let policy = match crate::session::resolve_speaker_policy(database, profile.agent_id).await {
        Ok(Some(mode)) => mode,
        Ok(None) => crate::session::SpeakerPolicyMode::Off,
        Err(cause) => {
            warn!(
                subsystem = "voice.speaker",
                event = "speaker_policy_unavailable",
                %cause,
                "speaker identification preference unavailable"
            );
            return None;
        }
    };
    let enabled = policy == crate::session::SpeakerPolicyMode::Observe;
    let runtime = state.speaker_runtime.as_ref();
    if !enabled || runtime.is_none() {
        info!(
            subsystem = "voice.speaker",
            event = "speaker_observe_configured",
            agent_key = %profile.agent_key,
            speaker_enabled = enabled,
            engine_available = runtime.is_some(),
            candidate_count = 0,
            "ws speaker identification configuration"
        );
        return None;
    }
    let runtime = runtime.expect("checked built-in speaker runtime availability");
    let plan = match crate::session::resolve_observe_plan(
        database,
        profile.agent_id,
        0,
        runtime.embedding_space_id(),
    )
    .await
    {
        Ok(crate::session::ObserveResolution::Plan(plan)) => plan,
        Ok(_) => {
            info!(
                subsystem = "voice.speaker",
                event = "speaker_observe_configured",
                agent_key = %profile.agent_key,
                speaker_enabled = true,
                engine_available = true,
                candidate_count = 0,
                "ws speaker identification configuration"
            );
            return None;
        }
        Err(cause) => {
            warn!(
                subsystem = "voice.speaker",
                event = "speaker_candidates_unavailable",
                %cause,
                "speaker candidates unavailable, continuing without identification"
            );
            return None;
        }
    };
    if plan.candidates.len() > config.speaker_recognition.max_candidates_per_agent {
        warn!(
            subsystem = "voice.speaker",
            event = "speaker_candidate_cap_exceeded",
            agent_key = %profile.agent_key,
            "speaker candidate cap exceeded"
        );
        return None;
    }
    info!(
        subsystem = "voice.speaker",
        event = "speaker_observe_configured",
        agent_key = %profile.agent_key,
        speaker_enabled = true,
        engine_available = true,
        candidate_count = plan.candidates.len(),
        "ws speaker identification configuration"
    );
    let observe_config = &config.speaker_recognition.observe;
    let quality = crate::audio::enrollment::QualityProfile {
        min_clip_ms: observe_config.min_clip_ms,
        max_clip_ms: 6_000, // Observe PCM buffer hard cap.
        min_speech_ms: observe_config.min_speech_ms,
        max_window_ms: observe_config.max_window_ms,
    };
    Some(Arc::new(SpeakerObserve::new_builtin(
        Arc::clone(runtime),
        plan,
        quality,
        config.speaker_recognition.similarity_threshold,
        observe_config.join_timeout_ms,
    )))
}

fn header_is(headers: &HeaderMap, name: &str, expected: &str) -> bool {
    headers.get(name).and_then(|value| value.to_str().ok()) == Some(expected)
}

/// Browser clients cannot set custom headers; legacy clients may omit the version entirely.
fn header_or_query_is_or_absent(
    headers: &HeaderMap,
    header_name: &str,
    query_value: Option<&str>,
    expected: &str,
) -> bool {
    match headers.get(header_name) {
        Some(_) => header_is(headers, header_name, expected),
        None => query_value.is_none_or(|value| value == expected),
    }
}

/// The Voice bearer credential, from the header or the browser query fallback.
fn bearer_matches(headers: &HeaderMap, query_value: Option<&str>, token: &str) -> bool {
    let expected = format!("Bearer {token}");
    match headers.get(header::AUTHORIZATION) {
        Some(_) => header_is(headers, header::AUTHORIZATION.as_str(), &expected),
        None => query_value == Some(expected.as_str()),
    }
}

/// Browser WebSocket APIs cannot set custom headers, so accept identity query fallbacks.
fn header_or_query_present(
    headers: &HeaderMap,
    header_name: &str,
    query_value: Option<&str>,
) -> bool {
    match headers.get(header_name) {
        Some(value) => value.to_str().is_ok_and(|value| !value.is_empty()),
        None => query_value.is_some_and(|value| !value.is_empty()),
    }
}

fn header_or_query_value<'a>(
    headers: &'a HeaderMap,
    header_name: &str,
    query_value: Option<&'a str>,
) -> Option<&'a str> {
    headers
        .get(header_name)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .or(query_value.filter(|value| !value.is_empty()))
}

/// Device identity is an opaque protocol key: bounded, printable and never normalized before
/// admission so the database unique key retains the exact client-provided identity.
fn valid_device_identity(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && !value
            .bytes()
            .any(|byte| byte == b'\0' || byte.is_ascii_control())
}
