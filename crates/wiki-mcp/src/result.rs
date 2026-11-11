use serde_json::{Value, json};

pub(super) fn bound_result_chars(mut value: Value, max_chars: usize) -> Value {
    let serialized = serde_json::to_string(&value).unwrap_or_default();
    if serialized.len() <= max_chars {
        return value;
    }

    if let Some(object) = value.as_object_mut() {
        object.insert("truncated".into(), Value::Bool(true));
        if let Some(metadata) = object.get("metadata").cloned() {
            let metadata = match metadata {
                Value::Object(mut values) => {
                    for key in ["results", "tree", "entries", "backlinks", "outgoing_links"] {
                        values.remove(key);
                    }
                    Value::Object(values)
                }
                _ => Value::Null,
            };
            object.insert("metadata".into(), metadata);
        }
    }
    if serde_json::to_string(&value).unwrap_or_default().len() <= max_chars {
        return value;
    }

    if let Some(Value::String(content)) = value
        .as_object_mut()
        .and_then(|object| object.get_mut("content"))
    {
        let budget = max_chars.saturating_sub(256);
        let mut escaped_len = 0;
        let mut cut = content.len();
        for (index, character) in content.char_indices() {
            escaped_len += match character {
                '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
                c if c < ' ' => 6,
                c => c.len_utf8(),
            };
            if escaped_len > budget {
                cut = index;
                break;
            }
        }
        content.truncate(cut);
        content.push_str("\n...[truncated]");
    }
    if serde_json::to_string(&value).unwrap_or_default().len() <= max_chars {
        return value;
    }
    json!({"truncated": true, "content": "...[truncated]"})
}
