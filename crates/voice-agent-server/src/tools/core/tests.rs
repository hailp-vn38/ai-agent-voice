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
fn web_is_registered_and_persisted_choices_remain_supported() {
    let choices = frozen_choices();
    let catalog = bundled_catalog();
    assert_eq!(catalog["total"], 4);
    assert_eq!(catalog["operation_total"], 8);
    assert!(
        catalog["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "core.web")
    );
    for operation in ["web.fetch", "web.extract"] {
        let raw = choices
            .iter()
            .find(|choice| choice["operation"] == operation)
            .unwrap();
        let definition = contract(&decode(raw).unwrap()).unwrap();
        let input = jsonschema::validator_for(&definition["input_schema"]).unwrap();
        assert!(
            input.is_valid(
                &serde_json::json!({"url":"https://example.org","offset":0,"max_chars":1})
            )
        );
        for invalid in [
            serde_json::json!({"url":"https://example.org","offset":-1}),
            serde_json::json!({"url":"https://example.org","offset":1.5}),
            serde_json::json!({"url":"https://example.org","max_chars":0}),
            serde_json::json!({"url":"https://example.org","mode":"invalid"}),
        ] {
            assert!(!input.is_valid(&invalid));
        }
        if operation == "web.extract" {
            for mode in ["article", "page"] {
                assert!(
                    input.is_valid(&serde_json::json!({"url":"https://example.org","mode":mode}))
                );
            }
        }
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
                "truncated":false,"offset":7,"next_offset":null,"body_limit_reached":false,
                "provenance":[{"source":"web","urls":["https://example.org/".to_owned() + &"x".repeat(2000)],"capabilities":["web_fetch"]}]}),
        ] {
            let original = result["text"].as_str().unwrap().to_owned();
            bound_text_result(&mut result).unwrap();
            assert!(result.to_string().len() <= 65536);
            assert!(original.starts_with(result["text"].as_str().unwrap()));
            if result["text"] != original {
                assert_eq!(result["truncated"], true);
                if result["offset"].is_number() {
                    assert_eq!(
                        result["next_offset"],
                        7 + result["text"].as_str().unwrap().chars().count()
                    );
                    assert!(!result["text"].as_str().unwrap().is_empty());
                }
            }
        }
    }
    let mut result = json!({"text":"aéb","truncated":false,"size_bytes":4});
    let original = result.clone();
    bound_text_result(&mut result).unwrap();
    assert_eq!(result, original);
}
