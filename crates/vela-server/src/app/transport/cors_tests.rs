use axum::http::{HeaderMap, HeaderValue};
use url::Url;

use super::is_trusted_origin;

fn headers(origin: &str, host: Option<&str>, proto: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("origin", HeaderValue::from_str(origin).unwrap());
    if let Some(host) = host {
        headers.insert("host", HeaderValue::from_str(host).unwrap());
    }
    if let Some(proto) = proto {
        headers.insert("x-forwarded-proto", HeaderValue::from_str(proto).unwrap());
    }
    headers
}

#[test]
fn same_origin_behind_tls_proxy_is_trusted() {
    let headers = headers(
        "https://homelab.tail5c78c9.ts.net:10443",
        Some("homelab.tail5c78c9.ts.net:10443"),
        Some("https"),
    );
    assert!(is_trusted_origin(&headers, &[]));
}

#[test]
fn same_origin_on_tailscale_ip_is_trusted() {
    let headers = headers("http://100.95.32.71:8080", Some("100.95.32.71:8080"), None);
    assert!(is_trusted_origin(&headers, &[]));
}

#[test]
fn same_origin_with_default_port_is_trusted() {
    let headers = headers(
        "https://studio.example",
        Some("studio.example:443"),
        Some("https"),
    );
    assert!(is_trusted_origin(&headers, &[]));
}

#[test]
fn host_with_different_port_is_not_same_origin() {
    let headers = headers("http://100.95.32.71:9000", Some("100.95.32.71:8080"), None);
    assert!(!is_trusted_origin(&headers, &[]));
}

#[test]
fn scheme_mismatch_is_not_same_origin() {
    let headers = headers(
        "http://homelab.tail5c78c9.ts.net:10443",
        Some("homelab.tail5c78c9.ts.net:10443"),
        Some("https"),
    );
    assert!(!is_trusted_origin(&headers, &[]));
}

#[test]
fn cross_origin_is_rejected() {
    let headers = headers("https://evil.example", Some("100.95.32.71:8080"), None);
    assert!(!is_trusted_origin(&headers, &[]));
}

#[test]
fn configured_origin_is_trusted() {
    let headers = headers("https://studio.example", Some("api.example"), Some("https"));
    let trusted = [Url::parse("https://studio.example").unwrap()];
    assert!(is_trusted_origin(&headers, &trusted));
}

#[test]
fn local_origin_is_still_trusted() {
    let headers = headers("http://localhost:5173", Some("127.0.0.1:8000"), None);
    assert!(is_trusted_origin(&headers, &[]));
}

#[test]
fn duplicate_origin_headers_are_rejected() {
    let mut headers = headers("http://100.95.32.71:8080", Some("100.95.32.71:8080"), None);
    headers.append(
        "origin",
        HeaderValue::from_static("http://100.95.32.71:8080"),
    );
    assert!(!is_trusted_origin(&headers, &[]));
}

#[test]
fn non_canonical_origin_is_rejected() {
    let headers = headers("http://100.95.32.71:8080/", Some("100.95.32.71:8080"), None);
    assert!(!is_trusted_origin(&headers, &[]));
}
