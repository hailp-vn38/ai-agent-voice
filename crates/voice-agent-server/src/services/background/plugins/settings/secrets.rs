//! Secret references for plugin value configurations.
//! CredentialCipher::seal yields an encrypted JSON *record*; the caller must
//! wrap that record in a scoped reference before persisting it.
use crate::database::{
    credentials::{self, CredentialCipher},
    secrets::{SecretRef, SecretResolver, SecretValue},
};
use crate::services::background::plugins::PluginError;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

pub(super) fn seal_ref(
    cipher: &CredentialCipher,
    value: &SecretValue,
    scope: &str,
) -> Result<String, PluginError> {
    let record = cipher
        .seal(value, scope)
        .map_err(|_| PluginError::PermissionRequired)?;
    credentials::reference(scope, Some(&record), None).ok_or(PluginError::PermissionRequired)
}

pub(super) fn open_ref(
    cipher: &CredentialCipher,
    stored: &str,
    scope: &str,
) -> Result<SecretValue, PluginError> {
    // Earlier implementation stored Cipher::seal()'s JSON record without
    // wrapping it in a reference. Accept only a valid authenticated record
    // for the pinned scope; the next write re-seals it in the correct format.
    let wrapped = if stored.starts_with('{') {
        credentials::reference(scope, Some(stored), None).ok_or(PluginError::PermissionRequired)?
    } else {
        stored.to_owned()
    };
    let prefix = format!("sealed:{}:", BASE64.encode(scope));
    if !wrapped.starts_with(&prefix) {
        return Err(PluginError::PermissionRequired);
    }
    let reference = SecretRef::parse(wrapped).map_err(|_| PluginError::PermissionRequired)?;
    cipher
        .resolve(&reference)
        .map_err(|_| PluginError::PermissionRequired)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cipher() -> CredentialCipher {
        CredentialCipher::new(1, &[17_u8; 32]).unwrap()
    }

    #[test]
    fn scoped_reference_roundtrips_and_rejects_cross_plugin_reuse() {
        let cipher = cipher();
        let secret = SecretValue::new("sk-example-secret".to_owned());
        let reference =
            seal_ref(&cipher, &secret, "plugin-value:one:1:schema:package:key").unwrap();
        assert!(reference.starts_with("sealed:"));
        assert_eq!(
            open_ref(&cipher, &reference, "plugin-value:one:1:schema:package:key")
                .unwrap()
                .expose(),
            secret.expose()
        );
        assert!(open_ref(&cipher, &reference, "plugin-value:two:1:schema:package:key").is_err());
        assert!(open_ref(&cipher, &reference, "plugin-value:one:2:schema:package:key").is_err());
    }

    #[test]
    fn previous_raw_encrypted_record_migrates_only_for_matching_scope() {
        let cipher = cipher();
        let scope = "plugin-value:one:1:schema:package:key";
        let old_record = cipher
            .seal(&SecretValue::new("sk-legacy-secret".to_owned()), scope)
            .unwrap();
        assert_eq!(
            open_ref(&cipher, &old_record, scope).unwrap().expose(),
            "sk-legacy-secret"
        );
        assert!(
            open_ref(
                &cipher,
                &old_record,
                "plugin-value:other:1:schema:package:key"
            )
            .is_err()
        );
        let migrated = seal_ref(
            &cipher,
            &open_ref(&cipher, &old_record, scope).unwrap(),
            "plugin-value:one:2:schema:package:key",
        )
        .unwrap();
        assert_eq!(
            open_ref(&cipher, &migrated, "plugin-value:one:2:schema:package:key")
                .unwrap()
                .expose(),
            "sk-legacy-secret"
        );
    }
}
