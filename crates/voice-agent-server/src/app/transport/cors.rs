//! Shared browser CORS policy for HTTP transports.
use axum::http::{HeaderMap, HeaderValue, Method, header};
use tower_http::cors::{AllowHeaders, AllowOrigin, CorsLayer};
use url::{Host, Url};

/// Browser clients in the supported trusted-LAN deployment use bearer headers, not cookies.
pub(crate) fn layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _| is_local_origin(origin)))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers(AllowHeaders::mirror_request())
        .expose_headers([
            header::ETAG,
            header::HeaderName::from_static("x-request-id"),
            header::HeaderName::from_static("x-provider-test-elapsed-ms"),
            header::HeaderName::from_static("x-provider-test-source"),
        ])
}

/// Accepts a browser origin that is local, configured, or the same origin the request was sent to.
///
/// Same-origin is decided from `Host` and `X-Forwarded-Proto`, so Studio works behind any
/// reverse proxy (LAN, Tailscale, TLS terminators) without listing each address.
pub(crate) fn is_trusted_origin(headers: &HeaderMap, trusted: &[Url]) -> bool {
    if headers.get_all(header::ORIGIN).iter().count() != 1 {
        return false;
    }
    let Some(origin) = headers.get(header::ORIGIN) else {
        return false;
    };
    if is_local_origin(origin) {
        return true;
    }
    let Some(text) = canonical_origin(origin) else {
        return false;
    };
    trusted.iter().any(|url| url.origin().ascii_serialization() == text)
        || request_origin(headers).is_some_and(|own| own == text)
}

fn canonical_origin(origin: &HeaderValue) -> Option<&str> {
    let text = origin.to_str().ok()?;
    let url = Url::parse(text).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.origin().ascii_serialization() == text)
        .then_some(text)
}

fn request_origin(headers: &HeaderMap) -> Option<String> {
    if headers.get_all(header::HOST).iter().count() != 1 {
        return None;
    }
    let host = headers.get(header::HOST)?.to_str().ok()?;
    let scheme = match headers.get("x-forwarded-proto") {
        Some(value) => value.to_str().ok()?.split(',').next()?.trim().to_ascii_lowercase(),
        None => "http".to_owned(),
    };
    if !matches!(scheme.as_str(), "http" | "https") || host.contains(['/', '@', '?', '#']) {
        return None;
    }
    Some(Url::parse(&format!("{scheme}://{host}")).ok()?.origin().ascii_serialization())
}

pub(crate) fn is_local_origin(origin: &HeaderValue) -> bool {
    let Ok(text) = origin.to_str() else {
        return false;
    };
    let Ok(url) = Url::parse(text) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") || url.origin().ascii_serialization() != text {
        return false;
    }
    match url.host() {
        Some(Host::Domain(host)) => host == "localhost",
        Some(Host::Ipv4(address)) => {
            address.is_private() || address.is_loopback() || address.is_link_local()
        }
        Some(Host::Ipv6(address)) => {
            address.is_loopback()
                || address.is_unique_local()
                || address.is_unicast_link_local()
                || address.to_ipv4_mapped().is_some_and(|address| {
                    address.is_private() || address.is_loopback() || address.is_link_local()
                })
        }
        None => false,
    }
}

#[cfg(test)]
#[path = "cors_tests.rs"]
mod tests;
