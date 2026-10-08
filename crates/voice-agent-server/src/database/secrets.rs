//! Deployment-owned secret boundary; SQLite never stores credential values or references.
use zeroize::Zeroizing;

#[derive(PartialEq, Eq)]
pub struct SecretRef(String);
impl SecretRef {
    pub fn parse(value: String) -> Result<Self, SecretRefError> {
        let bytes = value.as_bytes();
        if bytes.is_empty() || bytes.len() > 256 {
            return Err(SecretRefError::Length);
        }
        if bytes.iter().any(|b| !(0x20..=0x7e).contains(b)) {
            return Err(SecretRefError::Character);
        }
        if bytes.iter().all(|b| *b == b' ')
            || bytes.first() == Some(&b' ')
            || bytes.last() == Some(&b' ')
        {
            return Err(SecretRefError::Whitespace);
        }
        Ok(Self(value))
    }
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Debug for SecretRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretRef([REDACTED])")
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretRefError {
    Length,
    Character,
    Whitespace,
}
/// Environment names are derived exclusively from immutable resource identities.
/// No Admin mutation or database column chooses where secrets are read from.
fn env_key(key: &str) -> Option<String> {
    if key.is_empty()
        || !key
            .bytes()
            .all(|byte| byte == b'_' || byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        return None;
    }
    Some(key.to_ascii_uppercase())
}

/// Remote provider adapters obtain credentials from server deployment configuration.
pub fn provider_secret_env(key: &str, adapter: &str) -> Option<String> {
    if !matches!(adapter, "openai" | "chillaudio_ws") {
        return None;
    }
    Some(format!("VOICE_PROVIDER_{}_API_KEY", env_key(key)?))
}

/// An MCP server's auth scheme is persisted, but its credential source is not.
pub fn mcp_secret_env(key: &str, auth_type: &str) -> Option<String> {
    if !matches!(auth_type, "bearer" | "header") {
        return None;
    }
    Some(format!("VOICE_MCP_{}_TOKEN", env_key(key)?))
}

pub struct SecretValue(Zeroizing<String>);
impl SecretValue {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretValue([REDACTED])")
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretResolveError {
    Invalid,
    Unavailable,
}
pub trait SecretResolver: Send + Sync {
    fn resolve(&self, reference: &SecretRef) -> Result<SecretValue, SecretResolveError>;
}

/// V1 deployment resolver. Only this boundary interprets an in-memory reference as an
/// environment-variable name; Admin and SQLite never accept one.
#[derive(Default)]
pub struct EnvSecretResolver;

impl SecretResolver for EnvSecretResolver {
    fn resolve(&self, reference: &SecretRef) -> Result<SecretValue, SecretResolveError> {
        let name = reference.as_str();
        if !name
            .bytes()
            .all(|byte| byte == b'_' || byte.is_ascii_uppercase() || byte.is_ascii_digit())
        {
            return Err(SecretResolveError::Invalid);
        }
        std::env::var(name)
            .map(SecretValue::new)
            .map_err(|_| SecretResolveError::Unavailable)
    }
}

#[cfg(test)]
mod deployment_name_tests {
    use super::*;

    #[test]
    fn provider_credentials_are_derived_only_for_remote_adapters() {
        assert_eq!(
            provider_secret_env("llm_abc123", "openai").as_deref(),
            Some("VOICE_PROVIDER_LLM_ABC123_API_KEY")
        );
        assert_eq!(
            provider_secret_env("tts_v1", "chillaudio_ws").as_deref(),
            Some("VOICE_PROVIDER_TTS_V1_API_KEY")
        );
        assert!(provider_secret_env("vad_local", "silero_onnx").is_none());
        assert!(provider_secret_env("INVALID-NAME", "openai").is_none());
    }

    #[test]
    fn mcp_credentials_are_derived_only_for_authenticated_servers() {
        assert_eq!(
            mcp_secret_env("weather", "bearer").as_deref(),
            Some("VOICE_MCP_WEATHER_TOKEN")
        );
        assert_eq!(
            mcp_secret_env("home_automation", "header").as_deref(),
            Some("VOICE_MCP_HOME_AUTOMATION_TOKEN")
        );
        assert!(mcp_secret_env("weather", "none").is_none());
        assert!(mcp_secret_env("foo-bar", "bearer").is_none());
    }
}
