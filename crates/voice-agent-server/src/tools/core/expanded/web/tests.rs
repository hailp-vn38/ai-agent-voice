use super::*;

#[tokio::test]
#[ignore = "requires the live 24h website"]
async fn extracts_live_24h_article() {
    let result = execute(&json!({"url":"https://www.24h.com.vn/bong-da/vff-thong-tin-ve-viec-hoan-vck-asian-cup-2027-c48a1802459.html", "max_chars":1000}), true).await.unwrap();
    assert_eq!(result["status"], 200);
    assert_eq!(
        result["title"],
        "VFF thông tin về việc hoãn VCK Asian Cup 2027"
    );
    let text = result["text"].as_str().unwrap();
    assert!(
        text.contains("Ngày 13/10"),
        "article introduction missing ({} chars)",
        text.chars().count()
    );
    assert!(text.contains("Saudi Arabia"), "article body missing");
}

#[tokio::test]
async fn prioritizes_article_over_navigation_before_text_limit() {
    let body = format!(
        "<title>Article</title><nav>{}</nav>\
         <article class=\"teaser\"><a>Related headline</a></article>\
         <article class=\"cate-24h-foot-arti-deta-info\"><h2>INTRO</h2>\
         <p>ARTICLE_BODY</p><div><p>ARTICLE_END</p></div></article><footer>FOOTER</footer>",
        "Menu ".repeat(400)
    );
    let (url, server) = serve_html(body).await;
    let result = execute(&json!({"url":url,"max_chars":1000}), true).await;
    server.abort();
    let result = result.unwrap();
    assert_eq!(result["text"], "INTRO ARTICLE_BODY ARTICLE_END");
    assert_eq!(result["truncated"], false);
}

#[test]
fn strips_active_html_and_extracts_title() {
    let (title, text) = html_plain_text(
        "<html><head><title>A &amp; B</title><script>alert(1)</script></head>\
         <body><h1>Hi &lt;there&gt;</h1><p>World</p></body></html>",
        true,
    )
    .unwrap();
    assert_eq!(title, "A & B");
    assert_eq!(text, "Hi <there> World");
    assert!(!text.contains("alert"));
}

#[test]
fn article_selection_ignores_script_markup_and_handles_nesting() {
    let (_, text) = html_plain_text(
        "<script>const fake = '<article>FAKE</article>';</script>\
         <!-- <article>COMMENT</article> -->\
         <article><p>INTRO</p><article><p>NESTED</p></article><p>END</p></article>\
         <article><script>padding padding padding padding</script>Teaser</article>",
        true,
    )
    .unwrap();
    assert_eq!(text, "INTRO NESTED END");
}

#[test]
fn falls_back_to_page_for_empty_or_incomplete_articles() {
    for body in [
        "<p>KEEP</p><article> </article>",
        "<p>KEEP</p><article><script>hidden</script></article>",
        "<p>KEEP</p><article>",
        "<article/><p>KEEP</p>",
    ] {
        let (_, text) = html_plain_text(body, true).unwrap();
        assert_eq!(text, "KEEP");
    }
    let (_, text) = html_plain_text("<p>BEFORE</p><article><p>PARTIAL</p>", true).unwrap();
    assert_eq!(text, "BEFORE PARTIAL");
}

#[test]
fn decodes_full_named_and_numeric_entities() {
    let (_, text) = html_plain_text(
        "<p>B&oacute;ng đ&aacute; &amp; &#38; &#x27; &Ocirc; &ocirc; \
         &mdash; &hellip; &euro;</p>",
        true,
    )
    .unwrap();
    assert_eq!(text, "Bóng đá & & ' Ô ô — … €");
    // Không decode kép: &amp;lt; -> &lt;, không phải <.
    let (_, text) = html_plain_text("<p>&amp;lt;tag&gt;</p>", true).unwrap();
    assert_eq!(text, "&lt;tag>");
}

