use super::*;

#[test]
fn excludes_scripts_and_navigation_from_readable_content() {
    let result = parse("<main><nav><p>Navigation</p><h2>Menu</h2></nav><h1>Title</h1><p>Text<script>secret</script><style>hidden</style></p></main>", None, 200, 10);
    assert_eq!(result["text"], "Title\nText");
    assert_eq!(result["headings"], json!([{"level":1,"text":"Title"}]));
}
#[test]
fn extracts_main_text_and_metadata() {
    let html = r#"<html><head><title>Example</title><meta name="description" content="A test"></head><body><main><h1>Hello</h1><p>World content</p><a href="/x">More</a></main></body></html>"#;
    let output = parse(html, Some("https://example.com/"), 200, 10);
    assert_eq!(output["title"], "Example");
    assert_eq!(output["description"], "A test");
    assert_eq!(output["word_count"], 3);
    assert_eq!(output["links"][0]["url"], "https://example.com/x");
}

#[test]
fn accepts_http_urls_without_destination_allowlist() {
    for address in [
        "http://127.0.0.1/secret",
        "http://169.254.169.254/latest",
        "http://[::ffff:127.0.0.1]/secret",
        "http://[::1]/secret",
        "http://localhost:8080/",
        "https://example.com:8443/path",
    ] {
        assert!(http_url(address).is_ok(), "{address}");
    }
    assert!(http_url("file:///etc/passwd").is_err());
    assert!(http_url("https://user:pass@example.com/").is_err());
}

#[test]
fn declared_settings_supply_bounded_defaults_and_operation_arguments_override_them() {
    let mut request = Request {
        schema_version: 1,
        request_id: "fixture".into(),
        operation: "extract_html".into(),
        arguments: json!({}),
        artifact_dir: "/tmp".into(),
        inputs: None,
        settings: Some(json!({
            "default_max_chars": 1234,
            "default_max_links": 5
        })),
    };
    assert_eq!(optional_limits(&request).unwrap(), (1234, 5));
    request.arguments = json!({"max_chars": 120, "max_links": 2});
    assert_eq!(optional_limits(&request).unwrap(), (120, 2));
    request.settings = None;
    request.arguments = json!({});
    assert_eq!(optional_limits(&request).unwrap(), (12000, 30));
}

#[test]
fn executes_real_plugin_operation_with_declared_settings() {
    let dir = std::env::temp_dir().join(format!(
        "web-extractor-settings-{}", std::process::id()
    ));
    std::fs::create_dir(&dir).unwrap();
    let request = Request {
        schema_version: 1,
        request_id: "worker-e2e".into(),
        operation: "extract_html".into(),
        arguments: json!({
            "html": "<main><p>A long text with enough words</p><a href=\"https://example.com/one\">One</a><a href=\"https://example.com/two\">Two</a></main>"
        }),
        artifact_dir: dir.to_string_lossy().into_owned(),
        inputs: None,
        settings: Some(json!({
            "default_max_chars": 8,
            "default_max_links": 1
        })),
    };
    let outcome = execute(&request).unwrap();
    assert_eq!(outcome.result["text"], "A long t");
    assert_eq!(outcome.result["links"].as_array().unwrap().len(), 1);
    assert!(dir.join("page.json").is_file());
    assert!(dir.join("page.md").is_file());
    std::fs::remove_dir_all(dir).unwrap();
}
