use agent_job_plugin_sdk::{clean, clipped, positive_limit, run, Failure, Outcome, Request};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{Cursor, Read},
};

fn extract_pdf(bytes: &[u8]) -> Result<(String, Option<usize>), Failure> {
    let doc = lopdf::Document::load_mem(bytes)
        .map_err(|_| Failure::new("invalid_pdf", "Cannot open PDF"))?;
    if doc.is_encrypted() {
        return Err(Failure::new(
            "encrypted_pdf",
            "Encrypted PDFs are not supported",
        ));
    }
    let pages = doc.get_pages();
    if pages.is_empty() || pages.len() > 100 {
        return Err(Failure::new(
            "unsupported_pdf",
            "PDF must contain 1-100 pages",
        ));
    }
    let mut text = String::new();
    for number in pages.keys() {
        let page = doc
            .extract_text(&[*number])
            .map_err(|_| Failure::new("invalid_pdf", "Cannot extract page text"))?;
        text.push_str(&page);
        text.push('\n');
    }
    if text.trim().is_empty() {
        return Err(Failure::new(
            "no_extractable_text",
            "No selectable text (OCR not supported)",
        ));
    }
    Ok((text, Some(pages.len())))
}

fn extract_docx(bytes: &[u8]) -> Result<String, Failure> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| Failure::new("invalid_docx", "Invalid ZIP container"))?;
    if archive.len() > 500 {
        return Err(Failure::new("invalid_docx", "Too many DOCX parts"));
    }
    let xml_file = archive
        .by_name("word/document.xml")
        .map_err(|_| Failure::new("invalid_docx", "Missing document.xml"))?;
    if xml_file.size() > 2 * 1024 * 1024 {
        return Err(Failure::new("document_too_large", "DOCX XML too large"));
    }
    let mut xml = Vec::new();
    xml_file
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut xml)
        .map_err(|_| Failure::new("invalid_docx", "Cannot read document.xml"))?;
    if xml.len() > 2 * 1024 * 1024 {
        return Err(Failure::new("document_too_large", "DOCX XML too large"));
    }
    let mut reader = quick_xml::Reader::from_reader(xml.as_slice());
    reader.trim_text(false);
    let mut buf = Vec::new();
    let mut text = String::new();
    let mut inside_text = false;
    let mut depth: usize = 0;
    loop {
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|_| Failure::new("invalid_docx", "Malformed XML"))?;
        match &event {
            quick_xml::events::Event::Start(_) => depth += 1,
            quick_xml::events::Event::End(_) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or(Failure::new("invalid_docx", "Malformed XML"))?;
            }
            _ => {}
        }
        match event {
            quick_xml::events::Event::Start(e) if e.local_name().as_ref() == b"t" => {
                inside_text = true
            }
            quick_xml::events::Event::End(e) if e.local_name().as_ref() == b"t" => {
                inside_text = false
            }
            quick_xml::events::Event::End(e) if e.local_name().as_ref() == b"p" => text.push('\n'),
            quick_xml::events::Event::Empty(e) if e.local_name().as_ref() == b"tab" => {
                text.push(' ')
            }
            quick_xml::events::Event::Empty(e) if e.local_name().as_ref() == b"br" => {
                text.push('\n')
            }
            quick_xml::events::Event::Text(e) if inside_text => {
                let value = e
                    .unescape()
                    .map_err(|_| Failure::new("invalid_docx", "Malformed text element"))?;
                text.push_str(&value);
            }
            quick_xml::events::Event::Eof => {
                if depth != 0 {
                    return Err(Failure::new("invalid_docx", "Unclosed XML element"));
                }
                break;
            }
            _ => {}
        }
        buf.clear();
    }
    if text.trim().is_empty() {
        return Err(Failure::new("no_extractable_text", "DOCX has no text"));
    }
    Ok(text)
}

fn extract_markdown(source: &str) -> (String, Vec<String>) {
    let mut text = String::new();
    let mut headings = Vec::new();
    let mut active_heading: Option<String> = None;
    for event in Parser::new(source) {
        match event {
            Event::Start(Tag::Heading { .. }) => active_heading = Some(String::new()),
            Event::End(TagEnd::Heading(_)) => {
                if let Some(title) = active_heading.take() {
                    if !title.trim().is_empty() && headings.len() < 80 {
                        headings.push(clean(&title));
                    }
                }
                text.push('\n');
            }
            Event::End(TagEnd::Paragraph) => text.push('\n'),
            Event::SoftBreak | Event::HardBreak => text.push(' '),
            Event::Text(value) | Event::Code(value) => {
                text.push_str(&value);
                text.push(' ');
                if let Some(title) = active_heading.as_mut() {
                    title.push_str(&value);
                    title.push(' ');
                }
            }
            _ => {}
        }
    }
    (text, headings)
}

