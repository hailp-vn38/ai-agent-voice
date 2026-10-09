//! AES-GCM encrypted resource credentials. Callers never handle nonce or key selection.
use std::collections::BTreeMap;

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use ring::{
    aead,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use zeroize::Zeroizing;

use super::secrets::{
    EnvSecretResolver, SecretRef, SecretResolveError, SecretResolver, SecretValue,
};

const PREFIX: &str = "sealed:";
const MAX_RECORD_BYTES: usize = 8192;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EncryptedCredential {
    id: String,
    encrypted_value: String,
    nonce: String,
    key_version: u32,
    key_hint: String,
}

pub fn valid_input(value: &SecretValue) -> bool {
    let bytes = value.expose().as_bytes();
    !bytes.is_empty() && bytes.len() <= 4096 && bytes.iter().all(u8::is_ascii_graphic)
}

/// A missing stored record keeps the existing deployment credential source.
/// An encrypted snapshot includes its expected owner, so swapping records fails authentication.
pub fn reference(scope: &str, record: Option<&str>, fallback: Option<String>) -> Option<String> {
    record
        .map(|record| format!("{PREFIX}{}:{}", BASE64.encode(scope), BASE64.encode(record)))
        .or(fallback)
}

pub fn metadata(record: Option<&str>) -> Value {
    let record = record
        .filter(|value| value.len() <= MAX_RECORD_BYTES)
        .and_then(|value| serde_json::from_str::<EncryptedCredential>(value).ok());
    match record {
        Some(record) if uuid::Uuid::parse_str(&record.id).is_ok() && record.key_version > 0 => {
            let valid_hint = record.key_hint == "••••"
                || ["sk-...", "…"].iter().any(|prefix| {
                    record.key_hint.strip_prefix(prefix).is_some_and(|tail| {
                        tail.len() == 4 && tail.bytes().all(|b| b.is_ascii_graphic())
                    })
                });
            if !valid_hint {
                return Value::Null;
            }
            serde_json::json!({"id":record.id,"masked_key":record.key_hint,"key_version":record.key_version,"status":"active"})
        }
        _ => Value::Null,
    }
}

pub struct CredentialCipher {
    current_version: u32,
    keys: BTreeMap<u32, aead::LessSafeKey>,
}

impl CredentialCipher {
    pub fn from_file(path: &std::path::Path) -> Result<Self, SecretResolveError> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct KeyFile {
            current_version: u32,
            keys: BTreeMap<u32, SecretValue>,
        }
        let bytes =
            Zeroizing::new(std::fs::read(path).map_err(|_| SecretResolveError::Unavailable)?);
        if bytes.len() > 65_536 {
            return Err(SecretResolveError::Invalid);
        }
        let file: KeyFile =
            serde_json::from_slice(&bytes).map_err(|_| SecretResolveError::Invalid)?;
        if file.current_version == 0 || file.keys.len() > 32 {
            return Err(SecretResolveError::Invalid);
        }
        let mut keys = BTreeMap::new();
        for (version, value) in file.keys {
            if version == 0 {
                return Err(SecretResolveError::Invalid);
            }
            let bytes = Zeroizing::new(
                BASE64
                    .decode(value.expose())
                    .map_err(|_| SecretResolveError::Invalid)?,
            );
            let key = aead::UnboundKey::new(&aead::AES_256_GCM, &bytes)
                .map_err(|_| SecretResolveError::Invalid)?;
            keys.insert(version, aead::LessSafeKey::new(key));
        }
        if !keys.contains_key(&file.current_version) {
            return Err(SecretResolveError::Unavailable);
        }
        Ok(Self {
            current_version: file.current_version,
            keys,
        })
    }

    pub fn new(version: u32, key: &[u8]) -> Result<Self, SecretResolveError> {
        if version == 0 {
            return Err(SecretResolveError::Invalid);
        }
        let key = aead::UnboundKey::new(&aead::AES_256_GCM, key)
            .map_err(|_| SecretResolveError::Invalid)?;
        Ok(Self {
            current_version: version,
            keys: BTreeMap::from([(version, aead::LessSafeKey::new(key))]),
        })
    }

    pub fn from_environment() -> Result<Self, SecretResolveError> {
        let current_version = match std::env::var("VOICE_CREDENTIAL_KEY_VERSION") {
            Ok(value) => value
                .parse::<u32>()
                .map_err(|_| SecretResolveError::Invalid)?,
            Err(std::env::VarError::NotPresent) => 1,
            Err(_) => return Err(SecretResolveError::Invalid),
        };
        if current_version == 0 {
            return Err(SecretResolveError::Invalid);
        }
        let mut keys = BTreeMap::new();
        for (name, value) in std::env::vars_os() {
            let Some(version) = name
                .to_str()
                .and_then(|name| name.strip_prefix("VOICE_CREDENTIAL_KEY_"))
                .and_then(|version| version.parse::<u32>().ok())
            else {
                continue;
            };
            if version == 0 || keys.len() >= 32 {
                return Err(SecretResolveError::Invalid);
            }
            let value = Zeroizing::new(
                value
                    .into_string()
                    .map_err(|_| SecretResolveError::Invalid)?,
            );
            let bytes = Zeroizing::new(
                BASE64
                    .decode(value.as_bytes())
                    .map_err(|_| SecretResolveError::Invalid)?,
            );
            let key = aead::UnboundKey::new(&aead::AES_256_GCM, &bytes)
                .map_err(|_| SecretResolveError::Invalid)?;
            if keys.insert(version, aead::LessSafeKey::new(key)).is_some() {
                return Err(SecretResolveError::Invalid);
            }
        }
        if !keys.contains_key(&current_version) {
            return Err(SecretResolveError::Unavailable);
        }
        Ok(Self {
            current_version,
            keys,
        })
    }

    fn encrypt(&self, value: &SecretValue, scope: &str) -> Result<String, SecretResolveError> {
        if !valid_input(value) {
            return Err(SecretResolveError::Invalid);
        }
        let id = uuid::Uuid::new_v4().to_string();
        let mut nonce = [0; 12];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| SecretResolveError::Unavailable)?;
        let aad =
            serde_json::to_vec(&("admin", scope, &id)).map_err(|_| SecretResolveError::Invalid)?;
        let mut buffer = Zeroizing::new(value.expose().as_bytes().to_vec());
        self.keys
            .get(&self.current_version)
            .ok_or(SecretResolveError::Unavailable)?
            .seal_in_place_append_tag(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(aad),
                &mut *buffer,
            )
            .map_err(|_| SecretResolveError::Unavailable)?;
        let raw = value.expose();
        let key_hint = if raw.len() <= 8 {
            "••••".to_owned()
        } else {
            format!(
                "{}{}",
                if raw.starts_with("sk-") {
                    "sk-..."
                } else {
                    "…"
                },
                &raw[raw.len() - 4..]
            )
        };
        serde_json::to_string(&EncryptedCredential {
            id,
            encrypted_value: BASE64.encode(&*buffer),
            nonce: BASE64.encode(nonce),
            key_version: self.current_version,
            key_hint,
        })
        .map_err(|_| SecretResolveError::Invalid)
    }

    fn decrypt(&self, reference: &SecretRef) -> Result<SecretValue, SecretResolveError> {
        let encoded = reference
            .as_str()
            .strip_prefix(PREFIX)
            .ok_or(SecretResolveError::Invalid)?;
        let (scope, record) = encoded.split_once(':').ok_or(SecretResolveError::Invalid)?;
        let scope = BASE64
            .decode(scope)
            .map_err(|_| SecretResolveError::Invalid)?;
        let scope = std::str::from_utf8(&scope).map_err(|_| SecretResolveError::Invalid)?;
        let record = BASE64
            .decode(record)
            .map_err(|_| SecretResolveError::Invalid)?;
        if record.len() > MAX_RECORD_BYTES {
            return Err(SecretResolveError::Invalid);
        }
        let record: EncryptedCredential =
            serde_json::from_slice(&record).map_err(|_| SecretResolveError::Invalid)?;
        uuid::Uuid::parse_str(&record.id).map_err(|_| SecretResolveError::Invalid)?;
        let nonce = BASE64
            .decode(record.nonce)
            .map_err(|_| SecretResolveError::Invalid)?;
        let nonce = aead::Nonce::try_assume_unique_for_key(&nonce)
            .map_err(|_| SecretResolveError::Invalid)?;
        let aad = serde_json::to_vec(&("admin", scope, &record.id))
            .map_err(|_| SecretResolveError::Invalid)?;
        let mut buffer = Zeroizing::new(
            BASE64
                .decode(record.encrypted_value)
                .map_err(|_| SecretResolveError::Invalid)?,
        );
        let plaintext = self
            .keys
            .get(&record.key_version)
            .ok_or(SecretResolveError::Unavailable)?
            .open_in_place(nonce, aead::Aad::from(aad), &mut buffer)
            .map_err(|_| SecretResolveError::Invalid)?;
        let secret = SecretValue::new(
            std::str::from_utf8(plaintext)
                .map_err(|_| SecretResolveError::Invalid)?
                .to_owned(),
        );
        if !valid_input(&secret) {
            return Err(SecretResolveError::Invalid);
        }
        Ok(secret)
    }
}

#[cfg(test)]
#[path = "credential_file_tests.rs"]
mod file_tests;

impl SecretResolver for CredentialCipher {
    fn resolve(&self, reference: &SecretRef) -> Result<SecretValue, SecretResolveError> {
        if reference.as_str().starts_with(PREFIX) {
            self.decrypt(reference)
        } else {
            EnvSecretResolver.resolve(reference)
        }
    }
    fn seal(&self, value: &SecretValue, scope: &str) -> Result<String, SecretResolveError> {
        self.encrypt(value, scope)
    }
}
