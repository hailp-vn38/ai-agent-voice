use super::execution::take_chars;
use super::*;
#[test]
fn frozen_roundtrip_and_revision_fail_closed() {
    for value in frozen_choices() {
        let choice = decode(&value).unwrap();
        assert!(matches_frozen(&value, &choice));
        assert!(contract(&choice).is_ok());
        let mut changed = choice;
        changed.version = "unsupported".into();
        assert!(contract(&changed).is_err());
    }
}
#[test]
fn json_pointer_contract_is_projectable_and_artifact_is_job_scoped() {
    assert!(contract(&decode(&frozen_choices()[0]).unwrap()).is_ok());
    assert!(contract(&decode(&frozen_choices()[1]).unwrap()).is_ok());
    assert_eq!(take_chars("aéb", 2), ("aé".into(), true));
}

#[test]
fn web_is_hidden_but_persisted_choices_remain_supported() {
    let choices = frozen_choices();
    let catalog = bundled_catalog();
    assert_eq!(catalog["total"], 3);
    assert_eq!(catalog["operation_total"], choices.len());
    assert!(
        catalog["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["id"] != "core.web")
    );
    for operation in ["web.fetch", "web.extract"] {
        assert!(
            choices
                .iter()
                .all(|choice| choice["operation"] != operation)
        );
        let persisted = serde_json::json!({
            "kind":"core", "plugin_id":"core", "operation":operation,
            "contract_revision":"1"
        });
        assert!(decode(&persisted).is_ok());
    }
}

#[test]
fn json_pointer_rejects_invalid_escapes_and_accepts_literal_tildes() {
    use super::execution::valid_pointer;
    for pointer in ["/a~2", "/a~", "a", "/~01~3"] {
        assert!(!valid_pointer(pointer), "{pointer}");
    }
    for pointer in ["", "/", "/a~0b~1c", "/~01", "/é"] {
        assert!(valid_pointer(pointer), "{pointer}");
    }
}

#[test]
fn text_result_preserves_unicode_and_fits_serialized_step_budget() {
    use super::execution::bound_text_result;
    use serde_json::json;
    for text in ["\0".repeat(16000), "😀".repeat(16000)] {
        for mut result in [
            json!({"text":text,"truncated":false,"size_bytes":64000}),
            json!({"text":text,"status":200,"content_type":"text/plain",
                "truncated":false,"provenance":[{"source":"web","urls":["https://example.org"],"capabilities":["web_fetch"]}]}),
        ] {
            let original = result["text"].as_str().unwrap().to_owned();
            bound_text_result(&mut result).unwrap();
            assert!(result.to_string().len() <= 65536);
            assert!(original.starts_with(result["text"].as_str().unwrap()));
            if result["text"] != original {
                assert_eq!(result["truncated"], true);
            }
        }
    }
    let mut result = json!({"text":"aéb","truncated":false,"size_bytes":4});
    let original = result.clone();
    bound_text_result(&mut result).unwrap();
    assert_eq!(result, original);
}
