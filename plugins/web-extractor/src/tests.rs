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
