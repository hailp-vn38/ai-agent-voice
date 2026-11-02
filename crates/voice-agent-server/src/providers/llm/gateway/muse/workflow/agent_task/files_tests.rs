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
