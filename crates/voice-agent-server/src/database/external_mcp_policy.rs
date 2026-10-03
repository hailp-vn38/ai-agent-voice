//! External MCP destination policy shared by desired-configuration validation and runtime dial.
//!
//! Two rules bind every destination, and both are the operator's to set:
//!
//! - The scheme.  HTTPS always; `http` only when `allow_http_lan` is explicitly enabled.
//! - The address.  Every destination must be named — an IP literal in `allowed_cidrs`, a hostname
//!   in `allowed_hosts` — and it is re-checked after DNS resolution, so a name that resolves
//!   somewhere else is refused just before the connection rather than trusted.
//!
//! Loopback is inside the LAN scope of the HTTP exception, because a homelab MCP server usually
//! runs on the same host as the agent.  That grants nothing on its own: an IP-literal loopback URL
//! still has to match `allowed_cidrs`, a `localhost` name still has to match `allowed_hosts`, and
//! the resolved address is validated like any other.
use crate::config::ExternalMcpNetworkConfig;
use std::net::IpAddr;
use url::{Host, Url};

/// Validates syntax only. DNS and the complete destination set are checked at dial time.
pub fn valid_desired_url(value: &str, network: &ExternalMcpNetworkConfig) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.scheme(), "https" | "http")
        || (url.scheme() == "http" && !network.allow_http_lan)
    {
        return false;
    }
    // The typed host is the whole point: `Url::host_str` renders an IPv6 literal bracketed, which
    // would make `[::1]` miss every CIDR an operator wrote for it.
    match url.host() {
        Some(Host::Ipv4(ip)) => {
            cidr_allowed(IpAddr::V4(ip), &network.allowed_cidrs)
                && (url.scheme() != "http" || is_lan_ip(IpAddr::V4(ip)))
        }
        Some(Host::Ipv6(ip)) => {
            cidr_allowed(IpAddr::V6(ip), &network.allowed_cidrs)
                && (url.scheme() != "http" || is_lan_ip(IpAddr::V6(ip)))
        }
        Some(Host::Domain(host)) => {
            host_allowed(host, &network.allowed_hosts) || !network.allowed_cidrs.is_empty()
        }
        None => false,
    }
}

/// Validates every address returned by DNS immediately before an outbound connection.
/// A hostname allowlist authorizes the TLS name; CIDR rules constrain numeric destinations.
pub fn valid_resolved_destination(
    url: &Url,
    addresses: &[IpAddr],
    network: &ExternalMcpNetworkConfig,
) -> bool {
    if addresses.is_empty() {
        return false;
    }
    // A destination is admitted when the operator named the host, or named the range every address
    // it resolved to falls in.  The typed host matters because a bracketed `[::1]` matches neither
    // spelling, and an IP literal can only have been admitted by a CIDR in the first place.
    let admitted = match url.host() {
        Some(Host::Domain(host)) => host_allowed(host, &network.allowed_hosts),
        Some(Host::Ipv4(ip)) => cidr_allowed(IpAddr::V4(ip), &network.allowed_cidrs),
        Some(Host::Ipv6(ip)) => cidr_allowed(IpAddr::V6(ip), &network.allowed_cidrs),
        None => return false,
    };
    (admitted
        || addresses
            .iter()
            .all(|ip| cidr_allowed(*ip, &network.allowed_cidrs)))
        && (url.scheme() != "http" || addresses.iter().all(|ip| is_lan_ip(*ip)))
}

/// Runtime seam: resolves once per outbound attempt and rejects mixed DNS answers.
#[allow(clippy::result_unit_err)] // Callers deliberately expose no network-policy detail.
pub async fn resolve_and_validate(
    url: &Url,
    network: &ExternalMcpNetworkConfig,
) -> Result<Vec<IpAddr>, ()> {
    let host = url.host_str().ok_or(())?;
    let port = url.port_or_known_default().ok_or(())?;
    let addresses: Vec<IpAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| ())?
        .map(|address| address.ip())
        .collect();
    valid_resolved_destination(url, &addresses, network)
        .then_some(addresses)
        .ok_or(())
}

fn host_allowed(host: &str, allowed: &[String]) -> bool {
    allowed.iter().any(|rule| {
        rule == host
            || rule
                .strip_prefix("*.")
                .is_some_and(|suffix| host.ends_with(suffix) && host.len() > suffix.len())
    })
}
fn cidr_allowed(ip: IpAddr, allowed: &[String]) -> bool {
    allowed.iter().any(|cidr| cidr_contains(cidr, ip))
}

