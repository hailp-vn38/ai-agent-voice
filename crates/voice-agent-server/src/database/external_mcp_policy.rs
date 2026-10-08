//! Shared External MCP URL validation for Admin mutations and runtime connections.
use crate::config::ExternalMcpNetworkConfig;
use url::Url;

pub fn valid_desired_url(value: &str, network: &ExternalMcpNetworkConfig) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.scheme(), "https" | "http")
    {
        return false;
    }
    url.host_str().is_some_and(|host| {
        network.allowed_hosts.is_empty()
            || network.allowed_hosts.iter().any(|rule| {
                rule == host
                    || rule.strip_prefix("*.").is_some_and(|suffix| {
                        host.strip_suffix(suffix)
                            .is_some_and(|prefix| prefix.ends_with('.'))
                    })
            })
    })
}

#[cfg(test)]
#[path = "external_mcp_policy_tests.rs"]
mod tests;
