//! Forget Tombstones: keyed fingerprints only, so what was forgotten cannot be read from the trace.

use super::{fact_file::canonical_text, normalize::normalize, types::format_time, vault};
use chrono::{DateTime, Utc};
use ring::hmac;
use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Tombstone {
    fp: String,
    at: String,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Tombstones(Vec<Tombstone>);

impl Tombstones {
    /// An unreadable file is an error, never "no tombstones": that would bring forgotten facts back.
    pub(super) fn read(path: &Path) -> io::Result<Self> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Self)
                .map_err(|_| io::ErrorKind::InvalidData.into()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    /// Lifting a tombstone comes before the commit point of the change that asked for it.
    pub(super) fn write(&self, path: &Path) -> io::Result<()> {
        vault::write_atomic(path, &self.to_bytes()?)
    }

    /// Adding a tombstone is the commit point of `Forget`.
    pub(super) fn commit(&self, path: &Path) -> io::Result<vault::Committed> {
        vault::commit_atomic(path, &self.to_bytes()?)
    }

    fn to_bytes(&self) -> io::Result<Vec<u8>> {
        serde_json::to_vec(&self.0).map_err(|_| io::ErrorKind::InvalidData.into())
    }

    pub(super) fn contains(&self, fingerprint: &str) -> bool {
        self.0.iter().any(|entry| entry.fp == fingerprint)
    }

    pub(super) fn add(&mut self, fingerprint: String, at: &DateTime<Utc>) {
        if !self.contains(&fingerprint) {
            self.0.push(Tombstone {
                fp: fingerprint,
                at: format_time(at),
            });
        }
    }

    pub(super) fn remove(&mut self, fingerprint: &str) {
        self.0.retain(|entry| entry.fp != fingerprint);
    }
}

/// `HMAC-SHA256(key, device_dir || 0x00 || normalize(body))`, hex encoded.
pub(super) fn fingerprint(key: &[u8; 32], device_dir: &str, body: &str) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, key);
    let mut context = hmac::Context::with_key(&key);
    context.update(device_dir.as_bytes());
    context.update(&[0]);
    context.update(normalize(&canonical_text(body)).as_bytes());
    context
        .sign()
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