/// Whether a destination sits inside the private scope the explicit `allow_http_lan` exception
/// covers.  Loopback counts: a homelab MCP server usually runs on the same host as the agent, and
/// reaching it still requires the operator to name `127.0.0.0/8` or the host explicitly in the
/// allowlist, so this widens where an operator may point HTTP without widening who may.
fn is_lan_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private() || ip.is_link_local() || ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_unique_local() || ip.is_unicast_link_local() || ip.is_loopback(),
    }
}
fn cidr_contains(cidr: &str, ip: IpAddr) -> bool {
    let Some((base, bits)) = cidr.split_once('/') else {
        return false;
    };
    let Ok(base) = base.parse::<IpAddr>() else {
        return false;
    };
    let Ok(bits) = bits.parse::<u8>() else {
        return false;
    };
    match (base, ip) {
        (IpAddr::V4(base), IpAddr::V4(ip)) if bits <= 32 => {
            u32::from(base) >> (32 - bits) == u32::from(ip) >> (32 - bits)
        }
        (IpAddr::V6(base), IpAddr::V6(ip)) if bits <= 128 => {
            u128::from(base) >> (128 - bits) == u128::from(ip) >> (128 - bits)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hostname_can_be_admitted_by_its_resolved_cidr_but_mixed_answers_fail() {
        let network = ExternalMcpNetworkConfig {
            allow_http_lan: false,
            allowed_hosts: vec![],
            allowed_cidrs: vec!["10.0.0.0/8".into()],
        };
        assert!(valid_desired_url("https://mcp.internal.test/rpc", &network));
        let https = Url::parse("https://mcp.internal.test/rpc").unwrap();
        assert!(valid_resolved_destination(
            &https,
            &["10.12.0.5".parse().unwrap()],
            &network
        ));
        assert!(!valid_resolved_destination(
            &https,
            &[
                "10.12.0.5".parse().unwrap(),
                "198.51.100.1".parse().unwrap()
            ],
            &network
        ));
    }

    #[test]
    fn http_lan_exception_rejects_a_public_dns_answer() {
        let network = ExternalMcpNetworkConfig {
            allow_http_lan: true,
            allowed_hosts: vec!["mcp.internal.test".into()],
            allowed_cidrs: vec![],
        };
        let http = Url::parse("http://mcp.internal.test/rpc").unwrap();
        assert!(!valid_resolved_destination(
            &http,
            &["198.51.100.1".parse().unwrap()],
            &network
        ));
        assert!(valid_resolved_destination(
            &http,
            &["10.12.0.5".parse().unwrap()],
            &network
        ));
    }

    /// A same-host MCP server is the common homelab case, so loopback is inside the HTTP exception's
    /// LAN scope.  Both halves of the rule are proven here: the exception must be on, and the
    /// operator must have named the destination themselves.  Nothing about loopback is a shortcut
    /// past the allowlist.
    #[test]
    fn http_loopback_needs_both_the_exception_and_an_explicit_allowlist_entry() {
        let named_v4 = ExternalMcpNetworkConfig {
            allow_http_lan: true,
            allowed_hosts: vec![],
            allowed_cidrs: vec!["127.0.0.0/8".into()],
        };
        let named_v6 = ExternalMcpNetworkConfig {
            allow_http_lan: true,
            allowed_hosts: vec![],
            allowed_cidrs: vec!["::1/128".into()],
        };
        let by_name = ExternalMcpNetworkConfig {
            allow_http_lan: true,
            allowed_hosts: vec!["mcp.localhost".into()],
            allowed_cidrs: vec![],
        };

        // Both address families, at both the desired-configuration check and the dial-time check.
        for (url, network, address) in [
            (
                "http://127.0.0.1:8931/rpc",
                &named_v4,
                "127.0.0.1".parse().unwrap(),
            ),
            ("http://[::1]:8931/rpc", &named_v6, "::1".parse().unwrap()),
        ] {
            assert!(valid_desired_url(url, network), "{url} is explicitly named");
            let parsed = Url::parse(url).unwrap();
            assert!(
                valid_resolved_destination(&parsed, &[address], network),
                "{url} still passes after DNS resolution"
            );
        }
        assert!(valid_resolved_destination(
            &Url::parse("http://mcp.localhost:8931/rpc").unwrap(),
            &["127.0.0.1".parse().unwrap()],
            &by_name
        ));

        // The exception off: loopback is refused like any other HTTP destination.
        let https_only = ExternalMcpNetworkConfig {
            allow_http_lan: false,
            allowed_cidrs: vec!["127.0.0.0/8".into(), "::1/128".into()],
            ..Default::default()
        };
        assert!(!valid_desired_url("http://127.0.0.1:8931/rpc", &https_only));
        assert!(!valid_desired_url("http://[::1]:8931/rpc", &https_only));

        // The exception on but nothing named: refused, because an unlisted loopback is unlisted.
        let unnamed = ExternalMcpNetworkConfig {
            allow_http_lan: true,
            allowed_hosts: vec![],
            allowed_cidrs: vec![],
        };
        assert!(!valid_desired_url("http://127.0.0.1:8931/rpc", &unnamed));
        assert!(!valid_desired_url("http://[::1]:8931/rpc", &unnamed));
        // Naming only the other family does not admit this one.
        assert!(!valid_desired_url("http://[::1]:8931/rpc", &named_v4));
        assert!(!valid_desired_url("http://127.0.0.1:8931/rpc", &named_v6));

        // A name that resolves off the allowlisted range is refused before the connection.
        assert!(!valid_resolved_destination(
            &Url::parse("http://mcp.localhost:8931/rpc").unwrap(),
            &["203.0.113.9".parse().unwrap()],
            &by_name
        ));
    }

    /// The IPv6 literal is matched as an address, not as the bracketed text a URL renders it as.
    #[test]
    fn an_ipv6_literal_is_validated_as_an_address() {
        let network = ExternalMcpNetworkConfig {
            allow_http_lan: false,
            allowed_hosts: vec![],
            allowed_cidrs: vec!["fd00::/8".into()],
        };
        assert!(valid_desired_url("https://[fd00::5]/rpc", &network));
        assert!(!valid_desired_url("https://[2001:db8::5]/rpc", &network));
    }
}
