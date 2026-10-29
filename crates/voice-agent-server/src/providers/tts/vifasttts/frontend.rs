//! Pure-Rust Vietnamese text normalizer, phrase chunker, and 7-ID syllable parser
//! for `dvchd/vifasttts-ngochuyen-small` (`v2_slot5`, `onset_has_q = true`).

use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;

pub const MAX_CHUNK_CHARS: usize = 300;
pub const NUM_FEATURES: usize = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParsedSyllable {
    pub onset: &'static str,
    pub medial: &'static str,
    pub nucleus: &'static str,
    pub coda: &'static str,
    pub tone: &'static str,
}

const ONSET_SPELLINGS: &[&str] = &[
    "ngh", "ng", "nh", "ch", "gh", "gi", "kh", "ph", "th", "tr", "b", "c", "d", "đ", "g", "h", "k",
    "l", "m", "n", "p", "r", "s", "t", "v", "x",
];

fn onset_abstract(spelling: &str) -> Option<&'static str> {
    match spelling {
        "" => Some("NONE"),
        "b" => Some("B"),
        "m" => Some("M"),
        "ph" => Some("PH"),
        "v" => Some("V"),
        "t" => Some("T"),
        "th" => Some("TH"),
        "đ" => Some("Đ"),
        "n" => Some("N"),
        "d" => Some("D"),
        "gi" => Some("GI"),
        "r" => Some("R"),
        "x" => Some("X"),
        "s" => Some("S"),
        "ch" => Some("CH"),
        "tr" => Some("TR"),
        "nh" => Some("NH"),
        "l" => Some("L"),
        "c" | "k" => Some("K"),
        "kh" => Some("KH"),
        "ng" | "ngh" => Some("NG"),
        "g" | "gh" => Some("G"),
        "h" => Some("H"),
        "p" => Some("P"),
        "q" | "qu" => Some("Q"),
        _ => None,
    }
}

fn nucleus_abstract(spelling: &str) -> Option<&'static str> {
    match spelling {
        "a" => Some("A"),
        "ă" => Some("Ă"),
        "â" => Some("Â"),
        "e" => Some("E"),
        "ê" => Some("Ê"),
        "i" | "y" => Some("I"),
        "o" => Some("O"),
        "ô" => Some("Ô"),
        "ơ" => Some("Ơ"),
        "u" => Some("U"),
        "ư" => Some("Ư"),
        "iê" | "yê" | "ia" | "ya" => Some("IÊ"),
        "uô" | "ua" => Some("UÔ"),
        "ươ" | "ưa" => Some("ƯƠ"),
        "oo" => Some("OO"),
        _ => None,
    }
}

fn special_vowel(spelling: &str) -> Option<(&'static str, &'static str)> {
    match spelling {
        "oa" => Some(("o", "a")),
        "oă" => Some(("o", "ă")),
        "oe" => Some(("o", "e")),
        "uâ" => Some(("u", "â")),
        "uê" => Some(("u", "ê")),
        "uy" => Some(("u", "y")),
        "uya" => Some(("u", "ya")),
        "uyê" => Some(("u", "yê")),
        "uơ" => Some(("u", "ơ")),
        _ => None,
    }
}

const CONSONANT_CODAS: &[(&str, &str)] = &[
    ("nh", "NH"),
    ("ng", "NG"),
    ("ch", "CH"),
    ("m", "M"),
    ("n", "N"),
    ("p", "P"),
    ("t", "T"),
    ("c", "C"),
];

fn onset_id(onset: &str) -> i64 {
    match onset {
        "NONE" => 0,
        "B" => 1,
        "M" => 2,
        "PH" => 3,
        "V" => 4,
        "T" => 5,
        "TH" => 6,
        "Đ" => 7,
        "N" => 8,
        "D" => 9,
        "GI" => 10,
        "R" => 11,
        "X" => 12,
        "S" => 13,
        "CH" => 14,
        "TR" => 15,
        "NH" => 16,
        "L" => 17,
        "K" => 18,
        "KH" => 19,
        "NG" => 20,
        "G" => 21,
        "H" => 22,
        "P" => 23,
        "Q" => 24,
        _ => 0,
    }
}

fn medial_id(medial: &str) -> i64 {
    match medial {
        "W" => 1,
        _ => 0,
    }
}

