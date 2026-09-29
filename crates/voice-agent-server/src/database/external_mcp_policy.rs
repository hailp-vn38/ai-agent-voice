//! External MCP destination policy shared by desired-configuration validation and runtime dial.
use crate::config::ExternalMcpNetworkConfig;
use std::net::IpAddr;
use url::Url;

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
    let Some(host) = url.host_str() else {
        return false;
    };
    if let Ok(ip) = host.parse::<IpAddr>() {
        return cidr_allowed(ip, &network.allowed_cidrs)
            && (url.scheme() != "http" || is_lan_ip(ip));
    }
    host_allowed(host, &network.allowed_hosts) || !network.allowed_cidrs.is_empty()
}

/// Validates every address returned by DNS immediately before an outbound connection.
/// A hostname allowlist authorizes the TLS name; CIDR rules constrain numeric destinations.
pub fn valid_resolved_destination(
    url: &Url,
    addresses: &[IpAddr],
    network: &ExternalMcpNetworkConfig,
) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host_allowed = host_allowed(host, &network.allowed_hosts);
    !addresses.is_empty()
        && (host_allowed
            || addresses
                .iter()
                .all(|ip| cidr_allowed(*ip, &network.allowed_cidrs)))
        && (url.scheme() != "http" || addresses.iter().all(|ip| is_lan_ip(*ip)))
}

/// Runtime seam: resolves once per outbound attempt and rejects mixed DNS answers.
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

    /// A same-host MCP server is the common homelab case.  It is reachable over the documented LAN
    /// exception, and only because the operator named the loopback range themselves.
    #[test]
    fn http_lan_exception_reaches_loopback_only_when_it_is_allowlisted() {
        let named = ExternalMcpNetworkConfig {
            allow_http_lan: true,
            allowed_hosts: vec![],
            allowed_cidrs: vec!["127.0.0.0/8".into()],
        };
        assert!(valid_desired_url("http://127.0.0.1:8931/rpc", &named));
        let loopback = Url::parse("http://127.0.0.1:8931/rpc").unwrap();
        assert!(valid_resolved_destination(
            &loopback,
            &["127.0.0.1".parse().unwrap()],
            &named
        ));

        // The same destination without an operator-named range, and the same shape with the
        // exception switched off, both stay refused.
        let unnamed = ExternalMcpNetworkConfig {
            allow_http_lan: true,
            allowed_hosts: vec![],
            allowed_cidrs: vec![],
        };
        assert!(!valid_desired_url("http://127.0.0.1:8931/rpc", &unnamed));
        let https_only = ExternalMcpNetworkConfig {
            allow_http_lan: false,
            allowed_cidrs: vec!["127.0.0.0/8".into()],
            ..Default::default()
        };
        assert!(!valid_desired_url("http://127.0.0.1:8931/rpc", &https_only));
    }
}
