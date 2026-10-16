use super::contract;
use crate::services::background::{
    JobEngine,
    plugins::{FrozenChoice, InvokeResult, PluginError},
};
use serde_json::{Value, json};

async fn knowledge_runtime(
    engine: &JobEngine,
) -> Result<&crate::services::knowledge::Runtime, PluginError> {
    let data_root = engine
        .database
        .durable_path()
        .and_then(|path| path.parent())
        .ok_or(PluginError::StorageUnavailable)?
        .to_path_buf();
    let core = engine
        .knowledge_runtime
        .get_or_try_init(|| async {
            crate::services::knowledge::Runtime::open(data_root)
                .await
                .map_err(|_| PluginError::StorageUnavailable)
        })
        .await?;
    Ok(core.as_ref())
}

pub(super) fn take_chars(input: &str, cap: usize) -> (String, bool) {
    let result: String = input.chars().take(cap).collect();
    let truncated = input.chars().count() > cap;
    (result, truncated)
}

pub(crate) async fn invoke(
    engine: &JobEngine,
    choice: &FrozenChoice,
    arguments: Value,
    job_id: &str,
) -> Result<InvokeResult, PluginError> {
    let definition = contract(choice)?;
    let input_validator = jsonschema::validator_for(&definition["input_schema"])
        .map_err(|_| PluginError::WorkerProtocolError)?;
    if !input_validator.is_valid(&arguments) {
        return Err(PluginError::ApplicationError);
    }
    let maximum = arguments["max_chars"].as_u64().unwrap_or(16000).min(16000) as usize;
    let mut result = match choice.operation.as_str() {
        "data.json_query" => {
            let document = arguments["json_text"]
                .as_str()
                .ok_or(PluginError::ApplicationError)?;
            let pointer = arguments["pointer"]
                .as_str()
                .ok_or(PluginError::ApplicationError)?;
            if !valid_pointer(pointer) {
                return Err(PluginError::ApplicationError);
            }
            let value: Value =
                serde_json::from_str(document).map_err(|_| PluginError::ApplicationError)?;
            let selected = value
                .pointer(pointer)
                .ok_or(PluginError::ApplicationError)?;
            if selected.to_string().len() > 16000 {
                return Err(PluginError::ApplicationError);
            }
            json!({"value":selected})
        }
        "artifact.read_text" => {
            let artifact = arguments["artifact_id"]
                .as_str()
                .ok_or(PluginError::ApplicationError)?;
            let (_, content, size) = super::expanded::owned_text(engine, job_id, artifact).await?;
            let (text, truncated) = take_chars(&content, maximum);
            json!({"text":text,"truncated":truncated,"size_bytes":size})
        }
        "knowledge.search" => {
            let query = arguments["query"]
                .as_str()
                .ok_or(PluginError::ApplicationError)?;
            let limit = arguments["limit"].as_i64().unwrap_or(10);
            let core = knowledge_runtime(engine).await?;
            let items = core
                .knowledge
                .search(query, limit)
                .await
                .map_err(|_| PluginError::ApplicationError)?;
            json!({"items":items})
        }
        "knowledge.read" => {
            let id = arguments["id"]
                .as_str()
                .ok_or(PluginError::ApplicationError)?;
            let from_line = arguments["from_line"].as_u64().unwrap_or(1) as usize;
            let max_lines = arguments["max_lines"].as_u64().unwrap_or(100) as usize;
            let core = knowledge_runtime(engine).await?;
            let page = core
                .knowledge
                .read_verified(id, from_line, max_lines)
                .await
                .map_err(|_| PluginError::ApplicationError)?;
            let (text, truncated) = take_chars(&page.body, 16000);
            json!({"id":page.id,"title":page.title,"revision":page.revision,"text":text,"truncated":truncated})
        }
        "memory.recall" => {
            let query = arguments["query"]
                .as_str()
                .ok_or(PluginError::ApplicationError)?;
            let limit = arguments["limit"].as_i64().unwrap_or(10);
            let core = knowledge_runtime(engine).await?;
            let items = core
                .memory
                .recall(query, limit)
                .await
                .map_err(|_| PluginError::ApplicationError)?;
            json!({"items":items})
        }
        "memory.summary" => {
            let core = knowledge_runtime(engine).await?;
            let text = core
                .memory
                .summary()
                .await
                .map_err(|_| PluginError::ApplicationError)?;
            json!({"text":text})
        }
        "web.fetch" => super::expanded::fetch(&arguments).await?,
        "web.extract" | "document.extract" | "document.inspect" | "data.transform"
        | "artifact.inspect" => {
            super::expanded::invoke(engine, &choice.operation, &arguments, job_id).await?
        }
        _ => return Err(PluginError::PermissionRequired),
    };
    bound_text_result(&mut result)?;
    let output_validator = jsonschema::validator_for(&definition["output_schema"])
        .map_err(|_| PluginError::WorkerProtocolError)?;
    if !output_validator.is_valid(&result) {
        return Err(PluginError::WorkerProtocolError);
    }
    Ok(InvokeResult {
        result,
        artifacts: Vec::new(),
        workspace: None,
    })
}

// serde_json resolves pointer tokens but does not reject invalid RFC 6901 escapes.
pub(super) fn valid_pointer(pointer: &str) -> bool {
    if !pointer.is_empty() && !pointer.starts_with('/') {
        return false;
    }
    let mut chars = pointer.chars();
    while let Some(c) = chars.next() {
        if c == '~' && !matches!(chars.next(), Some('0' | '1')) {
            return false;
        }
    }
    true
}

/// Preserve the durable Step's serialized byte budget, including escaping and metadata.
pub(super) fn bound_text_result(result: &mut Value) -> Result<(), PluginError> {
    const MAX_BYTES: usize = 65536;
    if result.to_string().len() <= MAX_BYTES {
        return Ok(());
    }
    let text = result["text"]
        .as_str()
        .ok_or(PluginError::ApplicationError)?
        .to_owned();
    result["truncated"] = json!(true);
    let mut low = 0;
    let mut high = text.chars().count();
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        result["text"] = json!(text.chars().take(mid).collect::<String>());
        if let Some(offset) = result["offset"].as_u64() {
            result["next_offset"] = json!(offset + mid as u64);
        }
        if result.to_string().len() <= MAX_BYTES {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    result["text"] = json!(text.chars().take(low).collect::<String>());
    if let Some(offset) = result["offset"].as_u64() {
        result["next_offset"] = json!(offset + low as u64);
    }
    if result.to_string().len() > MAX_BYTES {
        return Err(PluginError::ApplicationError);
    }
    Ok(())
}
