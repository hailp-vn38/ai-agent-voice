const SUPPORTED_EXTENSIONS: &[&str] = &[
    "md", "txt", "pdf", "csv", "json", "html", "docx", "xlsx", "png", "jpg", "webp", "mp4",
];

pub(crate) fn supported_agent_task_file(path: &str) -> bool {
    path.rsplit('.').next().is_some_and(|extension| {
        SUPPORTED_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
    })
}

#[cfg(test)]
mod tests {
    use super::supported_agent_task_file;

    #[test]
    fn supported_agent_task_extensions_are_case_insensitive_and_shared() {
        for extension in [
            "md", "txt", "pdf", "csv", "json", "html", "docx", "xlsx", "png", "jpg", "webp", "mp4",
        ] {
            assert!(supported_agent_task_file(&format!("report.{extension}")));
            assert!(supported_agent_task_file(&format!(
                "report.{}",
                extension.to_ascii_uppercase()
            )));
        }
        for extension in ["sh", "exe", "bin", "zip"] {
            assert!(!supported_agent_task_file(&format!("report.{extension}")));
        }
    }
}
