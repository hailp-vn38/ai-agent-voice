use super::*;

impl LlmGatewayHub {
    pub async fn start_oauth_login(
        &self,
        req: StartOAuthLoginRequest,
    ) -> Result<OAuthLoginChallenge, GatewayError> {
        let label = req.label.trim();
        if label.is_empty() || label.len() > 256 {
            return Err(GatewayError::ValidationFailed);
        }
        let driver = self.driver_for(req.vendor)?;
        let session_id = format!("oas_{}", uuid::Uuid::new_v4().simple());
        let now = Instant::now();
        self.prune_expired_sessions(now);

        match req.vendor.login_mode() {
            OAuthLoginMode::Pkce => {
                let (code_verifier, code_challenge) = generate_pkce_pair()?;
                let state = generate_state_token()?;
                let redirect_uri = driver.resolve_redirect_uri(req.redirect_uri.as_deref());
                let authorize_url =
                    driver.build_pkce_authorize_url(&state, &code_challenge, &redirect_uri)?;
                let session = OAuthLoginSession {
                    session_id: session_id.clone(),
                    vendor: req.vendor,
                    label: label.to_owned(),
                    created_at: now,
                    expires_at: now + OAUTH_SESSION_TTL,
                    flow: OAuthSessionFlowState::Pkce {
                        state,
                        code_verifier,
                        redirect_uri,
                        completed_account: None,
                        failure: None,
                    },
                };
                self.sessions
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(session_id.clone(), session);
                Ok(OAuthLoginChallenge {
                    session_id,
                    vendor: req.vendor,
                    mode: OAuthLoginMode::Pkce,
                    authorize_url: Some(authorize_url),
                    verification_uri: None,
                    user_code: None,
                    poll_interval_secs: 2,
                    expires_in_secs: OAUTH_SESSION_TTL.as_secs(),
                })
            }
            OAuthLoginMode::DeviceCode => {
                let grant = driver.start_device_flow().await?;
                let ttl = Duration::from_secs(grant.expires_in_secs.clamp(60, 1800));
                let session = OAuthLoginSession {
                    session_id: session_id.clone(),
                    vendor: req.vendor,
                    label: label.to_owned(),
                    created_at: now,
                    expires_at: now + ttl,
                    flow: OAuthSessionFlowState::DeviceCode {
                        device_code: grant.device_code,
                        user_code: grant.user_code.clone(),
                        verification_uri: grant.verification_uri.clone(),
                        poll_interval_secs: grant.interval_secs.max(1),
                        next_poll_at: None,
                        completed_account: None,
                        failure: None,
                    },
                };
                self.sessions
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(session_id.clone(), session);
                Ok(OAuthLoginChallenge {
                    session_id,
                    vendor: req.vendor,
                    mode: OAuthLoginMode::DeviceCode,
                    authorize_url: None,
                    verification_uri: Some(grant.verification_uri),
                    user_code: Some(grant.user_code),
                    poll_interval_secs: grant.interval_secs.max(1),
                    expires_in_secs: ttl.as_secs(),
                })
            }
            OAuthLoginMode::CookiePaste => Err(GatewayError::ValidationFailed),
        }
    }

    /// Creates an account directly from pasted session cookies; there is no login session to poll.
    pub async fn import_cookie_account(
        &self,
        req: ImportCookieAccountRequest,
    ) -> Result<GatewayAccountView, GatewayError> {
        let label = req.label.trim();
        if label.is_empty()
            || label.len() > 256
            || req.vendor.login_mode() != OAuthLoginMode::CookiePaste
        {
            return Err(GatewayError::ValidationFailed);
        }
        let cookies = req.cookies.trim();
        if cookies.is_empty() || cookies.len() > MAX_PASTED_COOKIES_BYTES {
            return Err(GatewayError::OAuthCallbackInvalid);
        }
        let exchanged = self
            .driver_for(req.vendor)?
            .import_cookie_session(cookies)
            .await?;
        self.save_new_account(req.vendor, label, exchanged).await
    }

    pub async fn complete_oauth_login(
        &self,
        session_id: &str,
        submission: OAuthCallbackSubmission,
    ) -> Result<GatewayAccountView, GatewayError> {
        let (vendor, label, expected_state, code_verifier, redirect_uri) = {
            let mut guard = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
            let session = guard
                .get_mut(session_id)
                .ok_or(GatewayError::OAuthSessionNotFound)?;
            if session.is_expired(Instant::now()) {
                guard.remove(session_id);
                return Err(GatewayError::OAuthSessionExpired);
            }
            match &session.flow {
                OAuthSessionFlowState::Pkce {
                    state,
                    code_verifier,
                    redirect_uri,
                    completed_account: Some(acc),
                    ..
                } => {
                    let _ = (state, code_verifier, redirect_uri);
                    return Ok(acc.clone());
                }
                OAuthSessionFlowState::Pkce {
                    state,
                    code_verifier,
                    redirect_uri,
                    completed_account: None,
                    ..
                } => (
                    session.vendor,
                    session.label.clone(),
                    state.clone(),
                    code_verifier.clone(),
                    redirect_uri.clone(),
                ),
                OAuthSessionFlowState::DeviceCode { .. } => {
                    return Err(GatewayError::OAuthCallbackInvalid);
                }
            }
        };

        let (code, submitted_state) = parse_oauth_callback_submission(&submission)?;

        if let Some(given_state) = submitted_state.as_deref()
            && given_state != expected_state
        {
            return Err(GatewayError::OAuthStateMismatch);
        }

        let driver = self.driver_for(vendor)?;
        let exchanged = driver
            .exchange_pkce_code(&code, &expected_state, &code_verifier, &redirect_uri)
            .await?;
        let account_view = self.save_new_account(vendor, &label, exchanged).await?;

        let mut guard = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(session) = guard.get_mut(session_id)
            && let OAuthSessionFlowState::Pkce {
                completed_account, ..
            } = &mut session.flow
        {
            *completed_account = Some(account_view.clone());
        }
        Ok(account_view)
    }

