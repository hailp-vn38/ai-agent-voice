use std::time::{Duration, Instant};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::rand::{SecureRandom, SystemRandom};
use sha2::{Digest, Sha256};
use url::Url;

use super::types::{
    GatewayAccountView, GatewayError, GatewayVendor, OAuthCallbackSubmission, OAuthLoginMode,
};

pub const OAUTH_SESSION_TTL: Duration = Duration::from_secs(600);

#[derive(Debug, Clone)]
pub enum OAuthSessionFlowState {
    Pkce {
        state: String,
        code_verifier: String,
        redirect_uri: String,
        completed_account: Option<GatewayAccountView>,
        failure: Option<String>,
    },
    DeviceCode {
        device_code: String,
        user_code: String,
        verification_uri: String,
        poll_interval_secs: u64,
        completed_account: Option<GatewayAccountView>,
        failure: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct OAuthLoginSession {
    pub session_id: String,
    pub vendor: GatewayVendor,
    pub label: String,
    pub created_at: Instant,
    pub expires_at: Instant,
    pub flow: OAuthSessionFlowState,
}

impl OAuthLoginSession {
    pub fn is_expired(&self, now: Instant) -> bool {
        now >= self.expires_at
    }

    pub const fn mode(&self) -> OAuthLoginMode {
        match &self.flow {
            OAuthSessionFlowState::Pkce { .. } => OAuthLoginMode::Pkce,
            OAuthSessionFlowState::DeviceCode { .. } => OAuthLoginMode::DeviceCode,
        }
    }
}

/// Generates a RFC 7636 PKCE `(code_verifier, code_challenge)` pair using S256.
pub fn generate_pkce_pair() -> Result<(String, String), GatewayError> {
    let mut verifier_bytes = [0u8; 48];
    SystemRandom::new()
        .fill(&mut verifier_bytes)
        .map_err(|_| GatewayError::CredentialStorageUnavailable)?;
    let code_verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);
    let digest = Sha256::digest(code_verifier.as_bytes());
    let code_challenge = URL_SAFE_NO_PAD.encode(digest);
    Ok((code_verifier, code_challenge))
}

/// Generates a 24-byte URL-safe random OAuth `state` parameter.
pub fn generate_state_token() -> Result<String, GatewayError> {
    let mut bytes = [0u8; 24];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| GatewayError::CredentialStorageUnavailable)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// Extracts `(code, Option<state>)` from:
/// 1. A full callback URL (`http://localhost:54545/callback?code=abc&state=xyz` or `...?code=abc#xyz`)
/// 2. Anthropic's `code#state` CLI string
/// 3. A raw authorization code
pub fn parse_oauth_callback_submission(
    submission: &OAuthCallbackSubmission,
) -> Result<(String, Option<String>), GatewayError> {
    let raw = submission.callback_url_or_code.trim();
    if raw.is_empty() || raw.len() > 4096 {
        return Err(GatewayError::OAuthCallbackInvalid);
    }

    if (raw.starts_with("http://") || raw.starts_with("https://"))
        && let Ok(parsed_url) = Url::parse(raw)
    {
        if parsed_url
            .query_pairs()
            .any(|(k, v)| k == "error" && !v.is_empty())
        {
            return Err(GatewayError::OAuthCallbackInvalid);
        }
        let mut code: Option<String> = None;
        let mut state: Option<String> = submission.state.clone();
        for (k, v) in parsed_url.query_pairs() {
            match k.as_ref() {
                "code" if !v.trim().is_empty() => code = Some(v.trim().to_owned()),
                "state" if !v.trim().is_empty() => state = Some(v.trim().to_owned()),
                _ => {}
            }
        }
        if state.is_none()
            && let Some(fragment) = parsed_url.fragment()
        {
            let frag = fragment.trim();
            if !frag.is_empty() {
                if frag.contains('=') {
                    for (k, v) in url::form_urlencoded::parse(frag.as_bytes()) {
                        if k == "code" && code.is_none() && !v.trim().is_empty() {
                            code = Some(v.trim().to_owned());
                        } else if k == "state" && state.is_none() && !v.trim().is_empty() {
                            state = Some(v.trim().to_owned());
                        }
                    }
                } else {
                    state = Some(frag.to_owned());
                }
            }
        }
        let Some(code) = code else {
            return Err(GatewayError::OAuthCallbackInvalid);
        };
        return Ok((code, state));
    }

    if let Some((code_part, state_part)) = raw.split_once('#') {
        let code = code_part.trim();
        let state = state_part.trim();
        if code.is_empty() {
            return Err(GatewayError::OAuthCallbackInvalid);
        }
        let resolved_state = if !state.is_empty() {
            Some(state.to_owned())
        } else {
            submission.state.clone()
        };
        return Ok((code.to_owned(), resolved_state));
    }

    if raw.bytes().any(|b| b.is_ascii_whitespace()) {
        return Err(GatewayError::OAuthCallbackInvalid);
    }

    Ok((raw.to_owned(), submission.state.clone()))
}
