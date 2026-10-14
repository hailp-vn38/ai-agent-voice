const NAMED_ENTITIES: &[(&str, char)] = &[
    ("quot", '\u{0022}'),
    ("amp", '\u{0026}'),
    ("apos", '\u{0027}'),
    ("lt", '\u{003C}'),
    ("gt", '\u{003E}'),
    ("Tab", '\u{0009}'),
    ("NewLine", '\u{000A}'),
    ("nbsp", '\u{00A0}'),
    ("iexcl", '\u{00A1}'),
    ("cent", '\u{00A2}'),
    ("pound", '\u{00A3}'),
    ("curren", '\u{00A4}'),
    ("yen", '\u{00A5}'),
    ("brvbar", '\u{00A6}'),
    ("sect", '\u{00A7}'),
    ("uml", '\u{00A8}'),
    ("copy", '\u{00A9}'),
    ("ordf", '\u{00AA}'),
    ("laquo", '\u{00AB}'),
    ("not", '\u{00AC}'),
    ("shy", '\u{00AD}'),
    ("reg", '\u{00AE}'),
    ("macr", '\u{00AF}'),
    ("deg", '\u{00B0}'),
    ("plusmn", '\u{00B1}'),
    ("sup2", '\u{00B2}'),
    ("sup3", '\u{00B3}'),
    ("acute", '\u{00B4}'),
    ("micro", '\u{00B5}'),
    ("para", '\u{00B6}'),
    ("middot", '\u{00B7}'),
    ("cedil", '\u{00B8}'),
    ("sup1", '\u{00B9}'),
    ("ordm", '\u{00BA}'),
    ("raquo", '\u{00BB}'),
    ("frac14", '\u{00BC}'),
    ("frac12", '\u{00BD}'),
    ("frac34", '\u{00BE}'),
    ("iquest", '\u{00BF}'),
    ("Agrave", '\u{00C0}'),
    ("Aacute", '\u{00C1}'),
    ("Acirc", '\u{00C2}'),
    ("Atilde", '\u{00C3}'),
    ("Auml", '\u{00C4}'),
    ("Aring", '\u{00C5}'),
    ("AElig", '\u{00C6}'),
    ("Ccedil", '\u{00C7}'),
    ("Egrave", '\u{00C8}'),
    ("Eacute", '\u{00C9}'),
    ("Ecirc", '\u{00CA}'),
    ("Euml", '\u{00CB}'),
    ("Igrave", '\u{00CC}'),
    ("Iacute", '\u{00CD}'),
    ("Icirc", '\u{00CE}'),
    ("Iuml", '\u{00CF}'),
    ("ETH", '\u{00D0}'),
    ("Ntilde", '\u{00D1}'),
    ("Ograve", '\u{00D2}'),
    ("Oacute", '\u{00D3}'),
    ("Ocirc", '\u{00D4}'),
    ("Otilde", '\u{00D5}'),
    ("Ouml", '\u{00D6}'),
    ("times", '\u{00D7}'),
    ("Oslash", '\u{00D8}'),
    ("Ugrave", '\u{00D9}'),
    ("Uacute", '\u{00DA}'),
    ("Ucirc", '\u{00DB}'),
    ("Uuml", '\u{00DC}'),
    ("Yacute", '\u{00DD}'),
    ("THORN", '\u{00DE}'),
    ("szlig", '\u{00DF}'),
    ("agrave", '\u{00E0}'),
    ("aacute", '\u{00E1}'),
    ("acirc", '\u{00E2}'),
    ("atilde", '\u{00E3}'),
    ("auml", '\u{00E4}'),
    ("aring", '\u{00E5}'),
    ("aelig", '\u{00E6}'),
    ("ccedil", '\u{00E7}'),
    ("egrave", '\u{00E8}'),
    ("eacute", '\u{00E9}'),
    ("ecirc", '\u{00EA}'),
    ("euml", '\u{00EB}'),
    ("igrave", '\u{00EC}'),
    ("iacute", '\u{00ED}'),
    ("icirc", '\u{00EE}'),
    ("iuml", '\u{00EF}'),
    ("eth", '\u{00F0}'),
    ("ntilde", '\u{00F1}'),
    ("ograve", '\u{00F2}'),
    ("oacute", '\u{00F3}'),
    ("ocirc", '\u{00F4}'),
    ("otilde", '\u{00F5}'),
    ("ouml", '\u{00F6}'),
    ("divide", '\u{00F7}'),
    ("oslash", '\u{00F8}'),
    ("ugrave", '\u{00F9}'),
    ("uacute", '\u{00FA}'),
    ("ucirc", '\u{00FB}'),
    ("uuml", '\u{00FC}'),
    ("yacute", '\u{00FD}'),
    ("thorn", '\u{00FE}'),
    ("yuml", '\u{00FF}'),
    ("OElig", '\u{0152}'),
    ("oelig", '\u{0153}'),
    ("Scaron", '\u{0160}'),
    ("scaron", '\u{0161}'),
    ("Yuml", '\u{0178}'),
    ("fnof", '\u{0192}'),
    ("circ", '\u{02C6}'),
    ("tilde", '\u{02DC}'),
    ("Alpha", '\u{0391}'),
    ("Beta", '\u{0392}'),
    ("Gamma", '\u{0393}'),
    ("Delta", '\u{0394}'),
    ("Epsilon", '\u{0395}'),
    ("Zeta", '\u{0396}'),
    ("Eta", '\u{0397}'),
    ("Theta", '\u{0398}'),
    ("Iota", '\u{0399}'),
    ("Kappa", '\u{039A}'),
    ("Lambda", '\u{039B}'),
    ("Mu", '\u{039C}'),
    ("Nu", '\u{039D}'),
    ("Xi", '\u{039E}'),
    ("Omicron", '\u{039F}'),
    ("Pi", '\u{03A0}'),
    ("Rho", '\u{03A1}'),
    ("Sigma", '\u{03A3}'),
    ("Tau", '\u{03A4}'),
    ("Upsilon", '\u{03A5}'),
    ("Phi", '\u{03A6}'),
    ("Chi", '\u{03A7}'),
    ("Psi", '\u{03A8}'),
    ("Omega", '\u{03A9}'),
    ("alpha", '\u{03B1}'),
    ("beta", '\u{03B2}'),
    ("gamma", '\u{03B3}'),
    ("delta", '\u{03B4}'),
    ("epsilon", '\u{03B5}'),
    ("zeta", '\u{03B6}'),
    ("eta", '\u{03B7}'),
    ("theta", '\u{03B8}'),
    ("iota", '\u{03B9}'),
    ("kappa", '\u{03BA}'),
    ("lambda", '\u{03BB}'),
    ("mu", '\u{03BC}'),
    ("nu", '\u{03BD}'),
    ("xi", '\u{03BE}'),
    ("omicron", '\u{03BF}'),
    ("pi", '\u{03C0}'),
    ("rho", '\u{03C1}'),
    ("sigmaf", '\u{03C2}'),
    ("sigma", '\u{03C3}'),
    ("tau", '\u{03C4}'),
    ("upsilon", '\u{03C5}'),
    ("phi", '\u{03C6}'),
    ("chi", '\u{03C7}'),
    ("psi", '\u{03C8}'),
    ("omega", '\u{03C9}'),
    ("thetasym", '\u{03D1}'),
    ("upsih", '\u{03D2}'),
    ("piv", '\u{03D6}'),
    ("ensp", '\u{2002}'),
    ("emsp", '\u{2003}'),
    ("thinsp", '\u{2009}'),
    ("zwnj", '\u{200C}'),
    ("zwj", '\u{200D}'),
    ("lrm", '\u{200E}'),
    ("rlm", '\u{200F}'),
    ("ndash", '\u{2013}'),
    ("mdash", '\u{2014}'),
    ("lsquo", '\u{2018}'),
    ("rsquo", '\u{2019}'),
    ("sbquo", '\u{201A}'),
    ("ldquo", '\u{201C}'),
    ("rdquo", '\u{201D}'),
    ("bdquo", '\u{201E}'),
    ("dagger", '\u{2020}'),
    ("Dagger", '\u{2021}'),
    ("bull", '\u{2022}'),
    ("hellip", '\u{2026}'),
    ("permil", '\u{2030}'),
    ("prime", '\u{2032}'),
    ("Prime", '\u{2033}'),
    ("lsaquo", '\u{2039}'),
    ("rsaquo", '\u{203A}'),
    ("oline", '\u{203E}'),
    ("frasl", '\u{2044}'),
    ("euro", '\u{20AC}'),
    ("weierp", '\u{2118}'),
    ("image", '\u{2111}'),
    ("real", '\u{211C}'),
    ("trade", '\u{2122}'),
    ("alefsym", '\u{2135}'),
    ("larr", '\u{2190}'),
    ("uarr", '\u{2191}'),
    ("rarr", '\u{2192}'),
    ("darr", '\u{2193}'),
    ("harr", '\u{2194}'),
    ("crarr", '\u{21B5}'),
    ("forall", '\u{2200}'),
    ("part", '\u{2202}'),
    ("exist", '\u{2203}'),
    ("empty", '\u{2205}'),
    ("nabla", '\u{2207}'),
    ("isin", '\u{2208}'),
    ("notin", '\u{2209}'),
    ("ni", '\u{220B}'),
    ("prod", '\u{220F}'),
    ("sum", '\u{2211}'),
    ("minus", '\u{2212}'),
    ("lowast", '\u{2217}'),
    ("radic", '\u{221A}'),
    ("prop", '\u{221D}'),
    ("infin", '\u{221E}'),
    ("ang", '\u{2220}'),
    ("and", '\u{2227}'),
    ("or", '\u{2228}'),
    ("cap", '\u{2229}'),
    ("cup", '\u{222A}'),
    ("int", '\u{222B}'),
    ("there4", '\u{2234}'),
    ("sim", '\u{223C}'),
    ("cong", '\u{2245}'),
    ("asymp", '\u{2248}'),
    ("ne", '\u{2260}'),
    ("equiv", '\u{2261}'),
    ("le", '\u{2264}'),
    ("ge", '\u{2265}'),
    ("sub", '\u{2282}'),
    ("sup", '\u{2283}'),
    ("nsub", '\u{2284}'),
    ("sube", '\u{2286}'),
    ("supe", '\u{2287}'),
    ("oplus", '\u{2295}'),
    ("otimes", '\u{2297}'),
    ("perp", '\u{22A5}'),
    ("sdot", '\u{22C5}'),
    ("lceil", '\u{2308}'),
    ("rceil", '\u{2309}'),
    ("lfloor", '\u{230A}'),
    ("rfloor", '\u{230B}'),
    ("lang", '\u{27E8}'),
    ("rang", '\u{27E9}'),
    ("loz", '\u{25CA}'),
    ("spades", '\u{2660}'),
    ("clubs", '\u{2663}'),
    ("hearts", '\u{2665}'),
    ("diams", '\u{2666}'),
];

