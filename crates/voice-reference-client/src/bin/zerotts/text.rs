//! Vietnamese text frontend for the local reference runner.
//! Rules and abbreviation readings follow reference/ZeroTTS-main/src/zerotts/text_norm.
//! The upstream rules derive from soe-vinorm (MIT); see its bundled LICENSE.soe-vinorm.

use super::*;
use regex::{Captures, Regex};

const DIGITS: [&str; 10] = [
    "không", "một", "hai", "ba", "bốn", "năm", "sáu", "bảy", "tám", "chín",
];
const SCALES: [&str; 7] = ["", "nghìn", "triệu", "tỷ", "nghìn tỷ", "triệu tỷ", "tỷ tỷ"];
const ABBREVIATIONS: &str = include_str!("../../../../../assets/zerotts-text/abbreviations.txt");

#[derive(Clone, Copy)]
enum Form {
    TimeSeconds,
    Date,
    MonthYear,
    DayMonth,
    TimeMinutes,
    TimeH,
    Hour,
    Version,
    BareVersion,
    Fraction,
    Degree,
    Percent,
    Number,
    Prefix,
    AcronymPair,
    Code,
    Acronym,
    At,
}

fn patterns() -> &'static Vec<(Form, Regex)> {
    static PATTERNS: OnceLock<Vec<(Form, Regex)>> = OnceLock::new();
    PATTERNS.get_or_init(|| [
        (Form::TimeSeconds, r"^(?P<a>[01]?[0-9]|2[0-3])[:hg](?P<b>[0-5]?[0-9])[:mp](?P<c>[0-5]?[0-9])"),
        (Form::Date, r"^(?P<a>0?[1-9]|[12][0-9]|3[01])(?P<s>[/.-])(?P<b>0?[1-9]|1[0-2])(?P<t>[/.-])(?P<c>[12][0-9]{3})"),
        (Form::MonthYear, r"^(?P<a>0?[1-9]|1[0-2])[/.-](?P<b>1[0-9]{3}|20[0-9]{2}|21[0-9]{2})"),
        (Form::DayMonth, r"^(?P<cue>(?i:ngày|mùng|mồng|hôm|sáng|trưa|chiều|tối|đêm|từ|đến|và|hoặc))(?P<gap>\s+)(?P<a>0?[1-9]|[12][0-9]|3[01])[/\-](?P<b>0?[1-9]|1[0-2])"),
        (Form::TimeMinutes, r"^(?P<a>[01]?[0-9]|2[0-3]):(?P<b>[0-5][0-9])"),
        (Form::TimeH, r"^(?P<a>[01]?[0-9]|2[0-3])[hg](?P<b>[0-5][0-9])"),
        (Form::Hour, r"^(?P<a>[01]?[0-9]|2[0-3])[hg]"),
        (Form::Version, r"^(?P<p>[vV])(?P<n>[0-9]+(?:\.[0-9]+)+)"),
        (Form::BareVersion, r"^(?P<n>[0-9]+(?:\.[0-9]+){2,})"),
        (Form::Fraction, r"^(?P<a>[0-9]+)\s*/\s*(?P<b>[0-9]+)"),
        (Form::Degree, r"^(?P<n>-?[0-9][0-9.,]*)\s*°\s*(?P<u>[CF])?"),
        (Form::Percent, r"^(?P<n>[-+]?[0-9][0-9.,]*?)\s*%"),
        (Form::Number, r"^(?P<n>[-+]?[0-9][0-9.,]*(?:\s*[*^+]\s*[-+]?[0-9][0-9.,]*|\s+[-/]\s+[-+]?[0-9][0-9.,]*|[*^]\s*[-+]?[0-9][0-9.,]*)*)"),
        (Form::Prefix, r"^(?P<n>TP|TX|TT|KP|[QPH])\."),
        (Form::AcronymPair, r"^(?P<n>[\p{Lu}]{1,6}/[\p{Lu}]{1,6})"),
        (Form::Code, r"^(?P<a>[\p{Lu}]{1,4})-?(?P<b>[0-9]{1,6})"),
        (Form::Acronym, r"^(?P<n>[\p{Lu}][\p{Lu}0-9]+(?:\.[\p{Lu}][\p{Lu}0-9]*)*)"),
        (Form::At, r"^@"),
    ].into_iter().map(|(kind, pattern)| (kind, Regex::new(pattern).expect("valid text pattern"))).collect())
}

