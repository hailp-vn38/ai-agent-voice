use super::read_regular;

#[cfg(unix)]
#[tokio::test]
async fn artifact_reader_rejects_symlink_and_hardlink() {
    let dir = std::env::temp_dir().join(format!("job-artifact-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let original = dir.join("original");
    std::fs::write(&original, b"fixture").unwrap();
    let link = dir.join("link");
    std::os::unix::fs::symlink(&original, &link).unwrap();
    assert!(read_regular(link, 1024).await.is_err());
    let hardlink = dir.join("hardlink");
    std::fs::hard_link(&original, &hardlink).unwrap();
    assert!(read_regular(hardlink, 1024).await.is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}