#[test]
fn does_not_leak_truncated_style_block() {
    // Mô phỏng vết cắt 64 KB cũ: cắt giữa <style> đầu tiên.
    let padding = "x".repeat(70_000);
    let full = format!(
        "<html><head><title>T</title><style>{padding}</style></head>\
         <body><p>ARTICLE_SENTINEL</p></body></html>"
    );
    // Full phải lấy được bài.
    let (_, text) = html_plain_text(&full, true).unwrap();
    assert!(text.contains("ARTICLE_SENTINEL"));
    assert!(!text.contains('x'));
    // Lát cắt 64 KB (giữa <style>) không được rò CSS ra text.
    let truncated = String::from_utf8_lossy(&full.as_bytes()[..65_536]).into_owned();
    let (_, text) = html_plain_text(&truncated, true).unwrap();
    assert!(
        !text.contains('x'),
        "CSS bị rò ra text: {}",
        &text[..text.len().min(200)]
    );
    assert!(!text.contains("ARTICLE_SENTINEL"));
}

#[test]
fn does_not_cross_mismatched_close_tags() {
    // Code cũ dùng alternation chung cho close tag nên
    // <script>...</style> sẽ match sai. Code mới tách theo từng tag.
    let (_, text) = html_plain_text(
        "<script>var a = 1;</style>LEAK</scriptfoo></script><style>.a{color:red}</style>\
         <p>KEEP</p>",
        true,
    )
    .unwrap();
    assert_eq!(text, "KEEP");
}

#[test]
fn strips_tag_with_gt_inside_quoted_attribute() {
    // Case thật trên 24h: <path id="<Compound Path>" fill-rule=...>
    let (_, text) = html_plain_text(
        r#"<svg><path id="<Compound Path>" fill-rule="evenodd" d="m1 2"></path></svg><p>KEEP</p>"#,
        true,
    )
    .unwrap();
    assert_eq!(text, "KEEP");
    assert!(!text.contains("fill-rule"));
}

#[test]
fn strips_unclosed_script_to_eof() {
    let (_, text) = html_plain_text("<p>BEFORE</p><script>var x = 1; // không đóng", true).unwrap();
    assert_eq!(text, "BEFORE");
}

#[test]
fn strips_comments_and_keeps_separate_words() {
    let (_, text) =
        html_plain_text("<p>foo</p><!-- comment --><p>bar</p><div>baz</div>", true).unwrap();
    assert_eq!(text, "foo bar baz");
}

