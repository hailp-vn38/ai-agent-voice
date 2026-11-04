use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::Value;

use super::super::config::{COOKIE_DOMAIN, NEW_THREAD_URL, SITE_URL, format_cookie_header};
use super::super::credentials::{BootstrappedMuseSession, build_wss_url};
use super::super::pool::UpstreamFailureKind;
use super::{BrowserEngine, CookieParam};

const EXTRACT_GATEWAY_INIT_JS: &str = r#"(async () => {
    const g = window.__HATCH_GATEWAY_INIT__;
    if (g && (g.authToken || g.token) && (g.vmName || g.vmId || g.vm_id)) {
        return {
            vmId: g.vmName || g.vmId || g.vm_id,
            authToken: g.authToken || g.token,
            notaryToken: g.noiseNotaryToken || g.notaryToken || g.notary_token || ""
        };
    }
    const sResp = await fetch("/api/session?vm_state=live", { credentials: "include" });
    if (sResp.status === 401) {
        return { unauthorized: true, status: sResp.status };
    }
    if (!sResp.ok) {
        return { error: "session_http_" + sResp.status };
    }
    const s = await sResp.json();
    const firstVm = Array.isArray(s.vms) && s.vms.length > 0 ? s.vms[0] : null;
    const vmId = s.vmId || s.vm_id || s.vm_name || s.vmName || (firstVm && (firstVm.vm_id || firstVm.vmId || firstVm.id)) || "";
    const endpointUrl = s.endpointUrl || s.endpoint_url || (firstVm && (firstVm.endpointUrl || firstVm.endpoint_url)) || (vmId ? ("wss://" + vmId + ".metaaivm.com/") : "");
    const email = s.email || (s.user && s.user.email) || "";
    if (!vmId || !endpointUrl) {
        return { error: "missing_vm_or_endpoint" };
    }
    const tResp = await fetch("/api/hatch/token", {
        method: "POST",
        credentials: "include",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ vmAddress: endpointUrl, vmName: vmId })
    });
    if (!tResp.ok) {
        return { error: "token_http_" + tResp.status };
    }
    const t = await tResp.json();
    return {
        vmId,
        authToken: t.token || t.authToken || "",
        notaryToken: t.notary_token || t.noiseNotaryToken || t.notaryToken || "",
        email
    };
})()"#;

/// Tier-5 fallback: uses a temporary Chromium page via `BrowserEngine` to bypass Cloudflare
/// and extract the Hatch VM WebSocket token (`window.__HATCH_GATEWAY_INIT__` or `/api/hatch/token`),
/// then closes the page and browser context immediately.
pub(crate) async fn extract_wss_via_chromium(
    engine: &dyn BrowserEngine,
    context_key: &str,
    cookies: &BTreeMap<String, String>,
) -> Result<BootstrappedMuseSession, (UpstreamFailureKind, String)> {
    let cookie_params: Vec<CookieParam> = cookies
        .iter()
        .map(|(k, v)| CookieParam {
            name: k.clone(),
            value: v.clone(),
            domain: COOKIE_DOMAIN.to_string(),
            path: "/".to_string(),
            secure: true,
            expires: 0.0,
        })
        .collect();

    let page = engine
        .open_page(context_key, &cookie_params)
        .await
        .map_err(|e| {
            (
                UpstreamFailureKind::Transient,
                format!("cdp open_page: {e}"),
            )
        })?;

    let result = async {
        page.navigate(NEW_THREAD_URL)
            .await
            .map_err(|e| (UpstreamFailureKind::Transient, format!("cdp navigate: {e}")))?;
        let val = page
            .evaluate_json(EXTRACT_GATEWAY_INIT_JS, true, Duration::from_secs(20))
            .await
            .map_err(|e| (UpstreamFailureKind::Transient, format!("cdp evaluate: {e}")))?;
        if val
            .get("unauthorized")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Err((
                UpstreamFailureKind::AuthInvalid,
                "Muse session unauthorized inside Chromium fallback".to_string(),
            ));
        }
        let vm_id = val
            .get("vmId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                (
                    UpstreamFailureKind::Transient,
                    "Chromium fallback missing vmId".to_string(),
                )
            })?
            .to_string();
        let auth_token = val
            .get("authToken")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                (
                    UpstreamFailureKind::Transient,
                    "Chromium fallback missing authToken".to_string(),
                )
            })?;
        let notary_token = val.get("notaryToken").and_then(Value::as_str).unwrap_or("");
        let discovered_email = val
            .get("email")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned);
        let refreshed_cookies = page
            .get_cookies(&[SITE_URL, NEW_THREAD_URL])
            .await
            .unwrap_or_else(|_| cookies.clone());
        let merged_cookies = if refreshed_cookies.contains_key("hatch_sess") {
            refreshed_cookies
        } else {
            cookies.clone()
        };
        Ok(BootstrappedMuseSession {
            wss_url: build_wss_url(&vm_id, auth_token, notary_token),
            cookie_header: format_cookie_header(&merged_cookies),
            vm_id: Some(vm_id),
            discovered_email,
        })
    }
    .await;

    page.close().await;
    engine.dispose_context(context_key).await;
    result
}
