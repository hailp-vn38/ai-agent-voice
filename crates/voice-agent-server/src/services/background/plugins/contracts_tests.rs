use super::*;
    #[test]
    fn projection_keeps_optional_nullable_but_source_schema_decides() {
        let source = serde_json::json!({"type":"object","properties":{
            "url":{"type":"string","minLength":10},
            "max_chars":{"type":"integer","minimum":1}
        },"required":["url"],"additionalProperties":false});
        let projected = strict_projection(&source).unwrap();
        assert_eq!(projected["required"],serde_json::json!(["max_chars","url"]));
        assert_eq!(projected["properties"]["max_chars"]["type"],serde_json::json!(["integer","null"]));
        let mut args = serde_json::json!({"url":"https://example.com","max_chars":null});
        normalize_tool_arguments(&source,&mut args);
        assert_eq!(args,serde_json::json!({"url":"https://example.com"}));
        assert!(jsonschema::validator_for(&source).unwrap().is_valid(&args));
    }
    #[test]
    fn unsupported_schema_cannot_be_projected() {
        assert!(strict_projection(&serde_json::json!({"$ref":"https://example.com"})).is_none());
    }
