use super::*;
impl PluginStore {
    pub(crate) async fn operation_contract(
        &self,
        choice: &FrozenChoice,
    ) -> Result<serde_json::Value, PluginError> {
        let row = self.row(&choice.plugin_id).await?;
        if row.status != "ready"
            || row.version != choice.version
            || row.package_digest != choice.package_digest
            || row.installation_digest != choice.installation_digest
        {
            return Err(PluginError::PermissionRequired);
        }
        let bytes = tokio::fs::read(
            self.root
                .join("packages")
                .join(&choice.plugin_id)
                .join(&choice.version)
                .join(&choice.package_digest)
                .join("package.zip"),
        )
        .await
        .map_err(|_| PluginError::StorageUnavailable)?;
        let package = archive::validate(&bytes)?;
        if package.digest != choice.package_digest || package.manifest.plugin.id != choice.plugin_id
        {
            return Err(PluginError::InvalidPackage);
        }
        let operation = package
            .manifest
            .operations
            .get(&choice.operation)
            .ok_or(PluginError::PermissionRequired)?;
        let read = |path: &str| {
            package
                .files
                .iter()
                .find(|(name, _)| name == path)
                .map(|(_, bytes)| bytes.as_slice())
                .ok_or(PluginError::InvalidPackage)
        };
        let input_schema: serde_json::Value =
            serde_json::from_slice(read(&operation.input_schema)?)
                .map_err(|_| PluginError::InvalidPackage)?;
        let output_schema: serde_json::Value =
            serde_json::from_slice(read(&operation.output_schema)?)
                .map_err(|_| PluginError::InvalidPackage)?;
        let instructions = std::str::from_utf8(read(&package.manifest.agent.instructions)?)
            .map_err(|_| PluginError::InvalidPackage)?;
        if instructions.len() > 16384 {
            return Err(PluginError::InvalidPackage);
        }
        Ok(
            json!({"description":operation.description,"instructions":instructions,"input_schema":input_schema,"output_schema":output_schema,"provides":operation.provides,"sources":operation.sources}),
        )
    }
}

/// Compile only already-frozen, Child-allowed operations. Never use a live store
/// listing to expand the tool authority granted by the Root.
pub(crate) fn compile_child_tools(
    pinned: &[(FrozenChoice, serde_json::Value)],
) -> Result<(
    Vec<crate::providers::llm::ToolDefinition>,
    std::collections::HashMap<String, (FrozenChoice, serde_json::Value)>
), crate::services::background::JobError> {
    use crate::{providers::llm::ToolDefinition, services::background::JobError};
    let mut tools = Vec::with_capacity(pinned.len());
    let mut bindings = std::collections::HashMap::new();
    for (choice, contract) in pinned {
        let name = format!("p_{}_{}", choice.plugin_id.replace('-', "_"), choice.operation.replace('-', "_"));
        if name.len() > 64 || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || name == "submit_result" || bindings.contains_key(&name)
        {
            return Err(JobError::CapabilityUnavailable);
        }
        let projected = strict_projection(&contract["input_schema"])
            .ok_or(JobError::CapabilityUnavailable)?;
        let description = contract["description"].as_str().ok_or(JobError::CapabilityUnavailable)?;
        if description.len() > 1024 { return Err(JobError::CapabilityUnavailable); }
        tools.push(ToolDefinition {
            name: name.clone(), description: description.to_owned(), parameters: projected,
        });
        bindings.insert(name, (choice.clone(), contract.clone()));
    }
    Ok((tools, bindings))
}

/// Only a small safe superset of the source schema is sent to the model.
/// All original constraints, including oneOf/not and min/max, stay Rust-owned.
fn strict_projection(schema: &serde_json::Value) -> Option<serde_json::Value> {
    use serde_json::{Value, json};
    fn walk(input: &Value, optional: bool, depth: usize) -> Option<Value> {
        if depth > 8 { return None; }
        let kind = input.get("type").and_then(Value::as_str).or_else(|| {
            input.get("enum").and_then(Value::as_array).and_then(|items|
                items.first().and_then(|x| if x.is_string(){Some("string")}else if x.is_i64(){Some("integer")}else{None}))
        })?;
        let mut output = match kind {
            "object" => {
                let props = input.get("properties")?.as_object()?;
                let required = input.get("required").and_then(Value::as_array);
                let mut projected = serde_json::Map::new();
                for (key, item) in props {
                    let mandatory = required.is_some_and(|arr| arr.iter().any(|v| v.as_str()==Some(key)));
                    projected.insert(key.clone(), walk(item,!mandatory,depth+1)?);
                }
                json!({"type":"object","properties":projected,
                    "required":props.keys().collect::<Vec<_>>(),"additionalProperties":false})
            }
            "array" => json!({"type":"array","items":walk(input.get("items")?,false,depth+1)?}),
            "string" | "integer" | "number" | "boolean" => json!({"type":kind}),
            _ => return None,
        };
        if let Some(values) = input.get("enum").and_then(Value::as_array) {
            if !values.is_empty() {
                output["enum"] = Value::Array(values.clone());
            }
        }
        if optional {
            if let Some(values) = output.get_mut("enum").and_then(Value::as_array_mut) {
                if !values.contains(&Value::Null) { values.push(Value::Null); }
            }
            output["type"] = json!([kind,"null"]);
        }
        Some(output)
    }
    walk(schema,false,0)
}

/// Strip only optional nulls introduced by strict tool projection. Required null
/// values stay invalid under the plugin's source JSON Schema.
pub(crate) fn normalize_tool_arguments(schema: &serde_json::Value, args: &mut serde_json::Value) {
    use serde_json::Value;
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {return};
    let required = schema.get("required").and_then(Value::as_array);
    let Some(values) = args.as_object_mut() else {return};
    for (key, child_schema) in properties {
        let mandatory = required.is_some_and(|arr| arr.iter().any(|v| v.as_str()==Some(key)));
        if !mandatory && values.get(key).is_some_and(Value::is_null) {
            values.remove(key);
        } else if let Some(value) = values.get_mut(key) {
            normalize_tool_arguments(child_schema,value);
        }
    }
}

#[cfg(test)]
mod strict_tools_tests {
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
}