    pub async fn complete_oauth_by_state(
        &self,
        state_param: &str,
        code: &str,
    ) -> Result<(String, GatewayAccountView), GatewayError> {
        let session_id = {
            let guard = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
            guard
                .values()
                .find_map(|s| match &s.flow {
                    OAuthSessionFlowState::Pkce { state, .. } if state == state_param => {
                        Some(s.session_id.clone())
                    }
                    _ => None,
                })
                .ok_or(GatewayError::OAuthSessionNotFound)?
        };
        match self
            .complete_oauth_login(
                &session_id,
                OAuthCallbackSubmission {
                    callback_url_or_code: code.to_owned(),
                    state: Some(state_param.to_owned()),
                },
            )
            .await
        {
            Ok(view) => Ok((session_id, view)),
            Err(err) => {
                self.record_oauth_failure(&session_id, err.to_string());
                Err(err)
            }
        }
    }

    pub fn record_oauth_failure_by_state(&self, state_param: &str, message: impl Into<String>) {
        let msg = message.into();
        let mut guard = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
        for session in guard.values_mut() {
            if let OAuthSessionFlowState::Pkce {
                state,
                completed_account: None,
                failure,
                ..
            } = &mut session.flow
                && state == state_param
            {
                *failure = Some(msg);
                break;
            }
        }
    }

    pub(super) fn record_oauth_failure(&self, session_id: &str, message: String) {
        let mut guard = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(session) = guard.get_mut(session_id)
            && let OAuthSessionFlowState::Pkce {
                completed_account: None,
                failure,
                ..
            } = &mut session.flow
        {
            *failure = Some(message);
        }
    }

