use std::sync::Arc;

use super::super::cdp::extract_wss_via_chromium;
use super::super::client::{MuseVmClient, MuseVmTransport, VmError};
use super::super::config::{SITE_URL, format_cookie_header, parse_cookie_header};
use super::super::credentials::{
    WSS_EXPIRY_BUFFER_SECS, fetch_wss_from_cookie_http, get_wss_expiry_unix_secs, is_wss_valid,
};
use super::{SharedMusePool, UpstreamFailureKind};

impl SharedMusePool {
    pub(super) async fn ensure_account_client(
        &self,
        account_index: usize,
    ) -> Result<Arc<MuseVmClient>, (UpstreamFailureKind, String)> {
        let cred = {
            let state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
            let acc = state.accounts.get(account_index).ok_or_else(|| {
                (
                    UpstreamFailureKind::Transient,
                    "invalid Muse account index".to_string(),
                )
            })?;
            if let Some(existing) = &acc.vm_client
                && existing.is_connected()
                && acc
                    .credential
                    .wss_url
                    .as_deref()
                    .is_none_or(|u| is_wss_valid(u, WSS_EXPIRY_BUFFER_SECS))
            {
                return Ok(Arc::clone(existing));
            }
            acc.credential.clone()
        };

        if let Some(existing_wss) = cred
            .wss_url
            .as_deref()
            .filter(|u| is_wss_valid(u, WSS_EXPIRY_BUFFER_SECS))
        {
            match connect_and_init_client(&*self.inner.transport, existing_wss).await {
                Ok(client) => {
                    let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(acc) = state.accounts.get_mut(account_index) {
                        acc.vm_client = Some(Arc::clone(&client));
                    }
                    return Ok(client);
                }
                Err(VmError::AuthInvalid(msg)) if cred.cookies.is_empty() => {
                    return Err((UpstreamFailureKind::AuthInvalid, msg));
                }
                Err(_) if !cred.cookies.is_empty() => {
                    // Fall through to bootstrap a fresh WSS URL from cookies
                }
                Err(err) => {
                    return Err(vm_error_to_pool_failure(err));
                }
            }
        }

        if cred.cookies.is_empty() {
            return Err((
                UpstreamFailureKind::AuthInvalid,
                "Muse WSS URL expired and no session cookie is available for renewal".to_string(),
            ));
        }

        let wss_url = if self.inner.has_custom_transport {
            let vm_name = cred.vm_id.as_deref().unwrap_or(&cred.label);
            format!("wss://hatch.metaaivm.com/v1/noise?vm_id={vm_name}&app_id=hatch-web")
        } else {
            let cookie_header = format_cookie_header(&cred.cookies);
            let bootstrapped =
                match fetch_wss_from_cookie_http(&self.inner.http, &cookie_header, SITE_URL).await {
                    Ok(session) => session,
                    Err((UpstreamFailureKind::Transient, _))
                        if self.inner.fallback_browser.is_some() =>
                    {
                        let engine = self
                            .inner
                            .fallback_browser
                            .as_ref()
                            .expect("fallback_browser checked");
                        let ctx_key = format!("muse-bootstrap-{account_index}");
                        extract_wss_via_chromium(&**engine, &ctx_key, &cred.cookies).await?
                    }
                    Err(err) => return Err(err),
                };

            let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(acc) = state.accounts.get_mut(account_index) {
                acc.credential.wss_url = Some(bootstrapped.wss_url.clone());
                acc.credential.wss_expires = get_wss_expiry_unix_secs(&bootstrapped.wss_url);
                acc.credential.cookies = parse_cookie_header(&bootstrapped.cookie_header);
                if bootstrapped.vm_id.is_some() {
                    acc.credential.vm_id = bootstrapped.vm_id;
                }
            }
            bootstrapped.wss_url
        };

        let client = connect_and_init_client(&*self.inner.transport, &wss_url)
            .await
            .map_err(vm_error_to_pool_failure)?;

        {
            let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(acc) = state.accounts.get_mut(account_index) {
                if acc.credential.wss_url.is_none() {
                    acc.credential.wss_url = Some(wss_url);
                }
                acc.vm_client = Some(Arc::clone(&client));
            }
        }
        Ok(client)
    }
}

async fn connect_and_init_client(
    transport: &dyn MuseVmTransport,
    wss_url: &str,
) -> Result<Arc<MuseVmClient>, VmError> {
    let client = MuseVmClient::connect(transport, wss_url).await?;
    if let Err(err) = client.subscribe().await {
        client.close().await;
        return Err(err);
    }
    client.ensure_isolation_policy().await;
    Ok(client)
}

fn vm_error_to_pool_failure(err: VmError) -> (UpstreamFailureKind, String) {
    match err {
        VmError::AuthInvalid(msg) => (UpstreamFailureKind::AuthInvalid, msg),
        VmError::QuotaExceeded(msg) => (UpstreamFailureKind::QuotaExceeded, msg),
        VmError::Timeout(msg) | VmError::Transport(msg) => (UpstreamFailureKind::Transient, msg),
    }
}
