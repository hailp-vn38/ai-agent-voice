//! Deployment-owned secret boundary; database rows retain only opaque references.
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
