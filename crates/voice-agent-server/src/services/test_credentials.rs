//! Write-only credentials for one manual diagnostic; never sealed or persisted.
use crate::database::secrets::{SecretRef, SecretResolveError, SecretResolver, SecretValue};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedCredential {
    pub key: String,
    pub expected_revision: i64,
}

pub(crate) struct InlineCredential(pub SecretValue);
impl SecretResolver for InlineCredential {
    fn resolve(&self, reference: &SecretRef) -> Result<SecretValue, SecretResolveError> {
        if reference.as_str() != "DRAFT" {
            return Err(SecretResolveError::Invalid);
        }
        Ok(SecretValue::new(self.0.expose().to_owned()))
    }
}
