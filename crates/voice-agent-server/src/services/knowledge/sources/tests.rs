
use super::*;
#[tokio::test]
async fn immutable_source_verifies_provenance_and_checksum() {
    let root = std::env::temp_dir().join(format!("knowledge_source_{}", Uuid::new_v4()));
    let v = SourceVault::open(&root).await.unwrap();
    let s = v
        .import_markdown(SourceInput {
            name: "agent.md".into(),
            content: "# Evidence\nOriginal source".into(),
            provenance: "admin:manual".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        v.verify(&format!("{}:1", s.id)).await.unwrap().sha256,
        s.sha256
    );
    assert_eq!(v.list().await.unwrap().len(), 1);
    assert!(v.verify(&format!("{}:2", s.id)).await.is_err());
    assert!(v.verify("../../etc/passwd:1").await.is_err());
    tokio::fs::write(
        root.join("raw").join(&s.id).join("1/original.md"),
        "modified",
    )
    .await
    .unwrap();
    assert!(v.verify(&format!("{}:1", s.id)).await.is_err());
    drop(v);
    let _ = tokio::fs::remove_dir_all(root).await;
}
