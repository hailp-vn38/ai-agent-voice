//! Static enrollment prompts: no Agent provider, transcript or conversational permit.
mod audio;

use crate::config::{EnrollmentConfig, EnrollmentTransport};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub use audio::{PromptAssets, PromptError};

pub struct EnrollmentRuntime {
    assets: Arc<PromptAssets>,
    connections: Arc<Semaphore>,
    encoders: Arc<Semaphore>,
}

impl EnrollmentRuntime {
    pub async fn prepare(config: &EnrollmentConfig) -> Result<Option<Arc<Self>>, PromptError> {
        if !config.enabled || config.transport != EnrollmentTransport::Websocket {
            return Ok(None);
        }
        let path = config.prompt_assets_dir.clone();
        let assets = tokio::task::spawn_blocking(move || PromptAssets::load(&path))
            .await
            .map_err(|_| PromptError)?;
        Ok(Some(Arc::new(Self {
            assets: Arc::new(assets?),
            connections: Arc::new(Semaphore::new(config.ws_max_connections)),
            encoders: Arc::new(Semaphore::new(2)),
        })))
    }

    pub fn try_connection(&self) -> Option<OwnedSemaphorePermit> {
        self.connections.clone().try_acquire_owned().ok()
    }

    pub async fn encode(&self, code: String) -> Result<Vec<Vec<u8>>, PromptError> {
        let permit = self.encoders.clone().acquire_owned().await.map_err(|_| PromptError)?;
        let assets = self.assets.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            assets.encode(&code)
        })
        .await
        .map_err(|_| PromptError)?
    }
}

#[cfg(test)]
mod tests;