fn nucleus_id(nucleus: &str) -> i64 {
    match nucleus {
        "A" => 0,
        "Ă" => 1,
        "Â" => 2,
        "E" => 3,
        "Ê" => 4,
        "I" => 5,
        "O" => 6,
        "Ô" => 7,
        "Ơ" => 8,
        "U" => 9,
        "Ư" => 10,
        "IÊ" => 11,
        "UÔ" => 12,
        "ƯƠ" => 13,
        "OO" => 14,
        _ => 0,
    }
}

fn coda_id(coda: &str) -> i64 {
    match coda {
        "NONE" => 0,
        "M" => 1,
        "N" => 2,
        "NG" => 3,
        "NH" => 4,
        "P" => 5,
        "T" => 6,
        "C" => 7,
        "CH" => 8,
        "J" => 9,
        "W" => 10,
        _ => 0,
    }
}

fn tone_id(tone: &str) -> i64 {
    match tone {
        "NGANG" => 0,
        "HUYEN" => 1,
        "SAC" => 2,
        "HOI" => 3,
        "NGA" => 4,
        "NANG" => 5,
        _ => 0,
    }
}

fn boundary_from_punct(trail: &str) -> i64 {
    if trail.chars().any(|c| matches!(c, '.' | '!' | '?' | '…')) {
        3 // SENTENCE
    } else if trail.chars().any(|c| matches!(c, ',' | ';' | ':')) {
        2 // PHRASE
    } else {
        1 // WORD
    }
}

fn punct_id(trail: &str) -> i64 {
    if trail.is_empty() {
        0 // NONE
    } else if trail.contains('?') {
        3 // QUESTION
    } else if trail.contains('!') {
        4 // EXCLAMATION
    } else if trail.contains(',') {
        1 // COMMA
    } else if trail.contains('.') || trail.contains('…') {
        2 // PERIOD
    } else {
        5 // OTHER
    }
}

fn extract_tone(word: &str) -> (String, &'static str) {
    let mut tone = "NGANG";
    let mut base = String::with_capacity(word.len());
    for ch in word.nfd() {
        match ch {
            '\u{0301}' => tone = "SAC",
            '\u{0300}' => tone = "HUYEN",
            '\u{0309}' => tone = "HOI",
            '\u{0303}' => tone = "NGA",
            '\u{0323}' => tone = "NANG",
            _ => base.push(ch),
        }
    }
    (base.nfc().collect(), tone)
}

fn is_vietnamese_vowel_char(ch: char) -> bool {
    matches!(
        ch,
        'a' | 'ă' | 'â' | 'e' | 'ê' | 'i' | 'o' | 'ô' | 'ơ' | 'u' | 'ư' | 'y'
    )
}

fn parse_vowel(
    vowel_part: &str,
    qu_medial: bool,
) -> Option<(&'static str, &str, &'static str)> {
    if qu_medial {
        if vowel_part == "yê" {
            return Some(("W", "yê", "IÊ"));
        }
        if vowel_part == "ya" {
            return Some(("W", "ya", "IÊ"));
        }
        if let Some(nuc) = nucleus_abstract(vowel_part) {
            return Some(("W", vowel_part, nuc));
        }
        return None;
    }

    if let Some((_med_s, nuc_s)) = special_vowel(vowel_part) {
        let nuc_a = match nuc_s {
            "ya" | "yê" => "IÊ",
            _ => nucleus_abstract(nuc_s)?,
        };
        return Some(("W", nuc_s, nuc_a));
    }

    if let Some(nuc) = nucleus_abstract(vowel_part) {
        return Some(("NONE", vowel_part, nuc));
    }

    None
}

fn try_semivowel_coda(remainder: &str) -> Option<(&str, &str, &'static str)> {
    let mut chars = remainder.char_indices();
    let (last_idx, last_char) = chars.next_back()?;
    if last_idx == 0 {
        return None;
    }
    let base = &remainder[..last_idx];
    let last_str = &remainder[last_idx..];
    let valid_base = nucleus_abstract(base).is_some() || special_vowel(base).is_some();
    if !valid_base {
        return None;
    }
    if matches!(last_char, 'i' | 'y') {
        if matches!(base, "i" | "y" | "ê" | "iê" | "yê") {
            return None;
        }
        return Some((base, last_str, "J"));
    }
    if matches!(last_char, 'u' | 'o') {
        if matches!(base, "u" | "o" | "ô" | "uô") {
            return None;
        }
        return Some((base, last_str, "W"));
    }
    None
}

