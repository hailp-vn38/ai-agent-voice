use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use uuid::Uuid;

use super::super::config::{NEW_THREAD_URL, SESSION_API_URL, SITE_URL, VM_WAKE_API_URL};
use super::types::{AccountHealth, UpstreamFailureKind};
use super::{SharedMusePool, spawn_page_close};

impl SharedMusePool {
    pub(super) fn apply_account_failure(&self, account_idx: usize, kind: UpstreamFailureKind) {
        let mut stale_pages = Vec::new();
        {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(acc) = guard.accounts.get_mut(account_idx) {
                acc.health = match kind {
                    UpstreamFailureKind::AuthInvalid => AccountHealth::Invalid,
                    UpstreamFailureKind::QuotaExceeded => {
                        AccountHealth::Cooldown(Instant::now() + Duration::from_secs(3600))
                    }
                    UpstreamFailureKind::Transient => {
                        AccountHealth::Cooldown(Instant::now() + Duration::from_secs(30))
                    }
                };
                if matches!(kind, UpstreamFailureKind::AuthInvalid) {
                    for idle in acc.idle_tabs.drain(..) {
                        acc.active_tabs = acc.active_tabs.saturating_sub(1);
                        stale_pages.push(idle.page);
                    }
                }
            }
        }
        for page in stale_pages {
            spawn_page_close(page);
        }
        self.notify.notify_waiters();
    }

    pub(super) async fn ensure_account_warm(
        &self,
        account_idx: usize,
    ) -> Result<(), UpstreamFailureKind> {
        if !self.http_session_warmup {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(acc) = guard.accounts.get_mut(account_idx) {
                acc.warmed = true;
            }
            return Ok(());
        }

        let (already_warmed, cookies, vm_id) = {
            let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let Some(acc) = guard.accounts.get(account_idx) else {
                return Err(UpstreamFailureKind::AuthInvalid);
            };
            (acc.warmed, acc.in_memory_cookies.clone(), acc.vm_id.clone())
        };
        if already_warmed {
            return Ok(());
        }

        let cookie_header = cookies
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ");

        let resp = self
            .http_client
            .get(SESSION_API_URL)
            .header("Origin", SITE_URL)
            .header("Referer", NEW_THREAD_URL)
            .header("Accept", "application/json")
            .header("Cookie", &cookie_header)
            .send()
            .await;

        let mut rotated_cookies = cookies;
        let mut discovered_vm_id = vm_id;

        if let Ok(resp) = resp {
            let status = resp.status().as_u16();
            if status == 401 || status == 403 {
                self.apply_account_failure(account_idx, UpstreamFailureKind::AuthInvalid);
                return Err(UpstreamFailureKind::AuthInvalid);
            }
            for header_val in resp.headers().get_all(reqwest::header::SET_COOKIE) {
                if let Ok(raw) = header_val.to_str()
                    && let Some(first_part) = raw.split(';').next()
                    && let Some((k, v)) = first_part.split_once('=')
                {
                    let k = k.trim();
                    let v = v.trim();
                    if !k.is_empty() && !v.is_empty() {
                        rotated_cookies.insert(k.to_string(), v.to_string());
                    }
                }
            }
            if let Ok(body) = resp.json::<serde_json::Value>().await
                && discovered_vm_id.is_none()
            {
                discovered_vm_id = body
                    .get("vm_id")
                    .and_then(serde_json::Value::as_str)
                    .map(ToString::to_string);
            }
        }

        if let Some(vm) = &discovered_vm_id {
            let updated_cookie_header = rotated_cookies
                .iter()
                .filter(|(_, v)| !v.is_empty())
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            let _ = self
                .http_client
                .post(VM_WAKE_API_URL)
                .header("Origin", SITE_URL)
                .header("Referer", NEW_THREAD_URL)
                .header("Cookie", updated_cookie_header)
                .json(&serde_json::json!({
                    "vm_id": vm,
                    "retry_count": 0,
                    "connect_attempt_id": Uuid::new_v4().to_string(),
                }))
                .send()
                .await;
        }

        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(acc) = guard.accounts.get_mut(account_idx) {
            if acc.in_memory_cookies != rotated_cookies {
                acc.in_memory_cookies = rotated_cookies;
                acc.cookie_expires = Some(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                        + 7 * 86400,
                );
                for tab in &mut acc.idle_tabs {
                    tab.stale = true;
                }
            }
            acc.vm_id = discovered_vm_id;
            acc.warmed = true;
        }
        Ok(())
    }
}
