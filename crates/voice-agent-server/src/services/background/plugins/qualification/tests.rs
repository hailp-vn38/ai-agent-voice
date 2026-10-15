use super::*;
use std::io::Write;

fn package() -> archive::Package {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let root = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/web-extractor"
    ));
    for name in [
        "plugin.toml",
        "instructions.md",
        "schemas/extract_html.input.json",
        "schemas/fetch_url.input.json",
        "schemas/extract.output.json",
        "schemas/settings.json",
    ] {
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer
            .write_all(&std::fs::read(root.join(name)).unwrap())
            .unwrap();
    }
    let mut elf = vec![0; 64];
    elf[..6].copy_from_slice(b"\x7fELF\x02\x01");
    elf[16] = 2;
    elf[18] = if std::env::consts::ARCH == "aarch64" {
        183
    } else {
        62
    };
    writer
        .start_file("bin/plugin", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(&elf).unwrap();
    archive::validate(&writer.finish().unwrap().into_inner()).unwrap()
}

#[test]
fn cases_require_declared_operations_valid_inputs_and_coverage() {
    let package = package();
    let mut cases: Vec<Case> = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/web-extractor/check-cases.json"
    )))
    .unwrap();
    validate_cases(&package, &cases).unwrap();
    let case = cases.pop().unwrap();
    cases[1].arguments = json!({"url": 12});
    assert!(validate_cases(&package, &cases).is_err());
    cases[1] = case;
    cases[0].operation = "undeclared".into();
    assert!(validate_cases(&package, &cases).is_err());
    assert!(validate_cases(&package, &cases[1..]).is_err());
}

#[tokio::test]
async fn python_static_check_validates_package_without_claiming_runtime_pass() {
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/fixture-research-1.0.0.zip"
    ));
    let report = check(bytes, &[], None, Path::new("unused")).await.unwrap();
    assert_eq!(report["plugin_id"], "fixture-research");
    assert_eq!(report["package"], "PASS");
    assert_eq!(report["runtime"], "NOT_RUN");
    assert_eq!(report["cases"], json!([]));
}

#[tokio::test]
async fn static_check_does_not_claim_runtime_pass() {
    let package = package();
    let report = check(&package.source_zip, &[], None, Path::new("unused"))
        .await
        .unwrap();
    assert_eq!(report["package"], "PASS");
    assert_eq!(report["runtime"], "NOT_RUN");
}

#[tokio::test]
async fn static_check_rejects_instructions_the_agent_cannot_read() {
    let package = package();
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in &package.files {
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer
            .write_all(if name == "instructions.md" {
                &[0xff]
            } else {
                bytes
            })
            .unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    assert!(check(&bytes, &[], None, Path::new("unused")).await.is_err());
}

#[tokio::test]
async fn shared_artifact_check_rejects_wrong_size_and_hash() {
    let root = std::env::temp_dir().join(format!("plugin-artifact-check-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir(&root).await.unwrap();
    tokio::fs::write(root.join("result.txt"), b"data")
        .await
        .unwrap();
    for (size, hash) in [(Some(3), None), (Some(4), Some("wrong".to_owned()))] {
        let refs = vec![protocol::ArtifactRef {
            path: "result.txt".into(),
            media_type: "text/plain".into(),
            size_bytes: size,
            sha256: hash,
        }];
        assert!(verify_artifacts(&root, refs, 1024).await.is_err());
    }
    use sha2::{Digest, Sha256};
    let digest: String = Sha256::digest(b"data")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    for hash in [None, Some(digest)] {
        let refs = vec![protocol::ArtifactRef {
            path: "result.txt".into(),
            media_type: "text/plain".into(),
            size_bytes: Some(4),
            sha256: hash,
        }];
        verify_artifacts(&root, refs, 1024).await.unwrap();
    }
    tokio::fs::remove_dir_all(root).await.unwrap();
}