pub fn parse_syllable(word: &str) -> Option<ParsedSyllable> {
    let normalized: String = word.trim().to_lowercase().nfc().collect();
    if normalized.is_empty() {
        return None;
    }
    let (toneless, tone) = extract_tone(&normalized);
    let chars: Vec<char> = toneless.chars().collect();
    if chars.is_empty() {
        return None;
    }

    let mut onset_spelling = "";
    let mut remainder = toneless.as_str();
    let mut qu_medial = false;

    if toneless.starts_with("gi")
        && (chars.len() == 2 || chars[2] == 'ê' || !is_vietnamese_vowel_char(chars[2]))
    {
        onset_spelling = "g";
        let idx = toneless.char_indices().nth(1)?.0;
        remainder = &toneless[idx..];
    } else if toneless.starts_with("qu") {
        if chars.len() >= 3 && chars[2] == 'ô' {
            onset_spelling = "q";
            let idx = toneless.char_indices().nth(1)?.0;
            remainder = &toneless[idx..];
            qu_medial = false;
        } else {
            onset_spelling = "qu";
            let idx = toneless
                .char_indices()
                .nth(2)
                .map(|(i, _)| i)
                .unwrap_or(toneless.len());
            remainder = &toneless[idx..];
            qu_medial = true;
        }
    } else {
        for &cand in ONSET_SPELLINGS {
            if let Some(rest) = toneless.strip_prefix(cand) {
                onset_spelling = cand;
                remainder = rest;
                break;
            }
        }
    }

    if remainder.is_empty() {
        return None;
    }

    let mut coda_spelling = "";
    let mut coda_abstract = "NONE";
    let mut vowel_part = remainder;

    let mut matched_consonant_coda = false;
    for &(coda_s, coda_a) in CONSONANT_CODAS {
        if remainder.len() > coda_s.len()
            && let Some(stem) = remainder.strip_suffix(coda_s)
        {
            coda_spelling = coda_s;
            coda_abstract = coda_a;
            vowel_part = stem;
            matched_consonant_coda = true;
            break;
        }
    }

    let (medial, nucleus_s, mut nucleus_a) = if !matched_consonant_coda {
        if let Some(parsed) = parse_vowel(remainder, qu_medial) {
            parsed
        } else if let Some((vpart, c_spell, c_abs)) = try_semivowel_coda(remainder) {
            coda_spelling = c_spell;
            coda_abstract = c_abs;
            parse_vowel(vpart, qu_medial)?
        } else {
            return None;
        }
    } else {
        parse_vowel(vowel_part, qu_medial)?
    };

    if matches!(coda_spelling, "y" | "u") && nucleus_s == "a" {
        nucleus_a = "Ă";
    }

    if medial == "W" && !matches!(nucleus_a, "A" | "Ă" | "Â" | "E" | "Ê" | "I" | "Ơ" | "IÊ") {
        return None;
    }

    if matches!(coda_abstract, "P" | "T" | "C" | "CH") && !matches!(tone, "SAC" | "NANG") {
        return None;
    }

    let onset_a = if onset_spelling == "g" && toneless.starts_with("gi") {
        "GI"
    } else {
        onset_abstract(onset_spelling)?
    };

    Some(ParsedSyllable {
        onset: onset_a,
        medial,
        nucleus: nucleus_a,
        coda: coda_abstract,
        tone,
    })
}

fn letter_name(ch: char) -> Option<&'static str> {
    match ch {
        'a' => Some("a"),
        'b' => Some("bê"),
        'c' => Some("xê"),
        'd' => Some("dê"),
        'đ' => Some("đê"),
        'e' => Some("e"),
        'f' => Some("ép"),
        'g' => Some("giê"),
        'h' => Some("hát"),
        'i' => Some("i"),
        'j' => Some("di"),
        'k' => Some("ca"),
        'l' => Some("lờ"),
        'm' => Some("mờ"),
        'n' => Some("nờ"),
        'o' => Some("o"),
        'p' => Some("pê"),
        'q' => Some("quy"),
        'r' => Some("rờ"),
        's' => Some("ét"),
        't' => Some("tê"),
        'u' => Some("u"),
        'v' => Some("vê"),
        'w' => Some("vê kép"),
        'x' => Some("ích"),
        'y' => Some("i"),
        'z' => Some("dét"),
        _ => None,
    }
}

