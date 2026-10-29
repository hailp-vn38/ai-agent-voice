use std::collections::BTreeMap;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::super::cdp::CookieParam;
use super::super::config::{COOKIE_DOMAIN, MuseAccountCredential};
use super::types::{AccountHealth, PooledTab};

pub(super) struct MuseAccountState {
    pub(super) credential: MuseAccountCredential,
    pub(super) in_memory_cookies: BTreeMap<String, String>,
    pub(super) cookie_expires: Option<u64>,
    pub(super) vm_id: Option<String>,
    pub(super) warmed: bool,
    pub(super) generation: u64,
    pub(super) health: AccountHealth,
    pub(super) active_tabs: usize,
    pub(super) idle_tabs: Vec<PooledTab>,
}

impl MuseAccountState {
    pub(super) fn new(credential: MuseAccountCredential) -> Self {
        let in_memory_cookies = credential.cookies.clone();
        let cookie_expires = credential.cookie_expires;
        let vm_id = credential.vm_id.clone();
        Self {
            credential,
            in_memory_cookies,
            cookie_expires,
            vm_id,
            warmed: false,
            generation: 0,
            health: AccountHealth::Healthy,
            active_tabs: 0,
            idle_tabs: Vec::new(),
        }
    }

    pub(super) fn is_available(&self, now: Instant) -> bool {
        match self.health {
            AccountHealth::Healthy => true,
            AccountHealth::Cooldown(until) => now >= until,
            AccountHealth::Invalid => false,
        }
    }

    pub(super) fn refresh_health(&mut self, now: Instant) {
        if let AccountHealth::Cooldown(until) = self.health {
            if now >= until {
                self.health = AccountHealth::Healthy;
            }
        }
    }

    pub(super) fn context_key(&self, fingerprint: &[u8; 32]) -> String {
        let prefix = hex_prefix(&fingerprint[..8]);
        format!("{prefix}:{}:{}", self.credential.label, self.generation)
    }

    pub(super) fn cookie_params(&self) -> Vec<CookieParam> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        let floor = now + 3600.0;
        let expires = match self.cookie_expires.map(|e| e as f64) {
            Some(exp) if exp > floor => exp,
            _ => now + 7.0 * 86400.0,
        };
        self.in_memory_cookies
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| CookieParam {
                name: k.clone(),
                value: v.clone(),
                domain: COOKIE_DOMAIN.to_string(),
                path: "/".to_string(),
                secure: true,
                expires,
            })
            .collect()
    }
}

fn hex_prefix(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}
