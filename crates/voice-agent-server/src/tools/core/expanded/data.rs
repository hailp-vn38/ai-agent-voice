use crate::services::background::plugins::PluginError;
use serde_json::{Value, json};

/// No scripts, JQ interpreter or dynamic user-defined expressions: three
/// deterministic transformations over bounded input JSON.
pub(super) fn transform(arguments: &Value) -> Result<Value, PluginError> {
    let input = arguments["json_text"].as_str().ok_or(PluginError::ApplicationError)?;
    let value: Value = serde_json::from_str(input).map_err(|_| PluginError::ApplicationError)?;
    let output = match arguments["mode"].as_str().ok_or(PluginError::ApplicationError)? {
        "pick" => {
            let object = value.as_object().ok_or(PluginError::ApplicationError)?;
            let fields = arguments["fields"].as_array().ok_or(PluginError::ApplicationError)?;
            if fields.is_empty() { return Err(PluginError::ApplicationError); }
            let mut selected = serde_json::Map::new();
            for field in fields {
                let field = field.as_str().ok_or(PluginError::ApplicationError)?;
                if let Some(member) = object.get(field) {
                    selected.insert(field.to_owned(), member.clone());
                }
            }
            Value::Object(selected)
        }
        "pluck" => {
            let field = arguments["field"].as_str().filter(|s| !s.is_empty())
                .ok_or(PluginError::ApplicationError)?;
            let items = value.as_array().ok_or(PluginError::ApplicationError)?;
            if items.len() > 128 { return Err(PluginError::ApplicationError); }
            Value::Array(items.iter()
                .map(|item| item.get(field).cloned().unwrap_or(Value::Null))
                .collect())
        }
        "filter_eq" => {
            let field = arguments["field"].as_str().filter(|s| !s.is_empty())
                .ok_or(PluginError::ApplicationError)?;
            let equals = arguments["equals"].as_str().ok_or(PluginError::ApplicationError)?;
            let items = value.as_array().ok_or(PluginError::ApplicationError)?;
            if items.len() > 128 { return Err(PluginError::ApplicationError); }
            Value::Array(items.iter()
                .filter(|item| item.get(field).and_then(Value::as_str) == Some(equals))
                .cloned().collect())
        }
        _ => return Err(PluginError::ApplicationError),
    };
    if output.to_string().len() > 16_000 {
        return Err(PluginError::ApplicationError);
    }
    Ok(json!({"value":output}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_modes_are_deterministic_and_bounded() {
        let picked = transform(&json!({"json_text":r#"{"a":1,"b":2}"#,
            "mode":"pick","fields":["b"]})).unwrap();
        assert_eq!(picked["value"], json!({"b":2}));
        let plucked = transform(&json!({"json_text":r#"[{"a":1},{"b":2}]"#,
            "mode":"pluck","field":"a"})).unwrap();
        assert_eq!(plucked["value"], json!([1,null]));
        let filtered = transform(&json!({"json_text":r#"[{"tag":"yes"},{"tag":"no"}]"#,
            "mode":"filter_eq","field":"tag","equals":"yes"})).unwrap();
        assert_eq!(filtered["value"], json!([{"tag":"yes"}]));
        assert!(transform(&json!({"json_text":"{}","mode":"pick"})).is_err());
        assert!(transform(&json!({"json_text":"[]","mode":"pluck"})).is_err());
    }
}