fn loanword(word: &str) -> Option<&'static [&'static str]> {
    match word {
        "ok" | "okay" => Some(&["ô", "kê"]),
        "wifi" => Some(&["oai", "phai"]),
        "video" => Some(&["vi", "đê", "ô"]),
        "camera" => Some(&["ca", "mê", "ra"]),
        "menu" => Some(&["mê", "nu"]),
        "auto" => Some(&["ô", "tô"]),
        "server" => Some(&["sơ", "vơ"]),
        "model" => Some(&["mô", "đơn"]),
        "token" => Some(&["tô", "kần"]),
        "agent" => Some(&["ây", "giần"]),
        "voice" => Some(&["voi"]),
        "online" => Some(&["on", "lai"]),
        "offline" => Some(&["óp", "lai"]),
        "email" => Some(&["i", "meo"]),
        "app" => Some(&["áp"]),
        "web" => Some(&["oép"]),
        "bot" => Some(&["bót"]),
        "chat" => Some(&["chát"]),
        "file" => Some(&["phai"]),
        "code" => Some(&["cốt"]),
        "test" => Some(&["tét"]),
        "api" => Some(&["ây", "pi", "ai"]),
        "tts" => Some(&["tê", "tê", "ét"]),
        "asr" => Some(&["a", "ét", "rờ"]),
        "llm" => Some(&["lờ", "lờ", "mờ"]),
        "vad" => Some(&["vê", "a", "dê"]),
        "mcp" => Some(&["mờ", "xê", "pê"]),
        "cpu" => Some(&["xê", "pê", "u"]),
        "gpu" => Some(&["giê", "pê", "u"]),
        "ram" => Some(&["ram"]),
        "usb" => Some(&["u", "ét", "bê"]),
        _ => None,
    }
}

fn resolve_oov(word: &str) -> Option<Vec<&'static str>> {
    if let Some(mapped) = loanword(word) {
        return Some(mapped.to_vec());
    }
    if word.len() <= 5 && word.chars().all(|c| c.is_ascii_alphabetic()) {
        let mut out = Vec::with_capacity(word.len());
        for ch in word.chars() {
            let spelled = letter_name(ch.to_ascii_lowercase())?;
            out.extend(spelled.split_whitespace());
        }
        if !out.is_empty() {
            return Some(out);
        }
    }
    None
}

fn syllable_to_ids(syl: &ParsedSyllable, boundary: i64, punct: i64) -> [i64; NUM_FEATURES] {
    [
        onset_id(syl.onset),
        medial_id(syl.medial),
        nucleus_id(syl.nucleus),
        coda_id(syl.coda),
        tone_id(syl.tone),
        boundary,
        punct,
    ]
}

const DIGITS: [&str; 10] = [
    "không", "một", "hai", "ba", "bốn", "năm", "sáu", "bảy", "tám", "chín",
];

fn three_digits_to_words(group: u16, has_higher: bool) -> Vec<&'static str> {
    let h = (group / 100) as usize;
    let t = ((group / 10) % 10) as usize;
    let u = (group % 10) as usize;
    let mut words = Vec::with_capacity(5);

    if h > 0 || has_higher {
        words.push(DIGITS[h]);
        words.push("trăm");
    }

    if t == 0 {
        if u > 0 {
            if h > 0 || has_higher {
                words.push("lẻ");
            }
            words.push(DIGITS[u]);
        }
    } else if t == 1 {
        words.push("mười");
        if u == 5 {
            words.push("lăm");
        } else if u > 0 {
            words.push(DIGITS[u]);
        }
    } else {
        words.push(DIGITS[t]);
        words.push("mươi");
        match u {
            0 => {}
            1 => words.push("mốt"),
            4 => words.push("tư"),
            5 => words.push("lăm"),
            _ => words.push(DIGITS[u]),
        }
    }

    words
}

pub fn number_to_vietnamese_words(raw: &str) -> String {
    let trimmed = raw.trim_start_matches('0');
    if trimmed.is_empty() {
        return "không".into();
    }
    let Ok(n) = trimmed.parse::<u64>() else {
        return raw
            .chars()
            .filter_map(|c| c.to_digit(10).map(|d| DIGITS[d as usize]))
            .collect::<Vec<_>>()
            .join(" ");
    };
    if n == 0 {
        return "không".into();
    }
    if n >= 1_000_000_000_000 {
        return raw
            .chars()
            .filter_map(|c| c.to_digit(10).map(|d| DIGITS[d as usize]))
            .collect::<Vec<_>>()
            .join(" ");
    }

    let scales = [
        (1_000_000_000_u64, "tỷ"),
        (1_000_000_u64, "triệu"),
        (1_000_u64, "nghìn"),
        (1_u64, ""),
    ];
    let mut rem = n;
    let mut has_higher = false;
    let mut out = Vec::new();
    for (divisor, unit) in scales {
        let group = (rem / divisor) as u16;
        rem %= divisor;
        if group > 0 {
            out.extend(three_digits_to_words(group, has_higher));
            if !unit.is_empty() {
                out.push(unit);
            }
            has_higher = true;
        }
    }
    out.join(" ")
}