async fn serve_html(body: String) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let app = axum::Router::new().route(
        "/",
        axum::routing::get(move || {
            let body = body.clone();
            async move { axum::response::Html(body) }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, server)
}

#[tokio::test]
async fn extracts_article_after_large_head_and_bounds_returned_text() {
    let padding = "x".repeat(160_000);
    let body = format!(
        "<html><head><title>B&oacute;ng đá</title><style>{padding}</style></head>\
         <body><p>ARTICLE_SENTINEL</p></body></html>"
    );
    let (url, server) = serve_html(body).await;
    let result = execute(&json!({"url":url}), true).await;
    let limited = execute(&json!({"url":url,"max_chars":7}), true).await;
    let fetched = execute(&json!({"url":url,"max_chars":7}), false).await;
    server.abort();
    let result = result.unwrap();
    assert_eq!(result["title"], "Bóng đá");
    assert_eq!(result["text"], "ARTICLE_SENTINEL");
    assert_eq!(result["status"], 200);
    assert_eq!(result["truncated"], false);
    assert_eq!(result["provenance"][0]["capabilities"][0], "web_extract");
    let limited = limited.unwrap();
    assert_eq!(limited["text"], "ARTICLE");
    assert_eq!(limited["truncated"], true);
    let fetched = fetched.unwrap();
    assert_eq!(fetched["text"], "<html><");
    assert_eq!(fetched["truncated"], true);
    assert_eq!(fetched["provenance"][0]["capabilities"][0], "web_fetch");
}

#[tokio::test]
async fn reports_body_limit_without_leaking_cut_style() {
    let body = format!(
        "<p>KEEP</p><style>{}</style><p>AFTER</p>",
        "x".repeat(BODY_LIMIT)
    );
    let (url, server) = serve_html(body).await;
    let result = execute(&json!({"url":url}), true).await;
    server.abort();
    let result = result.unwrap();
    assert_eq!(result["text"], "KEEP");
    assert_eq!(result["truncated"], true);
    assert_eq!(result["body_limit_reached"], true);
    assert_eq!(result["next_offset"], Value::Null);
}

#[tokio::test]
async fn paginates_unicode_to_end_for_fetch_and_both_extract_modes() {
    let body =
        "<title>T</title><nav>Menu</nav><article>Chào 😀 thế giới!</article><footer>End</footer>";
    let (url, server) = serve_html(body.into()).await;
    for (extract, mode, expected) in [
        (false, "article", body),
        (true, "article", "Chào 😀 thế giới!"),
        (true, "page", "Menu Chào 😀 thế giới! End"),
    ] {
        let operation = if extract { "web.extract" } else { "web.fetch" };
        let choice = crate::tools::core::decode(&json!({
            "kind":"core","plugin_id":"core","operation":operation,"contract_revision":"1"
        }))
        .unwrap();
        let schema = crate::tools::core::contract(&choice).unwrap();
        let validator = jsonschema::validator_for(&schema["output_schema"]).unwrap();
        let mut offset = 0;
        let mut joined = String::new();
        loop {
            let mut args = json!({"url":url,"offset":offset,"max_chars":3});
            if extract {
                args["mode"] = json!(mode);
            }
            let page = execute(&args, extract).await.unwrap();
            assert!(validator.is_valid(&page));
            assert_eq!(page["offset"], offset);
            assert_eq!(page["body_limit_reached"], false);
            let text = page["text"].as_str().unwrap();
            assert!(text.chars().count() <= 3);
            joined.push_str(text);
            match page["next_offset"].as_u64() {
                Some(next) => {
                    assert_eq!(next, offset + text.chars().count() as u64);
                    assert!(next > offset);
                    assert_eq!(page["truncated"], true);
                    offset = next;
                }
                None => {
                    assert_eq!(page["truncated"], false);
                    break;
                }
            }
        }
        assert_eq!(joined, expected);
        for offset in [expected.chars().count(), expected.chars().count() + 10] {
            let mut args = json!({"url":url,"offset":offset});
            if extract {
                args["mode"] = json!(mode);
            }
            let page = execute(&args, extract).await.unwrap();
            assert_eq!(page["text"], "");
            assert_eq!(page["next_offset"], Value::Null);
            assert_eq!(page["truncated"], false);
        }
    }
    server.abort();
}

#[tokio::test]
async fn default_page_limit_can_be_resumed_and_empty_page_is_exhausted() {
    let body = "é".repeat(TEXT_LIMIT + 1);
    let (url, server) = serve_html(body.clone()).await;
    for extract in [false, true] {
        let page = execute(&json!({"url":url}), extract).await.unwrap();
        assert_eq!(page["text"].as_str().unwrap().chars().count(), TEXT_LIMIT);
        assert_eq!(page["offset"], 0);
        assert_eq!(page["next_offset"], TEXT_LIMIT);
        let last = execute(&json!({"url":url,"offset":page["next_offset"]}), extract)
            .await
            .unwrap();
        assert_eq!(last["text"], "é");
        assert_eq!(last["next_offset"], Value::Null);
        assert_eq!(last["truncated"], false);
    }
    server.abort();
    let (url, server) = serve_html(String::new()).await;
    for extract in [false, true] {
        let page = execute(&json!({"url":url}), extract).await.unwrap();
        assert_eq!(page["text"], "");
        assert_eq!(page["next_offset"], Value::Null);
        assert_eq!(page["truncated"], false);
    }
    server.abort();
}
