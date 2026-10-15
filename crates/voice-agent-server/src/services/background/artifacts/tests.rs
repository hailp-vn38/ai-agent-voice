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

#[tokio::test]
async fn artifact_copy_streams_binary_and_checks_declared_hash() {
    let dir = std::env::temp_dir().join(format!("job-copy-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let source = dir.join("output.mp3");
    let bytes = vec![0xa5; 3 * 1024 * 1024];
    std::fs::write(&source, &bytes).unwrap();
    let target = dir.join(".staging");
    let (size, digest) = super::stage_output(source.clone(), target.clone(), 4 * 1024 * 1024, None, None).await.unwrap();
    assert_eq!(size, bytes.len() as u64);
    assert_eq!(std::fs::read(&target).unwrap(), bytes);
    assert_eq!(digest.len(), 64);
    std::fs::remove_file(&target).unwrap();
    assert!(super::stage_output(source.clone(), target, 4 * 1024 * 1024, Some(size), Some("0".repeat(64))).await.is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn artifact_copy_respects_byte_budget() {
    let dir = std::env::temp_dir().join(format!("job-budget-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let source = dir.join("video.mp4");
    std::fs::write(&source, vec![7u8; 1024]).unwrap();
    assert!(super::stage_output(source, dir.join(".staging"), 100, None, None).await.is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn legacy_v3_artifact_root_moves_once_without_changing_bytes() {
    let (db, _, runtime) = crate::services::background::tests::harness().await;
    runtime.shutdown().await;
    let old_root = std::path::PathBuf::from(format!(
        "{}.job-artifacts",
        db.durable_path().unwrap().display()
    ));
    let new_root = super::root(&db).unwrap();
    let job = uuid::Uuid::new_v4().to_string();
    let artifact = uuid::Uuid::new_v4().to_string();
    let old_folder = old_root.join(&job);
    tokio::fs::create_dir_all(&old_folder).await.unwrap();
    tokio::fs::write(old_folder.join(&artifact), b"immutable V3 file")
        .await
        .unwrap();
    super::relocate(&db).await.unwrap();
    super::relocate(&db).await.unwrap();
    assert_eq!(
        tokio::fs::read(new_root.join(job).join(artifact))
            .await
            .unwrap(),
        b"immutable V3 file"
    );
    assert!(!old_root.exists());
}
