//! Validation shared by native tool arguments and plugin outputs.
use serde_json::Value;

pub(crate) fn forbidden(value: &Value) -> bool {
    match value {
        Value::Object(fields) => fields.iter().any(|(key, value)| {
            key == "document_refs"
                || (key == "$ref"
                    && value
                        .as_str()
                        .is_some_and(|reference| !reference.starts_with('#')))
                || forbidden(value)
        }),
        Value::Array(values) => values.iter().any(forbidden),
        _ => false,
    }
}
