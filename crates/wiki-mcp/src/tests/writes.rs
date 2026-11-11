#[tokio::test]
async fn write_tools_preserve_patch_ingest_and_delete_compatibility() {
    let (dir, host) = host(true);
    host.call(
            context(Access::ReadWrite),
            "wiki_write_file",
            json!({"scope":"wiki","rel_path":"concepts/patch.md","content":"# Patch\n\nSee [[Old Page]].\n"}),
        )
        .await
        .unwrap();
    let patched = host
            .call(
                context(Access::ReadWrite),
                "wiki_patch_file",
                json!({"rel_path":"concepts/patch.md","target_text":"Old Page","replacement_text":"New Page"}),
            )
            .await
            .unwrap();
    assert!(patched["content"].as_str().unwrap().contains("(docid:"));
    assert!(dir.path().join("wiki/concepts/patch.md").exists());

    let ingested = host
        .call(
            context(Access::ReadWrite),
            "wiki_ingest_source",
            json!({
                "source_rel_path":"notes/provided.md",
                "raw_source_content":"# Source\n\nSupplied source content.",
                "title":"Provided source",
                "one_line_summary":"Supplied summary.",
                "wiki_markdown_body":"# Provided source\n\nSupplied wiki content.",
                "tags":["provided"],
                "related_entities":[],
                "related_concepts":[]
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        ingested["metadata"]["source_rel_path"],
        "sources/notes/provided.md"
    );
    assert!(dir.path().join("sources/notes/provided.md").exists());
    let wiki_path = ingested["metadata"]["wiki_rel_path"].as_str().unwrap();
    assert!(dir.path().join(wiki_path).exists());

    let synthesis = host
        .call(
            context(Access::ReadWrite),
            "wiki_file_synthesis",
            json!({
                "rel_path":"answer-without-extension",
                "query":"Which approach should we use?",
                "title":"Approach comparison",
                "one_line_summary":"A focused comparison summary.",
                "markdown_body":"## Recommendation\n\nUse the simpler approach.",
                "tags":["architecture"]
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        synthesis["metadata"]["rel_path"],
        "wiki/syntheses/answer-without-extension.md"
    );
    assert!(dir.path().join("wiki/log.md").exists());

    let deleted = host
        .call(
            context(Access::ReadWrite),
            "wiki_delete_file",
            json!({"scope":"wiki","rel_path":"concepts/patch.md"}),
        )
        .await
        .unwrap();
    assert_eq!(deleted["metadata"]["rel_path"], "concepts/patch.md");
    assert_eq!(deleted["metadata"]["deleted"], true);
    assert!(!dir.path().join("wiki/concepts/patch.md").exists());
}
use super::{context, host};
use crate::Access;
use serde_json::json;
