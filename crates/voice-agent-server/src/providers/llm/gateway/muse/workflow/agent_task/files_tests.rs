use super::*;
use serde_json::json;
#[test]
fn agent_task_files_extract_supported_unique_paths() {
    assert_eq!(
        collect_file_paths(
            &[
                json!({"data":{"files":[{"path":"sandbox:///home/user/report.md"},{"path":"/tmp/script.sh"}]}})
            ],
            &[json!({"file_path":"sandbox:///home/user/table.csv"})],
            &["See sandbox:///home/user/report.md and sandbox:///home/user/result.pdf.".into()]
        ),
        vec![
            "/home/user/report.md",
            "/home/user/table.csv",
            "/home/user/result.pdf"
        ]
    );
}

#[test]
fn agent_task_files_extract_markdown_links() {
    assert_eq!(
        collect_file_paths(
            &[],
            &[],
            &[
                "Saved: [report.md](sandbox://tasks/a/report.md), [data.csv](sandbox://tasks/a/data.csv)."
                    .into()
            ]
        ),
        vec!["tasks/a/report.md", "tasks/a/data.csv"]
    );
}