static RE_TIME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(\d{1,2}):(\d{2})\b").expect("valid time regex"));
static RE_DATE_FULL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(\d{1,2})/(\d{1,2})/(\d{4})\b").expect("valid full date regex")
});
static RE_DATE_SHORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(\d{1,2})/(\d{1,2})\b").expect("valid short date regex"));
static RE_THOUSANDS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b\d{1,3}(?:\.\d{3})+\b").expect("valid thousands regex"));
static RE_DECIMAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(\d+),(\d+)\b").expect("valid decimal regex"));
static RE_PERCENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d+)\s*%").expect("valid percent regex"));
static RE_CURRENCY_VND: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(\d+)\s*(?:đ|₫|vnd|vnđ)\b").expect("valid vnd currency regex")
});
static RE_CURRENCY_USD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(\d+)\s*(?:\$|usd\b)").expect("valid usd currency regex"));
static RE_INTEGER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b\d+\b").expect("valid integer regex"));
static RE_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([^\s\p{L}\p{N}]*)([\p{L}\p{N}]+)([^\s\p{L}\p{N}]*)").expect("valid token regex")
});
static RE_SENTENCE_SPLIT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([.!?…]+\s+)").expect("valid sentence split regex"));
static RE_CLAUSE_SPLIT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([,;:]+\s+)").expect("valid clause split regex"));

pub fn normalize_text(input: &str) -> String {
    let mut text: String = input.nfc().collect();
    text = text
        .replace(['“', '”', '‘', '’'], "\"")
        .replace(['–', '—'], ", ")
        .replace("...", "…");

    text = RE_TIME
        .replace_all(&text, |caps: &regex::Captures<'_>| {
            let h: u32 = caps[1].parse().unwrap_or(99);
            let m: u32 = caps[2].parse().unwrap_or(99);
            if h <= 23 && m <= 59 {
                let hw = number_to_vietnamese_words(&caps[1]);
                if m == 0 {
                    format!("{hw} giờ")
                } else {
                    let mw = number_to_vietnamese_words(&caps[2]);
                    format!("{hw} giờ {mw}")
                }
            } else {
                caps[0].to_string()
            }
        })
        .into_owned();

    text = RE_DATE_FULL
        .replace_all(&text, |caps: &regex::Captures<'_>| {
            let d: u32 = caps[1].parse().unwrap_or(0);
            let m: u32 = caps[2].parse().unwrap_or(0);
            if (1..=31).contains(&d) && (1..=12).contains(&m) {
                format!(
                    "ngày {} tháng {} năm {}",
                    number_to_vietnamese_words(&caps[1]),
                    number_to_vietnamese_words(&caps[2]),
                    number_to_vietnamese_words(&caps[3]),
                )
            } else {
                caps[0].to_string()
            }
        })
        .into_owned();

    text = RE_DATE_SHORT
        .replace_all(&text, |caps: &regex::Captures<'_>| {
            let d: u32 = caps[1].parse().unwrap_or(0);
            let m: u32 = caps[2].parse().unwrap_or(0);
            if (1..=31).contains(&d) && (1..=12).contains(&m) {
                format!(
                    "ngày {} tháng {}",
                    number_to_vietnamese_words(&caps[1]),
                    number_to_vietnamese_words(&caps[2]),
                )
            } else {
                caps[0].to_string()
            }
        })
        .into_owned();

    text = RE_THOUSANDS
        .replace_all(&text, |caps: &regex::Captures<'_>| {
            let digits = caps[0].replace('.', "");
            number_to_vietnamese_words(&digits)
        })
        .into_owned();

    text = RE_DECIMAL
        .replace_all(&text, |caps: &regex::Captures<'_>| {
            format!(
                "{} phẩy {}",
                number_to_vietnamese_words(&caps[1]),
                number_to_vietnamese_words(&caps[2]),
            )
        })
        .into_owned();

    text = RE_PERCENT
        .replace_all(&text, |caps: &regex::Captures<'_>| {
            format!("{} phần trăm", number_to_vietnamese_words(&caps[1]))
        })
        .into_owned();

    text = RE_CURRENCY_VND
        .replace_all(&text, |caps: &regex::Captures<'_>| {
            format!("{} đồng", number_to_vietnamese_words(&caps[1]))
        })
        .into_owned();

    text = RE_CURRENCY_USD
        .replace_all(&text, |caps: &regex::Captures<'_>| {
            format!("{} đô la", number_to_vietnamese_words(&caps[1]))
        })
        .into_owned();

    text = RE_INTEGER
        .replace_all(&text, |caps: &regex::Captures<'_>| {
            number_to_vietnamese_words(&caps[0])
        })
        .into_owned();

    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn split_keeping_delimiter(text: &str, regex: &Regex) -> Vec<String> {
    let mut out = Vec::new();
    let mut last = 0;
    for mat in regex.find_iter(text) {
        let piece = text[last..mat.end()].trim();
        if !piece.is_empty() {
            out.push(piece.to_string());
        }
        last = mat.end();
    }
    let tail = text[last..].trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}

