//! Source-qualified references and credential-free deployment catalog metadata.
use crate::{
    config::AppConfig,
    database::{DesiredProvider, providers::Provider},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderOrigin {
    Database,
    Config,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProviderRef {
    pub origin: ProviderOrigin,
    pub kind: String,
    pub key: String,
}
impl ProviderRef {
    pub fn parse(input: &str, expected_kind: Option<&str>) -> Result<Self, ()> {
        let (origin, kind, key) = if let Some(rest) = input.strip_prefix("config:") {
            let (kind, key) = rest.split_once(':').ok_or(())?;
            if !["vad", "asr", "llm", "tts", "speaker"].contains(&kind)
                || expected_kind.is_some_and(|expected| expected != kind)
            {
                return Err(());
            }
            (ProviderOrigin::Config, kind, key)
        } else {
            (ProviderOrigin::Database, expected_kind.unwrap_or(""), input)
        };
        if key.is_empty()
            || key.len() > 128
            || !key.bytes().all(|c| {
                c.is_ascii_alphanumeric()
                    || b"_-".contains(&c)
                    || (origin == ProviderOrigin::Database && c == b'.')
            })
        {
            return Err(());
        }
        Ok(Self {
            origin,
            kind: kind.into(),
            key: key.into(),
        })
    }
    pub fn public_key(&self) -> String {
        match self.origin {
            ProviderOrigin::Database => self.key.clone(),
            ProviderOrigin::Config => format!("config:{}:{}", self.kind, self.key),
        }
    }
    pub fn snapshot(&self, config: &AppConfig) -> Result<DesiredProvider, ()> {
        if self.origin != ProviderOrigin::Config {
            return Err(());
        }
        super::deployment_provider_snapshot(config, &self.kind, &self.key).map_err(|_| ())
    }
}
pub(crate) fn deployment_snapshots(config: &AppConfig) -> Vec<DesiredProvider> {
    let kinds = [
        (
            "vad",
            config.providers.vad.instances.keys().collect::<Vec<_>>(),
        ),
        ("asr", config.providers.asr.instances.keys().collect()),
        ("llm", config.providers.llm.instances.keys().collect()),
        ("tts", config.providers.tts.instances.keys().collect()),
        (
            "speaker",
            config.providers.speaker.instances.keys().collect(),
        ),
    ];
    kinds
        .into_iter()
        .flat_map(|(kind, keys)| {
            keys.into_iter().filter_map(move |key| {
                ProviderRef::parse(&format!("config:{kind}:{key}"), Some(kind))
                    .ok()?
                    .snapshot(config)
                    .ok()
            })
        })
        .collect()
}
pub(crate) fn public_provider(snapshot: &DesiredProvider) -> Provider {
    Provider {
        id: 0,
        key: ProviderRef {
            origin: ProviderOrigin::Config,
            kind: snapshot.kind.clone(),
            key: snapshot.key.clone(),
        }
        .public_key(),
        name: snapshot.key.clone(),
        kind: snapshot.kind.clone(),
        adapter: snapshot.adapter.clone(),
        config_json: "{}".into(),
        enabled: 1,
        revision: snapshot.revision,
        created_at: 0,
        updated_at: 0,
        credential_json: None,
    }
}
#[cfg(test)]
mod tests;
