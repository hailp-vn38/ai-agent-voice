//! Statically linked, safe-read Agent Job operations. Not Plugin Store installations.
//! Risk 2 (network target/SSRF policy) is explicitly deferred; do not expose web.fetch
//! or web.extract to untrusted tenants until a separate policy is implemented.

mod catalog;
mod contracts;
mod execution;
mod expanded;

pub(crate) use catalog::{
    CorePluginSetting, bundled_catalog, bundled_item, enabled_groups, group_for, known_group,
};
pub(crate) use contracts::{contract, decode, frozen_choices, is_core, matches_frozen};
pub(crate) use execution::invoke;
use execution::take_chars;

#[cfg(test)]
mod tests;

pub(crate) fn tts_settings_schema() -> serde_json::Value {
    serde_json::json!({"type":"object","properties":{
        "provider_ref":{"type":"string","minLength":1,"maxLength":256}
    },"required":["provider_ref"],"additionalProperties":false})
}
pub(crate) fn tts_settings_digest() -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(serde_json::to_vec(&tts_settings_schema()).expect("static schema"))
        .iter().map(|byte| format!("{byte:02x}")).collect()
}
