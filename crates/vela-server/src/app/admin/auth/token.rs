//! HS256 Admin Web Token encoding and verification. The claims carry only issue time, expiry and
//! the password-login revision the token was minted under; nothing identifies a person.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::hmac;
use serde::{Deserialize, Serialize};

const MAX_TOKEN_BYTES: usize = 2048;
const HEADER_JSON: &str = r#"{"alg":"HS256","typ":"JWT"}"#;

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Claims {
    pub iat: i64,
    pub exp: i64,
    pub rev: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    alg: String,
    typ: String,
}

pub(super) fn sign(key: &hmac::Key, claims: &Claims) -> String {
    let header = URL_SAFE_NO_PAD.encode(HEADER_JSON);
    let body = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).unwrap_or_default());
    let signing_input = format!("{header}.{body}");
    let tag = hmac::sign(key, signing_input.as_bytes());
    format!("{signing_input}.{}", URL_SAFE_NO_PAD.encode(tag.as_ref()))
}

/// Returns the claims only when the signature, algorithm, type and expiry all check out. The
/// caller still compares `rev` against the live revision.
pub(super) fn verify(key: &hmac::Key, token: &str, now: i64) -> Option<Claims> {
    if token.len() > MAX_TOKEN_BYTES {
        return None;
    }
    let mut segments = token.split('.');
    let (Some(head), Some(body), Some(signature), None) = (
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
    ) else {
        return None;
    };
    let signing_input = &token[..head.len() + 1 + body.len()];
    let signature = URL_SAFE_NO_PAD.decode(signature).ok()?;
    hmac::verify(key, signing_input.as_bytes(), &signature).ok()?;
    let header: Header = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(head).ok()?).ok()?;
    if header.alg != "HS256" || header.typ != "JWT" {
        return None;
    }
    let claims: Claims = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(body).ok()?).ok()?;
    (claims.exp > now).then_some(claims)
}
