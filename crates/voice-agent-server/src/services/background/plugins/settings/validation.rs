//! Bounded Settings payload for shared-worker one-shot JSONL.
use crate::services::background::plugins::PluginError;
use serde_json::Value;

/// Arguments may independently reach 16 KiB; reserve ample space below the
/// worker's 65,535-byte input cap for both fields and protocol metadata.
pub(super) fn validate_worker_budget(settings: &Value) -> Result<(), PluginError> {
    let encoded = serde_json::to_vec(settings).map_err(|_| PluginError::PermissionRequired)?;
    if encoded.len() > 16 * 1024 {
        return Err(PluginError::PermissionRequired);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accounts_for_utf8_and_json_escaping() {
        assert!(validate_worker_budget(&json!({"token":"A".repeat(16_000)})).is_ok());
        assert!(validate_worker_budget(&json!({"token":"A".repeat(16_500)})).is_err());
        assert!(validate_worker_budget(&json!({"token":"\\n".repeat(9_000)})).is_err());
    }

    #[test]
    fn many_secret_fields_cannot_overflow_worker_envelope() {
        let mut values = serde_json::Map::new();
        for n in 0..16 {
            values.insert(format!("key_{n}"), json!("s".repeat(3_500)));
        }
        assert!(validate_worker_budget(&Value::Object(values)).is_err());
    }
}
