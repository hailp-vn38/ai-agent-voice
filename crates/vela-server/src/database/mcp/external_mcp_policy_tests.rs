use super::*;

#[test]
fn network_configuration_only_has_an_optional_host_allowlist() {
    assert_eq!(
        toml::from_str::<ExternalMcpNetworkConfig>("").unwrap(),
        ExternalMcpNetworkConfig::default(),
    );
    for config in ["allow_http_lan = true", "allowed_cidrs = ['127.0.0.0/8']"] {
        assert!(toml::from_str::<ExternalMcpNetworkConfig>(config).is_err());
    }
}

#[test]
fn http_and_https_work_without_network_configuration() {
    let network = ExternalMcpNetworkConfig::default();
    for endpoint in [
        "http://192.168.1.157:8080/mcp",
        "http://127.0.0.1:8080/mcp",
        "http://[::1]:8080/mcp",
        "http://mcp.example.test/mcp",
        "http://198.51.100.1/mcp",
        "https://mcp.example.test/mcp",
    ] {
        assert!(valid_desired_url(endpoint, &network), "{endpoint}");
    }
}

#[test]
fn optional_host_allowlist_restricts_http_and_https() {
    let network = ExternalMcpNetworkConfig {
        allowed_hosts: vec!["mcp.example.test".into(), "*.internal.test".into()],
    };
    for scheme in ["http", "https"] {
        for host in ["mcp.example.test", "tools.internal.test"] {
            assert!(valid_desired_url(
                &format!("{scheme}://{host}/mcp"),
                &network
            ));
        }
        for host in [
            "outside.example.test",
            "internal.test",
            "evilinternal.test",
            "127.0.0.1",
        ] {
            assert!(!valid_desired_url(
                &format!("{scheme}://{host}/mcp"),
                &network
            ));
        }
    }
}

#[test]
fn malformed_urls_and_embedded_credentials_are_rejected() {
    let network = ExternalMcpNetworkConfig::default();
    for endpoint in [
        "not a url",
        "file:///etc/passwd",
        "ftp://mcp.example.test/mcp",
        "http://user:secret@mcp.example.test/mcp",
        "http://mcp.example.test/mcp?token=secret",
        "https://mcp.example.test/mcp#fragment",
    ] {
        assert!(!valid_desired_url(endpoint, &network), "{endpoint}");
    }
}
