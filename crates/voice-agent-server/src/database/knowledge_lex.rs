//! A bounded FTS5 compiler: model input never becomes executable MATCH syntax.
//! Terms use the same Unicode letter/number separators as unicode61.
use crate::services::knowledge::Error;
use unicode_normalization::UnicodeNormalization;

pub(super) struct LexQueries {
    pub strict: String,
    pub broad: Option<String>,
}

fn safe_parts(atom: &str) -> Vec<String> {
    let normalized: String = atom.nfc().collect();
    let mut parts = Vec::new();
    let mut current = String::new();
    for ch in normalized.chars() {
        if ch.is_alphanumeric() {
            current.push(ch);
        } else if !current.is_empty() {
            parts.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// Build AND tokens with safe, quoted compound terms. If AND has no hits,
/// caller may fall back to the same tokens joined by OR for paraphrases.
pub(super) fn compile(query: &str) -> Result<LexQueries, Error> {
    if query.trim().is_empty() || query.len() > 256 {
        return Err(Error::InvalidInput);
    }
    let mut clauses = Vec::new();
    let mut buffer = String::new();
    let mut quoted = false;
    for ch in query.chars() {
        if ch == '"' {
            if !buffer.trim().is_empty() {
                push_clause(&mut clauses, &buffer, quoted)?;
            }
            buffer.clear();
            quoted = !quoted;
        } else if !quoted && ch.is_whitespace() {
            if !buffer.trim().is_empty() {
                push_clause(&mut clauses, &buffer, false)?;
                buffer.clear();
            }
        } else {
            buffer.push(ch);
        }
    }
    if quoted {
        return Err(Error::InvalidInput);
    }
    if !buffer.trim().is_empty() {
        push_clause(&mut clauses, &buffer, false)?;
    }
    if clauses.is_empty() || clauses.len() > 20 {
        return Err(Error::InvalidInput);
    }
    let broad = (clauses.len() > 1).then(|| clauses.join(" OR "));
    Ok(LexQueries {
        strict: clauses.join(" AND "),
        broad,
    })
}

fn push_clause(clauses: &mut Vec<String>, atom: &str, quoted: bool) -> Result<(), Error> {
    let parts = safe_parts(atom);
    if parts.is_empty() {
        return Ok(());
    }
    if parts.len() > 20 {
        return Err(Error::InvalidInput);
    }
    let joined = parts.join(" ");
    // No user-supplied quote, parentheses, operators, or column selectors survive.
    let phrase = format!("\"{joined}\"");
    if parts.len() == 1 && !quoted && parts[0].chars().count() >= 3 {
        clauses.push(format!("{phrase}*"));
    } else {
        clauses.push(phrase);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