fn keywords(text: &str) -> Vec<Value> {
    let stop = [
        "the", "and", "for", "with", "are", "this", "that", "from", "của", "và", "những", "các",
        "được", "cho", "trong", "một", "với", "là", "có",
    ];
    let mut counts = HashMap::<String, usize>::new();
    for token in text.split(|c: char| !c.is_alphanumeric()) {
        let word = token.to_lowercase();
        if word.chars().count() < 3 || stop.contains(&word.as_str()) {
            continue;
        }
        *counts.entry(word).or_default() += 1;
    }
    let mut ranked: Vec<_> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked
        .into_iter()
        .take(12)
        .map(|(term, count)| json!({"term":term,"count":count}))
        .collect()
}

fn analyze(req: &Request) -> Result<Outcome, Failure> {
    if req.operation != "analyze_document" {
        return Err(Failure::new("unsupported_operation", "Unknown operation"));
    }
    let format = req
        .arguments
        .get("format")
        .and_then(Value::as_str)
        .ok_or(Failure::new("invalid_arguments", "format is required"))?;
    let max_chars = positive_limit(req.arguments.get("max_chars"), 10000, 12000)?;
    let (source, pages, headings) = match format {
        "markdown" => {
            let text = req
                .arguments
                .get("text")
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty() && v.len() <= 50000)
                .ok_or(Failure::new("invalid_arguments", "text is required"))?;
            let (source, headings) = extract_markdown(text);
            (source, None, headings)
        }
        "pdf" | "docx" => {
            let encoded = req
                .arguments
                .get("content_base64")
                .and_then(Value::as_str)
                .filter(|s| s.len() <= 60000)
                .ok_or(Failure::new(
                    "invalid_arguments",
                    "content_base64 is required",
                ))?;
            let bytes = STANDARD
                .decode(encoded)
                .map_err(|_| Failure::new("invalid_base64", "Invalid base64"))?;
            if bytes.len() > 45 * 1024 {
                return Err(Failure::new(
                    "document_too_large",
                    "Inline file limit is 45 KiB",
                ));
            }
            if format == "pdf" {
                let (source, count) = extract_pdf(&bytes)?;
                (source, count, vec![])
            } else {
                (extract_docx(&bytes)?, None, vec![])
            }
        }
        _ => return Err(Failure::new("invalid_arguments", "Unsupported format")),
    };
    let source = clean(&source);
    if source.is_empty() {
        return Err(Failure::new("no_extractable_text", "Empty document"));
    }
    let count_chars = source.chars().count();
    let count_words = source.split_whitespace().count();
    let title = headings
        .first()
        .cloned()
        .or_else(|| {
            req.arguments
                .get("filename")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| clipped(&source, 80).0);
    let (text, truncated) = clipped(&source, max_chars);
    let result = json!({
        "format":format, "title":clipped(&clean(&title), 160).0,
        "text":text, "excerpt":clipped(&source, 300).0,
        "headings":headings, "keywords":keywords(&source),
        "word_count":count_words, "character_count":count_chars,
        "page_count":pages, "truncated":truncated
    });
    let mut output = Outcome::new(result.clone());
    let pretty = serde_json::to_vec_pretty(&result)
        .map_err(|_| Failure::new("serialize_failed", "JSON serialization failed"))?;
    output.attach(
        &req.artifact_dir,
        "document.json",
        "application/json",
        &pretty,
    )?;
    let markdown = format!(
        "# {}\n\nFormat: {} | Words: {} | Characters: {}\n\n{}\n",
        result["title"].as_str().unwrap_or("Document"),
        format,
        count_words,
        count_chars,
        result["text"].as_str().unwrap_or("")
    );
    output.attach(
        &req.artifact_dir,
        "document.md",
        "text/markdown",
        markdown.as_bytes(),
    )?;
    Ok(output)
}
fn main() {
    run(analyze);
}

#[cfg(test)]
mod tests;
