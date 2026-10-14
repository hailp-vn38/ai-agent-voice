use super::entities::decode_html_entities;
use crate::services::background::plugins::PluginError;

/// Tìm `>` kết thúc tag bắt đầu ở `lt` (vị trí của `<`), tôn trọng quote
/// đơn/kép bên trong tag. Trả về index của `>`, None nếu tag không đóng.
fn tag_end(bytes: &[u8], lt: usize) -> Option<usize> {
    let mut quote: Option<u8> = None;
    let mut i = lt + 1;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
        } else if b == b'"' || b == b'\'' {
            quote = Some(b);
        } else if b == b'>' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Parse tên tag ở `lt` (vị trí của `<`). Trả về (tên lowercase, là thẻ đóng).
/// Tên rỗng nghĩa là đây không phải tag, ví dụ `< 3` hay `<>`.
fn parse_tag_name(bytes: &[u8], lt: usize) -> (String, bool) {
    let mut idx = lt + 1;
    let mut is_closing = false;
    if idx < bytes.len() && bytes[idx] == b'/' {
        is_closing = true;
        idx += 1;
    }
    let start = idx;
    while idx < bytes.len()
        && (bytes[idx].is_ascii_alphanumeric()
            || bytes[idx] == b'-'
            || bytes[idx] == b':'
            || bytes[idx] == b'_')
    {
        idx += 1;
    }
    if start == idx {
        return (String::new(), is_closing);
    }
    let name = String::from_utf8_lossy(&bytes[start..idx]).to_ascii_lowercase();
    (name, is_closing)
}

/// Tìm thẻ đóng `</tag ...>` từ `from`. Ký tự sau tên tag phải là
/// whitespace / `>` / `/` / EOF để không nhầm `</scriptfoo>` là `</script>`.
fn find_closing_tag(bytes: &[u8], from: usize, tag: &str) -> Option<usize> {
    let tag_bytes = tag.as_bytes();
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == b'<' && bytes[i + 1] == b'/' {
            let name_start = i + 2;
            let name_end = name_start + tag_bytes.len();
            if name_end <= bytes.len()
                && bytes[name_start..name_end].eq_ignore_ascii_case(tag_bytes)
                && (name_end == bytes.len()
                    || matches!(
                        bytes[name_end],
                        b'>' | b'/' | b' ' | b'\t' | b'\n' | b'\r' | b'\x0C'
                    ))
            {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn extract_title(html: &str) -> String {
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let (name, is_closing) = parse_tag_name(bytes, i);
        if !is_closing && name == "title" {
            let Some(open_end) = tag_end(bytes, i) else {
                return String::new();
            };
            let content_start = open_end + 1;
            let content_end = find_closing_tag(bytes, content_start, "title").unwrap_or(html.len());
            return html[content_start..content_end].to_owned();
        }
        // Nhảy qua tag này để không scan từng ký tự bên trong attribute.
        match tag_end(bytes, i) {
            Some(end) => i = end + 1,
            None => return String::new(),
        }
    }
    String::new()
}

/// Strip toàn bộ markup thành text. Các block script/style/noscript/template/
/// title bị bỏ theo đúng tag đóng của chính nó; block không có tag đóng
/// (fetch bị cắt giữa chừng) thì bỏ tới EOF thay vì rò nội dung ra text.
/// Tag thường được bỏ tôn trọng quote nên `id="<Compound Path>"` không làm
/// lệch. Mỗi tag được thay bằng một khoảng trắng để không dính chữ.
fn strip_to_plain_text(html: &str) -> String {
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len().min(1 << 20));
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            let next = html[i..].find('<').map(|p| i + p).unwrap_or(html.len());
            out.push_str(&html[i..next]);
            i = next;
            continue;
        }
        // Comment: <!-- ... -->
        if html[i..].starts_with("<!--") {
            match html[i + 4..].find("-->") {
                Some(end) => {
                    i += 4 + end + 3;
                    out.push(' ');
                    continue;
                }
                None => break, // comment không đóng -> bỏ tới EOF
            }
        }
        // CDATA: giữ nội dung bên trong làm text, bỏ marker.
        if html[i..].starts_with("<![CDATA[") {
            match html[i + 9..].find("]]>") {
                Some(end) => {
                    out.push_str(&html[i + 9..i + 9 + end]);
                    out.push(' ');
                    i += 9 + end + 3;
                    continue;
                }
                None => {
                    out.push_str(&html[i + 9..]);
                    break;
                }
            }
        }
        // <!DOCTYPE ...>, <?...?>, và các <!...> khác.
        if i + 1 < bytes.len() && (bytes[i + 1] == b'!' || bytes[i + 1] == b'?') {
            match tag_end(bytes, i) {
                Some(end) => {
                    i = end + 1;
                    out.push(' ');
                    continue;
                }
                None => break,
            }
        }
        let (name, is_closing) = parse_tag_name(bytes, i);
        if name.is_empty() {
            // Không phải tag, ví dụ `< 3`: giữ `<` như text.
            out.push('<');
            i += 1;
            continue;
        }
        if !is_closing
            && matches!(
                name.as_str(),
                "script" | "style" | "noscript" | "template" | "title"
            )
        {
            let Some(open_end) = tag_end(bytes, i) else {
                break;
            };
            let content_start = open_end + 1;
            match find_closing_tag(bytes, content_start, &name) {
                Some(close_lt) => match tag_end(bytes, close_lt) {
                    Some(close_end) => i = close_end + 1,
                    None => break,
                },
                // Block không đóng (trường hợp fetch bị cắt) -> bỏ tới EOF.
                None => break,
            }
            out.push(' ');
            continue;
        }
        // Tag thường (mở hoặc đóng): bỏ tới `>` tôn trọng quote.
        match tag_end(bytes, i) {
            Some(end) => {
                i = end + 1;
                out.push(' ');
            }
            None => break,
        }
    }
    out
}

/// Best-effort plain-text extraction, không phải browser renderer.
pub(super) fn html_plain_text(html: &str) -> Result<(String, String), PluginError> {
    let title_raw = extract_title(html);
    let title: String = decode_html_entities(&title_raw)
        .trim()
        .chars()
        .take(256)
        .collect();
    let stripped = strip_to_plain_text(html);
    let decoded = decode_html_entities(&stripped);
    let plain = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    Ok((title, plain))
}