pub fn chunk_text(text: &str, max_chars: usize) -> Vec<String> {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return Vec::new();
    }
    if collapsed.chars().count() <= max_chars {
        return vec![collapsed];
    }

    let sentences = split_keeping_delimiter(&collapsed, &RE_SENTENCE_SPLIT);
    let mut pieces = Vec::new();
    for sentence in sentences {
        if sentence.chars().count() <= max_chars {
            pieces.push(sentence);
            continue;
        }
        let clauses = split_keeping_delimiter(&sentence, &RE_CLAUSE_SPLIT);
        for clause in clauses {
            if clause.chars().count() <= max_chars {
                pieces.push(clause);
                continue;
            }
            let mut buf = String::new();
            for word in clause.split_whitespace() {
                let candidate_len = if buf.is_empty() {
                    word.chars().count()
                } else {
                    buf.chars().count() + 1 + word.chars().count()
                };
                if !buf.is_empty() && candidate_len > max_chars {
                    pieces.push(buf);
                    buf = word.to_string();
                } else {
                    if !buf.is_empty() {
                        buf.push(' ');
                    }
                    buf.push_str(word);
                }
            }
            if !buf.is_empty() {
                pieces.push(buf);
            }
        }
    }

    let mut chunks = Vec::new();
    let mut buf = String::new();
    for piece in pieces {
        let candidate_len = if buf.is_empty() {
            piece.chars().count()
        } else {
            buf.chars().count() + 1 + piece.chars().count()
        };
        if !buf.is_empty() && candidate_len > max_chars {
            chunks.push(buf);
            buf = piece;
        } else {
            if !buf.is_empty() {
                buf.push(' ');
            }
            buf.push_str(&piece);
        }
    }
    if !buf.is_empty() {
        chunks.push(buf);
    }
    chunks
}

pub fn pause_seconds(chunk: &str) -> f32 {
    let trimmed = chunk.trim_end();
    if trimmed.ends_with(['.', '!', '?', '…']) {
        0.15
    } else if trimmed.ends_with([',', ';', ':']) {
        0.08
    } else {
        0.05
    }
}

pub fn features_from_text(text: &str) -> Vec<[i64; NUM_FEATURES]> {
    let normalized = normalize_text(text);
    let mut rows = Vec::new();

    for caps in RE_TOKEN.captures_iter(&normalized) {
        let raw_word = &caps[2];
        let trail = &caps[3];
        let word: String = raw_word.to_lowercase().nfc().collect();
        let boundary = boundary_from_punct(trail);
        let punct = punct_id(trail);

        if let Some(syl) = parse_syllable(&word) {
            rows.push(syllable_to_ids(&syl, boundary, punct));
        } else if let Some(sub_words) = resolve_oov(&word) {
            let last_idx = sub_words.len().saturating_sub(1);
            for (idx, sw) in sub_words.into_iter().enumerate() {
                let is_last = idx == last_idx;
                let b = if is_last { boundary } else { 1 };
                let p = if is_last { punct } else { 0 };
                if let Some(sub_syl) = parse_syllable(sw) {
                    rows.push(syllable_to_ids(&sub_syl, b, p));
                } else {
                    rows.push([0, 0, 0, 0, 0, b, p]);
                }
            }
        } else {
            rows.push([0, 0, 0, 0, 0, boundary, punct]);
        }
    }

    rows
}
