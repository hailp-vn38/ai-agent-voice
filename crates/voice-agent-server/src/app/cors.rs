use axum::http::{HeaderValue, Method, header};
use tower_http::cors::{AllowHeaders, AllowOrigin, CorsLayer};
use url::{Host, Url};

/// Browser clients in the supported trusted-LAN deployment use bearer headers, not cookies.
pub(super) fn layer() -> CorsLayer {
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

pub(super) fn is_local_origin(origin: &HeaderValue) -> bool {
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