    pub async fn poll_oauth_login(
        &self,
        session_id: &str,
    ) -> Result<OAuthPollStatus, GatewayError> {
        enum Action {
            Return(OAuthPollStatus),
            PollDevice {
                vendor: GatewayVendor,
                label: String,
                device_code: String,
                poll_interval_secs: u64,
                wait_before_poll: Option<Duration>,
            },
        }

        let action = {
            let mut guard = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
            let session = guard
                .get_mut(session_id)
                .ok_or(GatewayError::OAuthSessionNotFound)?;
            let now = Instant::now();
            if session.is_expired(now) {
                guard.remove(session_id);
                return Err(GatewayError::OAuthSessionExpired);
            }
            let vendor = session.vendor;
            let label = session.label.clone();
            match &mut session.flow {
                OAuthSessionFlowState::Pkce {
                    completed_account: Some(acc),
                    ..
                }
                | OAuthSessionFlowState::DeviceCode {
                    completed_account: Some(acc),
                    ..
                } => Action::Return(OAuthPollStatus::Completed {
                    account: Box::new(acc.clone()),
                }),
                OAuthSessionFlowState::Pkce {
                    failure: Some(err), ..
                }
                | OAuthSessionFlowState::DeviceCode {
                    failure: Some(err), ..
                } => Action::Return(OAuthPollStatus::Failed { error: err.clone() }),
                OAuthSessionFlowState::Pkce { .. } => Action::Return(OAuthPollStatus::Pending {
                    poll_interval_secs: 2,
                }),
                OAuthSessionFlowState::DeviceCode {
                    device_code,
                    poll_interval_secs,
                    next_poll_at,
                    ..
                } => {
                    let code = device_code.clone();
                    let interval = *poll_interval_secs;
                    if let Some(not_before) = *next_poll_at
                        && now < not_before
                    {
                        let remaining = not_before.duration_since(now);
                        if remaining > Duration::from_millis(1500) {
                            Action::Return(OAuthPollStatus::Pending {
                                poll_interval_secs: remaining.as_secs().max(1),
                            })
                        } else {
                            *next_poll_at = Some(not_before + Duration::from_secs(interval));
                            Action::PollDevice {
                                vendor,
                                label,
                                device_code: code,
                                poll_interval_secs: interval,
                                wait_before_poll: Some(remaining),
                            }
                        }
                    } else {
                        *next_poll_at = Some(now + Duration::from_secs(interval));
                        Action::PollDevice {
                            vendor,
                            label,
                            device_code: code,
                            poll_interval_secs: interval,
                            wait_before_poll: None,
                        }
                    }
                }
            }
        };

        match action {
            Action::Return(status) => Ok(status),
            Action::PollDevice {
                vendor,
                label,
                device_code,
                poll_interval_secs,
                wait_before_poll,
            } => {
                if let Some(delay) = wait_before_poll {
                    tokio::time::sleep(delay).await;
                }
                let driver = self.driver_for(vendor)?;
                match driver.poll_device_flow(&device_code).await? {
                    DevicePollOutcome::Pending { next_interval_secs } => {
                        let (next_interval, schedule_guard) = match next_interval_secs {
                            Some(0) => (poll_interval_secs.max(1), false),
                            Some(n) if n >= 10 => {
                                let bumped = if n <= poll_interval_secs {
                                    (poll_interval_secs + 5).min(30)
                                } else {
                                    n.min(30)
                                };
                                (bumped, true)
                            }
                            Some(n) => (n.max(poll_interval_secs).max(1), true),
                            None => (poll_interval_secs.max(1), true),
                        };
                        let mut guard = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
                        if let Some(session) = guard.get_mut(session_id)
                            && let OAuthSessionFlowState::DeviceCode {
                                poll_interval_secs: ref mut stored,
                                next_poll_at: ref mut stored_next_poll_at,
                                ..
                            } = session.flow
                        {
                            *stored = next_interval;
                            *stored_next_poll_at = schedule_guard.then(|| {
                                Instant::now()
                                    + Duration::from_secs(next_interval)
                                    + Duration::from_millis(200)
                            });
                        }
                        Ok(OAuthPollStatus::Pending {
                            poll_interval_secs: next_interval,
                        })
                    }
                    DevicePollOutcome::Completed(exchanged) => {
                        let view = self.save_new_account(vendor, &label, exchanged).await?;
                        let mut guard = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
                        if let Some(session) = guard.get_mut(session_id)
                            && let OAuthSessionFlowState::DeviceCode {
                                completed_account, ..
                            } = &mut session.flow
                        {
                            *completed_account = Some(view.clone());
                        }
                        Ok(OAuthPollStatus::Completed {
                            account: Box::new(view),
                        })
                    }
                    DevicePollOutcome::Failed(err) => {
                        let mut guard = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
                        if let Some(session) = guard.get_mut(session_id)
                            && let OAuthSessionFlowState::DeviceCode { failure, .. } =
                                &mut session.flow
                        {
                            *failure = Some(err.clone());
                        }
                        Ok(OAuthPollStatus::Failed { error: err })
                    }
                }
            }
        }
    }

    pub(super) async fn save_new_account(
        &self,
        vendor: GatewayVendor,
        label: &str,
        exchanged: ExchangedOAuthAccount,
    ) -> Result<GatewayAccountView, GatewayError> {
        let short_id = &uuid::Uuid::new_v4().simple().to_string()[..8];
        let account_key = format!("gw_{}_{short_id}", vendor.as_str());
        let credential_json = exchanged
            .bundle
            .seal(self.secret_resolver.as_ref(), &account_key)?;
        let now_epoch = unix_seconds().unwrap_or(0);
        let extra_headers = self
            .drivers
            .get(&vendor)
            .map(|drv| drv.extra_headers_for(&exchanged.bundle))
            .unwrap_or_default();

        if let Some(pool) = &self.pool {
            sqlx::query(
                "INSERT INTO llm_gateway_accounts (
                    account_key, vendor, label, account_email, credential_json,
                    expires_at, enabled, priority, status, last_refreshed_at, created_at, updated_at
                 ) VALUES (?, ?, ?, ?, ?, ?, 1, 100, 'active', ?, ?, ?)",
            )
            .bind(&account_key)
            .bind(vendor.as_str())
            .bind(label)
            .bind(&exchanged.account_email)
            .bind(&credential_json)
            .bind(exchanged.expires_at)
            .bind(now_epoch)
            .bind(now_epoch)
            .bind(now_epoch)
            .execute(pool)
            .await
            .map_err(|_| GatewayError::DatabaseUnavailable)?;
        }

        {
            let mut guard = self.snapshot.write().unwrap_or_else(|p| p.into_inner());
            guard.accounts.insert(
                account_key.clone(),
                LoadedGatewayAccount {
                    account_key: account_key.clone(),
                    vendor,
                    label: label.to_owned(),
                    account_email: exchanged.account_email,
                    credential_json,
                    bundle: Some(exchanged.bundle),
                    extra_headers,
                    expires_at: exchanged.expires_at,
                    enabled: true,
                    priority: 100,
                    status: GatewayAccountStatus::Active,
                    last_error: None,
                    last_refreshed_at: Some(now_epoch),
                    last_used_at: None,
                    cooldown_until: None,
                },
            );
            guard.rebuild_vendor_index();
        }

        self.get_account_view(&account_key).await
    }

    pub(super) fn prune_expired_sessions(&self, now: Instant) {
        let mut guard = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
        guard.retain(|_, session| !session.is_expired(now));
    }
}