fn protected() -> &'static Regex {
    static PROTECTED: OnceLock<Regex> = OnceLock::new();
    PROTECTED.get_or_init(|| Regex::new(r"(?i)(?:https?|ftp)://\S+|www\.\S+|[\w.+-]+@[\w-]+(?:\.[\w-]+)+|\b[\w-]+(?:\.[\w-]+)*\.(?:com|net|org|vn|io|edu|gov|info|dev|ai)\b(?:/\S*)?").expect("valid protected pattern"))
}

fn group<'a>(caps: &'a Captures<'_>, name: &str) -> &'a str {
    caps.name(name).map_or("", |m| m.as_str())
}
fn digit(c: char) -> &'static str {
    c.to_digit(10)
        .and_then(|n| DIGITS.get(n as usize).copied())
        .unwrap_or("")
}
fn read_digits(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| {
            if c == ',' {
                "phẩy".to_owned()
            } else {
                let d = digit(c);
                if d.is_empty() {
                    c.to_string()
                } else {
                    d.to_owned()
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
fn sandhi(s: String) -> String {
    s.replace("mười năm", "mười lăm")
        .replace("mươi năm", "mươi lăm")
        .replace("mươi bốn", "mươi tư")
        .replace("mươi một", "mươi mốt")
        .replace("linh bốn", "linh tư")
}
fn speak_chunk(chunk: &str, scale: usize) -> Option<String> {
    if chunk == "000" {
        return Some(String::new());
    }
    let cs: Vec<char> = chunk.chars().collect();
    let mut result = String::new();
    for pos in (0..cs.len()).rev() {
        let c = cs[pos];
        if pos == cs.len() - 1 && c == '0' && cs.len() > 1 {
            continue;
        }
        if pos + 2 == cs.len() && (c == '1' || c == '0') {
            if pos == 0 && c == '0' {
                continue;
            }
            result = if cs[pos] == '1' {
                if cs[pos + 1] != '0' {
                    format!("mười {}", digit(cs[pos + 1]))
                } else {
                    "mười".into()
                }
            } else if cs[pos + 1] != '0' {
                format!("linh {}", digit(cs[pos + 1]))
            } else {
                String::new()
            };
        } else {
            let unit = ["", "mươi", "trăm"][cs.len() - pos - 1];
            result = format!("{} {} {}", digit(c), unit, result)
                .trim()
                .to_owned();
        }
    }
    Some(
        format!("{} {}", result.trim(), SCALES.get(scale)?)
            .trim()
            .to_owned(),
    )
}

fn plain_number(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }
    let (sign, mut number) = match input.chars().next().unwrap() {
        '-' => ("trừ", &input[1..]),
        '+' => ("cộng", &input[1..]),
        _ => ("", input),
    };
    while number.len() > 1 && number.starts_with('0') && number.as_bytes()[1].is_ascii_digit() {
        number = &number[1..];
    }
    let mut number: String = number
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
        .collect();
    let mut decimal = String::new();
    if number.matches(',').count() == 1 {
        number = number.replace('.', "");
        if let Some((whole, tail)) = number.split_once(',') {
            decimal = format!("phẩy {}", read_digits(tail));
            number = whole.to_owned();
        }
    } else if number.matches('.').count() == 1
        && number
            .split_once('.')
            .is_some_and(|(_, tail)| tail.len() <= 2)
    {
        number = number.replace(',', "");
        if let Some((whole, tail)) = number.split_once('.') {
            decimal = format!("chấm {}", read_digits(tail));
            number = whole.to_owned();
        }
    } else {
        number = number.replace('.', "");
    }
    let mut pieces = Vec::new();
    for (i, chunk) in number.as_bytes().rchunks(3).enumerate() {
        let chunk = std::str::from_utf8(chunk).unwrap_or("");
        let Some(part) = speak_chunk(chunk, i) else {
            return read_digits(&number);
        };
        if !part.is_empty() {
            pieces.push(part);
        }
    }
    pieces.reverse();
    format!("{} {} {}", sign, sandhi(pieces.join(" ")), decimal)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn expand_number(s: &str) -> String {
    let re = Regex::new(r"[-+]?[0-9][0-9.,]*").expect("valid number pattern");
    let matches: Vec<_> = re.find_iter(s).collect();
    if matches.len() <= 1 {
        return plain_number(s);
    }
    let mut out = String::new();
    let mut end = 0;
    for m in matches {
        let between = &s[end..m.start()];
        let op = between
            .trim()
            .chars()
            .map(|c| match c {
                '+' => "cộng",
                '-' => "trừ",
                '*' => "nhân",
                '/' => "chia",
                '^' => "mũ",
                _ => "",
            })
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if !op.is_empty() {
            out.push(' ');
            out.push_str(&op);
        }
        out.push(' ');
        out.push_str(&plain_number(m.as_str()));
        end = m.end();
    }
    out.trim().to_owned()
}
fn month(s: &str) -> String {
    if s.trim_start_matches('0') == "4" {
        "tư".into()
    } else {
        expand_number(s)
    }
}
fn time(h: &str, m: Option<&str>, s: Option<&str>) -> String {
    let mut out = format!("{} giờ", expand_number(h));
    if let Some(m) = m
        && (s.is_some() || m.chars().any(|c| c != '0'))
    {
        out.push_str(&format!(" {} phút", expand_number(m)));
    }
    if let Some(s) = s {
        out.push_str(&format!(" {} giây", expand_number(s)));
    }
    out
}
fn abbreviation(token: &str) -> Option<String> {
    let lookup = |key: &str| {
        ABBREVIATIONS
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(abbr, _)| *abbr == key)
            .map(|(_, v)| v.split(',').next().unwrap_or("").to_owned())
    };
    lookup(token)
        .or_else(|| lookup(&token.replace(['.', '-'], "")))
        .or_else(|| {
            let parts: Vec<_> = token.split(['.', '-']).collect();
            (parts.len() > 1
                && parts
                    .iter()
                    .all(|p| p.chars().count() >= 2 && lookup(p).is_some()))
            .then(|| {
                parts
                    .iter()
                    .filter_map(|p| lookup(p))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
        })
}
fn camel(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut lower_run = 0;
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() && i > 0 {
            let run = chars[i..].iter().take_while(|x| x.is_uppercase()).count();
            if (lower_run >= 2 && run >= 2)
                || (chars[i - 1].is_uppercase()
                    && chars.get(i + 1).is_some_and(|n| n.is_lowercase()))
            {
                out.push(' ');
            }
        }
        lower_run = if c.is_lowercase() { lower_run + 1 } else { 0 };
        out.push(c);
    }
    out
}
fn boundary(kind: Form, original: &str, start: usize, end: usize, caps: &Captures<'_>) -> bool {
    let before = original[..start].chars().next_back();
    let after = original[end..].chars().next();
    let word = |c: char| c.is_alphanumeric() || c == '_';
    match kind {
        Form::TimeSeconds | Form::TimeMinutes | Form::TimeH | Form::Hour => {
            !before.is_some_and(|c| c.is_ascii_digit() || c == ':')
                && !after.is_some_and(|c| c.is_ascii_digit() || c == ':')
        }
        Form::Date => {
            group(caps, "s") == group(caps, "t")
                && !before.is_some_and(|c| c.is_ascii_digit() || "/.-".contains(c))
                && !after.is_some_and(|c| c.is_ascii_digit() || "/-".contains(c))
        }
        Form::MonthYear | Form::DayMonth => {
            !before.is_some_and(|c| c.is_ascii_digit() || "/.-".contains(c))
                && !after.is_some_and(|c| c.is_ascii_digit() || "/-".contains(c))
        }
        Form::Version | Form::BareVersion => {
            let continuation = matches!(after, Some('.' | ','))
                && original[end + 1..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit());
            !before.is_some_and(|c| word(c) || c == '.')
                && !after.is_some_and(word)
                && !continuation
        }
        Form::Fraction | Form::Degree | Form::Percent | Form::Number => {
            !before.is_some_and(|c| word(c) || "/.,".contains(c))
                && !after.is_some_and(|c| {
                    c.is_ascii_digit() || (matches!(kind, Form::Number) && ".,".contains(c))
                })
        }
        Form::Prefix => {
            after.is_some_and(|c| c.is_whitespace())
                && original[end..]
                    .trim_start()
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_uppercase())
        }
        Form::AcronymPair | Form::Code | Form::Acronym => {
            !before.is_some_and(|c| word(c) || c == '.' || c == '/') && !after.is_some_and(word)
        }
        Form::At => true,
    }
}
fn expand(kind: Form, caps: &Captures<'_>, original: &str, start: usize, _end: usize) -> String {
    let a = group(caps, "a");
    let b = group(caps, "b");
    let c = group(caps, "c");
    let n = group(caps, "n");
    match kind {
        Form::TimeSeconds => time(a, Some(b), Some(c)),
        Form::Date => format!(
            "{} tháng {} năm {}",
            expand_number(a),
            month(b),
            expand_number(c)
        ),
        Form::MonthYear => {
            let prefix = original[..start].to_lowercase();
            let lead = if prefix.trim_end().ends_with("tháng") {
                ""
            } else {
                "tháng "
            };
            format!("{}{} năm {}", lead, month(a), expand_number(b))
        }
        Form::DayMonth => format!(
            "{}{}{} tháng {}",
            group(caps, "cue"),
            group(caps, "gap"),
            expand_number(a),
            month(b)
        ),
        Form::TimeMinutes | Form::TimeH => time(a, Some(b), None),
        Form::Hour => time(a, None, None),
        Form::Version => format!(
            "{} {}",
            group(caps, "p"),
            n.split('.')
                .map(expand_number)
                .collect::<Vec<_>>()
                .join(" chấm ")
        ),
        Form::BareVersion => {
            let parts: Vec<_> = n.split('.').collect();
            if parts.len() > 1 && parts[0].len() <= 3 && parts[1..].iter().all(|p| p.len() == 3) {
                expand_number(n)
            } else {
                parts
                    .into_iter()
                    .map(expand_number)
                    .collect::<Vec<_>>()
                    .join(" chấm ")
            }
        }
        Form::Fraction => format!("{} trên {}", expand_number(a), expand_number(b)),
        Form::Degree => format!(
            "{} độ{}",
            expand_number(n),
            match group(caps, "u") {
                "C" => " xê",
                "F" => " ép",
                _ => "",
            }
        ),
        Form::Percent => format!(
            "{} phần trăm",
            expand_number(n.trim_end_matches(['.', ',']))
        ),
        Form::Number => {
            let core = n.trim_end_matches(['.', ',', ' ']);
            let suffix = &n[core.len()..];
            if core
                .trim_start_matches(['-', '+'])
                .chars()
                .all(|c| c.is_ascii_digit())
                && core.trim_start_matches(['-', '+']).len() > 8
            {
                n.into()
            } else {
                format!("{}{}", expand_number(core), suffix)
            }
        }
        Form::Prefix => abbreviation(n).unwrap_or_else(|| n.into()),
        Form::Acronym => {
            let roman = [
                ("I", "một"),
                ("II", "hai"),
                ("III", "ba"),
                ("IV", "bốn"),
                ("V", "năm"),
                ("VI", "sáu"),
                ("VII", "bảy"),
                ("VIII", "tám"),
                ("IX", "chín"),
                ("X", "mười"),
            ];
            let prior = original[..start].to_lowercase();
            let prior = prior.trim_end();
            if [
                "quý",
                "thứ",
                "khóa",
                "kỳ",
                "đợt",
                "loại",
                "chương",
                "phần",
                "thế kỷ",
            ]
            .iter()
            .any(|cue| prior.ends_with(cue))
                && let Some((_, spoken)) = roman.iter().find(|(key, _)| *key == n)
            {
                return (*spoken).into();
            }
            abbreviation(n).unwrap_or_else(|| n.into())
        }
        Form::AcronymPair => n.into(),
        Form::Code => format!(
            "{} {}",
            a,
            b.chars()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        ),
        Form::At => "a còng".into(),
    }
}
fn scan(text: &str) -> String {
    let text = camel(text);
    let mut out = String::new();
    let mut i = 0;
    while i < text.len() {
        let mut handled = false;
        for (kind, re) in patterns() {
            let Some(caps) = re.captures(&text[i..]) else {
                continue;
            };
            let m = caps.get(0).expect("full match");
            if m.start() != 0 {
                continue;
            };
            let end = i + m.end();
            if !boundary(*kind, &text, i, end, &caps) {
                continue;
            };
            let expansion = expand(*kind, &caps, &text, i, end);
            if expansion != m.as_str()
                && text[..i]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric())
            {
                out.push(' ');
            }
            out.push_str(&expansion);
            if expansion != m.as_str()
                && text[end..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphanumeric())
            {
                out.push(' ');
            }
            i = end;
            handled = true;
            break;
        }
        if !handled {
            let c = text[i..]
                .chars()
                .next()
                .expect("character at valid boundary");
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

/// Python-compatible spoken-form normalization for the local TTS runner.
pub(super) fn normalize_vi_text(input: &str) -> String {
    if input.trim().is_empty() {
        return input.into();
    }
    let input: String = input.nfc().collect();
    let mut out = String::new();
    let mut start = 0;
    for m in protected().find_iter(&input) {
        out.push_str(&scan(&input[start..m.start()]));
        out.push_str(m.as_str());
        start = m.end();
    }
    out.push_str(&scan(&input[start..]));
    Regex::new(r"[ \t]{2,}")
        .expect("valid whitespace pattern")
        .replace_all(&out, " ")
        .into_owned()
}

/// Tokenizer normalization is deliberately separate from spoken-form normalization.
pub(super) fn normalize_text(text: &str) -> String {
    let mut normalized = String::new();
    let mut in_whitespace = false;
    for character in text.nfc() {
        if character.is_whitespace() {
            in_whitespace = true;
        } else {
            if in_whitespace && !normalized.is_empty() {
                normalized.push(' ');
            }
            normalized.push(character);
            in_whitespace = false;
        }
    }
    if in_whitespace && !normalized.is_empty() {
        normalized.push(' ');
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::normalize_vi_text;

    #[test]
    fn matches_python_spoken_form_fixtures() {
        let cases = [
            (
                "Ngày 23/8/2024 lúc 15h30, giá là 1.250.000 đồng, tăng 12,5 điểm.",
                "Ngày hai mươi ba tháng tám năm hai nghìn không trăm hai mươi tư lúc mười lăm giờ ba mươi phút, giá là một triệu hai trăm năm mươi nghìn đồng, tăng mười hai phẩy năm điểm.",
            ),
            (
                "Phiên bản v1.2.3 phát hành tháng 8/2024, còn 3/4 số máy chạy 1.2.3.",
                "Phiên bản v một chấm hai chấm ba phát hành tháng tám năm hai nghìn không trăm hai mươi tư, còn ba trên bốn số máy chạy một chấm hai chấm ba.",
            ),
            (
                "UBND TP.HCM và ATM của NHNN, gửi mail tới abc@gmail.com nhé.",
                "Ủy ban Nhân dân Thành phố Hồ Chí Minh và máy rút tiền tự động của Ngân hàng Nhà nước, gửi mail tới abc@gmail.com nhé.",
            ),
            (
                "Tính 2+3, rồi 10 - 4, rồi 2^10 và 6 * 7 = 42.",
                "Tính hai cộng ba, rồi mười trừ bốn, rồi hai mũ mười và sáu nhân bảy = bốn mươi hai.",
            ),
            (
                "Hôm nay giá sản phẩm là 3.5 triệu đồng.",
                "Hôm nay giá sản phẩm là ba chấm năm triệu đồng.",
            ),
            (
                "OpenAI ChatGPT ZeroTTS TTSModel MacBook iPhone YouTube",
                "Open AI Chat GPT Zero TTS TTS Model MacBook iPhone YouTube",
            ),
            (
                "38°C, 25%, 15:30:20, quý III, mã AB-1234, USD/VND.",
                "ba mươi tám độ xê, hai mươi lăm phần trăm, mười lăm giờ ba mươi phút hai mươi giây, quý ba, mã AB 1 2 3 4, USD/VND.",
            ),
            (
                "https://example.com/v1.2 và admin@foo.vn, www.example.com",
                "https://example.com/v1.2 và admin@foo.vn, www.example.com",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(normalize_vi_text(input), expected, "input: {input}");
        }
    }
}

#[cfg(test)]
mod additional_python_cases {
    use super::normalize_vi_text;

    #[test]
    fn matches_python_numeric_and_context_cases() {
        let cases = [
            ("0", "không"),
            ("04", "bốn"),
            ("15", "mười lăm"),
            ("21", "hai mươi mốt"),
            ("24", "hai mươi tư"),
            ("104", "một trăm linh tư"),
            ("1.250.000", "một triệu hai trăm năm mươi nghìn"),
            ("12,5", "mười hai phẩy năm"),
            ("3.14", "ba chấm một bốn"),
            ("15h", "mười lăm giờ"),
            ("17:00", "mười bảy giờ"),
            ("9g45", "chín giờ bốn mươi lăm phút"),
            ("ngày 3/4", "ngày ba tháng tư"),
            ("3/4", "ba trên bốn"),
            ("8/2024", "tháng tám năm hai nghìn không trăm hai mươi tư"),
            (
                "23-8-2024",
                "hai mươi ba tháng tám năm hai nghìn không trăm hai mươi tư",
            ),
            ("v1.2", "v một chấm hai"),
            ("1.2.3", "một chấm hai chấm ba"),
            ("TP. HCM", "thành phố Hồ Chí Minh"),
            ("AB-1234", "AB 1 2 3 4"),
            ("USD/VND", "USD/VND"),
            ("25%", "hai mươi lăm phần trăm"),
            ("38°C", "ba mươi tám độ xê"),
            ("2+3", "hai cộng ba"),
            ("10 - 4", "mười trừ bốn"),
            ("2^10", "hai mũ mười"),
            ("quý III", "quý ba"),
            ("abc@gmail.com", "abc@gmail.com"),
            ("iPhone ChatGPT", "iPhone Chat GPT"),
            ("123456789", "123456789"),
            (
                "Ngày 31/12/2025 lúc 15:30.",
                "Ngày ba mươi mốt tháng mười hai năm hai nghìn không trăm hai mươi lăm lúc mười lăm giờ ba mươi phút.",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(normalize_vi_text(input), expected, "input: {input}");
        }
    }
}
