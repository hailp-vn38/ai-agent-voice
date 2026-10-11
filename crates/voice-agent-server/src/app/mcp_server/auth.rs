//! Dedicated HS256 resource tokens. No Admin/Voice bearer credential is accepted here.
use crate::{
    config::AppConfig,
    tools::system_mcp::{SystemCaller, SystemToolContext, SystemToolGrants},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::hmac;
use serde::Deserialize;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

pub(super) struct Verifier {
    key: hmac::Key,
    issuer: String,
    audience: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    alg: String,
    typ: String,
}
#[derive(Deserialize)]
struct Claims {
    iss: String,
    aud: String,
    sub: String,
    jti: String,
    scope: String,
    exp: i64,
    iat: i64,
    nbf: Option<i64>,
}
impl Verifier {
    pub(super) fn new(config: &AppConfig) -> Result<Self, ()> {
        let key = std::env::var("VOICE_SYSTEM_MCP_JWT_KEY").map_err(|_| ())?;
        Self::from_key(config, &key)
    }
    pub(super) fn from_key(config: &AppConfig, key: &str) -> Result<Self, ()> {
        let http = &config.system_mcp.http;
        if !config.system_mcp.enabled
            || key.len() < 32
            || key == config.api.admin_token
            || key == config.auth.token
            || !valid_https(&http.issuer, false)
            || !valid_https(&http.audience, true)
            || http.allowed_origins.len() > 16
            || http.allowed_origins.iter().any(|o| !valid_origin(o))
        {
            return Err(());
        }
        // HTTPS terminates at a trusted local proxy; this plaintext listener must stay loopback.
        if !config.server.bind.ip().is_loopback() {
            return Err(());
        }
        Ok(Self {
            key: hmac::Key::new(hmac::HMAC_SHA256, key.as_bytes()),
            issuer: http.issuer.clone(),
            audience: http.audience.clone(),
        })
    }
    pub(super) fn verify(&self, token: &str) -> Result<SystemToolContext, ()> {
        if token.len() > 8192 {
            return Err(());
        }
        let mut segments = token.split('.');
        let (Some(head), Some(body), Some(signature), None) = (
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
        ) else {
            return Err(());
        };
        let header: Header = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(head).map_err(|_| ())?)
            .map_err(|_| ())?;
        if header.alg != "HS256" || header.typ != "JWT" {
            return Err(());
        }
        hmac::verify(
            &self.key,
            format!("{head}.{body}").as_bytes(),
            &URL_SAFE_NO_PAD.decode(signature).map_err(|_| ())?,
        )
        .map_err(|_| ())?;
        let claims: Claims = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(body).map_err(|_| ())?)
            .map_err(|_| ())?;
        let now = crate::database::unix_seconds().ok_or(())?;
        if claims.iss != self.issuer
            || claims.aud != self.audience
            || !identity(&claims.sub)
            || !identity(&claims.jti)
            || claims.exp <= now
            || claims.iat > now
            || claims.iat < now.saturating_sub(3600)
            || claims.exp > claims.iat.saturating_add(3600)
            || claims.exp <= claims.iat
            || claims.nbf.is_some_and(|t| t > now)
            || claims.scope.len() > 256
        {
            return Err(());
        }
        let scopes: Vec<_> = claims.scope.split_ascii_whitespace().collect();
        let grants = SystemToolGrants {
            system_read: scopes.contains(&"system:read"),
            mcp_read: scopes.contains(&"mcp:read"),
            mcp_discover: scopes.contains(&"mcp:discover"),
            mcp_manage_request: scopes.contains(&"mcp:manage:request"),
        };
        if !grants.system_read
            && !grants.mcp_read
            && !grants.mcp_discover
            && !grants.mcp_manage_request
        {
            return Err(());
        }
        Ok(SystemToolContext {
            caller: SystemCaller::Remote {
                subject: claims.sub,
                jti: claims.jti,
                expires_at: claims.exp,
            },
            grants,
            revoked: Arc::new(CancellationToken::new()),
            cancellation: CancellationToken::new(),
            deadline: tokio::time::Instant::now() + Duration::from_secs(30),
        })
    }
}
fn identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && value.bytes().all(|b| b > 0x20 && b < 0x7f)
}
fn valid_https(value: &str, resource: bool) -> bool {
    url::Url::parse(value).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str().is_some()
            && u.username().is_empty()
            && u.password().is_none()
            && u.query().is_none()
            && u.fragment().is_none()
            && (!resource || u.path() == "/mcp/system")
    })
}
fn valid_origin(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|u| {
        valid_https(value, false) && u.path() == "/" && u.origin().ascii_serialization() == value
    })
}

#[cfg(test)]
mod tests;
