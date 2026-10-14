use agent_job_plugin_sdk::{clean, clipped, positive_limit, run, Failure, Outcome, Request};
use reqwest::{blocking::Client, redirect::Policy};
use scraper::{ElementRef, Html, Selector};
use serde_json::{json, Value};
use std::{collections::HashSet, io::Read, time::Duration};
use url::Url;

const MAX_HTML_BYTES: u64 = 1024 * 1024;

fn arg_string<'a>(req: &'a Request, key: &str) -> Result<&'a str, Failure> {
    req.arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or(Failure::new(
            "invalid_arguments",
            format!("Missing string: {key}"),
        ))
}
fn optional_limits(req: &Request) -> Result<(usize, usize), Failure> {
    Ok((
        positive_limit(req.arguments.get("max_chars"), 12000, 20000)?,
        positive_limit(req.arguments.get("max_links"), 30, 100)?,
    ))
}
fn http_url(s: &str) -> Result<Url, Failure> {
    let url = Url::parse(s).map_err(|_| Failure::new("invalid_url", "Malformed URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host().is_none()
    {
        return Err(Failure::new(
            "invalid_url",
            "Only HTTP(S) URLs without embedded credentials are supported",
        ));
    }
    Ok(url)
}
fn fetch(url: &str) -> Result<String, Failure> {
    let url = http_url(url)?;
    // Use the Docker worker's network directly; this plugin does not follow redirects.
    // URL scheme validation is plugin-local; there is no host or port allowlist.
    let client = Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(20))
        .user_agent("AgentJob-WebExtractor/1.0")
        .build()
        .map_err(|_| Failure::new("network_unavailable", "HTTP client unavailable"))?;
    let response = client
        .get(url)
        .send()
        .map_err(|_| Failure::new("fetch_failed", "Request failed"))?;
    if !response.status().is_success() {
        return Err(Failure::new(
            "http_error",
            format!("HTTP {}", response.status().as_u16()),
        ));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !(content_type.contains("text/html") || content_type.contains("application/xhtml+xml")) {
        return Err(Failure::new(
            "unsupported_media_type",
            "URL did not return HTML",
        ));
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_HTML_BYTES)
    {
        return Err(Failure::new("page_too_large", "HTML exceeds 1 MiB"));
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_HTML_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::new("fetch_failed", "Unable to read response body"))?;
    if bytes.len() as u64 > MAX_HTML_BYTES {
        return Err(Failure::new("page_too_large", "HTML exceeds 1 MiB"));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
fn first_text(html: &Html, selector: &str) -> String {
    Selector::parse(selector)
        .ok()
        .and_then(|s| html.select(&s).next())
        .map(|el| clean(&el.text().collect::<Vec<_>>().join(" ")))
        .unwrap_or_default()
}
fn attr(html: &Html, selector: &str, attribute: &str) -> String {
    Selector::parse(selector)
        .ok()
        .and_then(|s| html.select(&s).next())
        .and_then(|el| el.value().attr(attribute))
        .unwrap_or("")
        .to_owned()
}
fn readable(element: &ElementRef<'_>) -> String {
    let text: Vec<_> = element
        .descendants()
        .filter_map(|node| {
            let scraper::Node::Text(text) = node.value() else {
                return None;
            };
            if node.ancestors().filter_map(ElementRef::wrap).any(|parent| {
                matches!(
                    parent.value().name(),
                    "script" | "style" | "noscript" | "template" | "nav" | "footer"
                )
            }) {
                return None;
            }
            Some(text.text.as_ref())
        })
        .collect();
    clean(&text.join(" "))
}
fn parse(html: &str, page_url: Option<&str>, max_chars: usize, max_links: usize) -> Value {
    let doc = Html::parse_document(html);
    let title = first_text(&doc, "title");
    let description = attr(&doc, r#"meta[name="description"]"#, "content");
    let root = ["article", "main", "body"].iter().find_map(|query| {
        let selector = Selector::parse(query).expect("static selector");
        doc.select(&selector).next()
    });
    let paragraphs =
        Selector::parse("h1, h2, h3, h4, h5, h6, p, li, blockquote").expect("static selector");
    let mut chunks: Vec<String> = Vec::new();
    if let Some(root) = root {
        for node in root.select(&paragraphs) {
            let txt = readable(&node);
            if !txt.is_empty() && chunks.last() != Some(&txt) {
                chunks.push(txt);
            }
        }
    }
    let text = if chunks.is_empty() {
        root.map(|x| readable(&x)).unwrap_or_default()
    } else {
        chunks.join("\n")
    };
    let word_count = text.split_whitespace().count();
    let (text, truncated) = clipped(&text, max_chars);

    let heading_selector = Selector::parse("h1,h2,h3,h4,h5,h6").expect("static selector");
    let headings: Vec<_> = doc
        .select(&heading_selector)
        .take(80)
        .filter_map(|el| {
            let t = readable(&el);
            if t.is_empty() {
                return None;
            }
            let level = el
                .value()
                .name()
                .chars()
                .nth(1)
                .and_then(|c| c.to_digit(10))
                .unwrap_or(1);
            Some(json!({"level": level, "text": clipped(&t, 200).0}))
        })
        .collect();
    let link_selector = Selector::parse("a[href]").expect("static selector");
    let base = page_url.and_then(|s| Url::parse(s).ok());
    let mut seen = HashSet::new();
    let mut links = Vec::new();
    for link in doc.select(&link_selector) {
        if links.len() >= max_links {
            break;
        }
        let href = link.value().attr("href").unwrap_or("");
        let resolved = base
            .as_ref()
            .and_then(|b| b.join(href).ok())
            .or_else(|| Url::parse(href).ok());
        let Some(url) = resolved else { continue };
        if !matches!(url.scheme(), "http" | "https") {
            continue;
        }
        let dest = url.to_string();
        if !seen.insert(dest.clone()) {
            continue;
        }
        links.push(json!({"text": clipped(&readable(&link), 160).0, "url": dest}));
    }
    json!({
        "url": page_url, "title": clipped(&title, 300).0,
        "description": clipped(&description, 1000).0,
        "text": text, "headings": headings, "links": links,
        "word_count": word_count, "truncated": truncated
    })
}
fn execute(req: &Request) -> Result<Outcome, Failure> {
    let (chars, links) = optional_limits(req)?;
    let (html, url) = match req.operation.as_str() {
        "extract_html" => {
            let html = arg_string(req, "html")?;
            if html.len() > 50_000 {
                return Err(Failure::new("page_too_large", "HTML input exceeds 50 KiB"));
            }
            let url = req.arguments.get("url").and_then(Value::as_str);
            if let Some(url) = url {
                http_url(url)?;
            }
            (html.to_owned(), url.map(str::to_owned))
        }
        "fetch_url" => {
            let url = arg_string(req, "url")?;
            (fetch(url)?, Some(url.to_owned()))
        }
        _ => {
            return Err(Failure::new(
                "unsupported_operation",
                "Unknown web operation",
            ))
        }
    };
    let result = parse(&html, url.as_deref(), chars, links);
    let mut output = Outcome::new(result.clone());
    let json_bytes = serde_json::to_vec_pretty(&result)
        .map_err(|_| Failure::new("serialize_failed", "Cannot serialize output"))?;
    output.attach(
        &req.artifact_dir,
        "page.json",
        "application/json",
        &json_bytes,
    )?;
    let report = format!(
        "# {}\n\nSource: {}\n\n{}\n",
        result["title"].as_str().unwrap_or("Untitled"),
        result["url"].as_str().unwrap_or("provided-html"),
        result["text"].as_str().unwrap_or("")
    );
    output.attach(
        &req.artifact_dir,
        "page.md",
        "text/markdown",
        report.as_bytes(),
    )?;
    Ok(output)
}
fn main() {
    run(execute);
}

#[cfg(test)]
mod tests;