fn lookup_named_entity(name: &str) -> Option<char> {
    NAMED_ENTITIES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, c)| *c)
}

/// Decode một entity không gồm `&` và `;`, ví dụ `amp`, `#38`, `#x27`.
fn decode_entity(entity: &str) -> Option<char> {
    if entity.is_empty() {
        return None;
    }
    if let Some(num) = entity.strip_prefix('#') {
        if num.is_empty() {
            return None;
        }
        if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
            if hex.is_empty() || hex.len() > 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let code = u32::from_str_radix(hex, 16).ok()?;
            if code == 0 {
                return Some('\u{FFFD}');
            }
            return char::from_u32(code);
        }
        if num.len() > 7 || !num.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let code: u32 = num.parse().ok()?;
        if code == 0 {
            return Some('\u{FFFD}');
        }
        return char::from_u32(code);
    }
    lookup_named_entity(entity)
}

/// Decode entities trong một lần duyệt duy nhất, nên `&amp;lt;` -> `&lt;`
/// (không decode kép thành `<`). Entity không nhận ra được giữ nguyên.
pub(super) fn decode_html_entities(value: &str) -> String {
    if !value.contains('&') {
        return value.to_owned();
    }
    let mut out = String::with_capacity(value.len());
    let mut i = 0;
    while i < value.len() {
        let rest = &value[i..];
        if !rest.starts_with('&') {
            let next = rest.find('&').unwrap_or(rest.len());
            out.push_str(&rest[..next]);
            i += next;
            continue;
        }
        // rest bắt đầu bằng '&': thử dạng `&...;`
        if let Some(semi) = rest.bytes().take(33).position(|byte| byte == b';') {
            if (2..=32).contains(&semi) {
                let entity = &rest[1..semi];
                if let Some(ch) = decode_entity(entity) {
                    out.push(ch);
                    i += semi + 1;
                    continue;
                }
            }
        }
        // Dạng không có `;` (HTML cho phép vài cái). Chỉ decode khi ký tự
        // sau tên không phải alphanumeric để tránh `&amazing` -> nhầm.
        let mut matched = false;
        for name in ["amp", "lt", "gt", "quot", "apos", "nbsp", "copy", "reg"] {
            if rest.len() > name.len() + 1
                && rest[1..].starts_with(name)
                && !rest.as_bytes()[1 + name.len()].is_ascii_alphanumeric()
            {
                if let Some(ch) = lookup_named_entity(name) {
                    out.push(ch);
                    i += 1 + name.len();
                    matched = true;
                    break;
                }
            }
        }
        if !matched {
            out.push('&');
            i += 1;
        }
    }
    out
}
