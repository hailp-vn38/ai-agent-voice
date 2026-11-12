//! PBKDF2-HMAC-SHA256 hashing for the Admin Password. Stored as
//! `pbkdf2_sha256$<iterations>$<salt>$<hash>` so the parameters travel with the hash.

use base64::{Engine, engine::general_purpose::STANDARD};
use ring::{pbkdf2, rand::SecureRandom};
use std::num::NonZeroU32;

const ALGORITHM: pbkdf2::Algorithm = pbkdf2::PBKDF2_HMAC_SHA256;
const PREFIX: &str = "pbkdf2_sha256";
const ITERATIONS: u32 = 600_000;
const MAX_ITERATIONS: u32 = 10_000_000;
const SALT_LEN: usize = 16;
const HASH_LEN: usize = 32;

pub(super) fn hash(password: &str) -> String {
    let mut salt = [0u8; SALT_LEN];
    ring::rand::SystemRandom::new()
        .fill(&mut salt)
        .expect("system random source is available");
    let mut out = [0u8; HASH_LEN];
    pbkdf2::derive(
        ALGORITHM,
        NonZeroU32::new(ITERATIONS).expect("iterations are non-zero"),
        &salt,
        password.as_bytes(),
        &mut out,
    );
    format!(
        "{PREFIX}${ITERATIONS}${}${}",
        STANDARD.encode(salt),
        STANDARD.encode(out)
    )
}

/// An empty or malformed stored value never verifies.
pub(super) fn verify(stored: &str, password: &str) -> bool {
    let parts: Vec<&str> = stored.split('$').collect();
    let [prefix, iterations, salt, expected] = parts.as_slice() else {
        return false;
    };
    if *prefix != PREFIX {
        return false;
    }
    let Some(iterations) = iterations
        .parse::<u32>()
        .ok()
        .and_then(NonZeroU32::new)
        .filter(|n| n.get() <= MAX_ITERATIONS)
    else {
        return false;
    };
    let (Ok(salt), Ok(expected)) = (STANDARD.decode(salt), STANDARD.decode(expected)) else {
        return false;
    };
    pbkdf2::verify(ALGORITHM, iterations, &salt, password.as_bytes(), &expected).is_ok()
}
