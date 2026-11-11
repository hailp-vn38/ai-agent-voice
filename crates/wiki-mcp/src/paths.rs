use crate::ToolError;
use wiki_core::{DocPath, Scope};

pub(super) fn parse_scope(raw: &str) -> Result<Scope, ToolError> {
    match raw {
        "raw" | "sources" => Ok(Scope::Raw),
        "wiki" => Ok(Scope::Wiki),
        "all" => Ok(Scope::All),
        _ => Err(ToolError::InvalidArguments(format!("invalid scope: {raw}"))),
    }
}

pub(super) fn scoped_doc_path(file: &str) -> Result<(Scope, DocPath), ToolError> {
    let clean = file.trim().trim_start_matches('/');
    let (scope, relative) = if let Some(relative) = clean.strip_prefix("sources/") {
        (Scope::Raw, relative)
    } else if let Some(relative) = clean.strip_prefix("wiki/") {
        (Scope::Wiki, relative)
    } else {
        (Scope::Wiki, clean)
    };
    Ok((scope, DocPath::parse(scope, relative)?))
}

pub(super) fn scoped_doc_path_with_scope(
    scope: Scope,
    file: &str,
) -> Result<(Scope, DocPath), ToolError> {
    let clean = file.trim().trim_start_matches('/');
    let prefixed = match scope {
        Scope::Raw if clean.starts_with("sources/") => &clean[8..],
        Scope::Wiki if clean.starts_with("wiki/") => &clean[5..],
        _ => clean,
    };
    Ok((scope, DocPath::parse(scope, prefixed)?))
}
