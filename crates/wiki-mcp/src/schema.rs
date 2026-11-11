use serde_json::Value;

pub(super) fn sanitize_schema(value: &mut Value) {
    const ALLOWED: &[&str] = &[
        "type",
        "description",
        "properties",
        "required",
        "additionalProperties",
        "enum",
        "const",
        "items",
        "minimum",
        "maximum",
        "minLength",
        "maxLength",
    ];
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.retain(|key, _| ALLOWED.contains(&key.as_str()));
    if let Some(Value::Object(properties)) = object.get_mut("properties") {
        properties.values_mut().for_each(sanitize_schema);
    }
    if let Some(items) = object.get_mut("items") {
        sanitize_schema(items);
    }
}
