use crate::providers::{VisionError, VisionProvider, VisionRequest, VisionResponse};
use std::{sync::Arc, time::Duration};

pub struct VisionRuntime {
    provider: Arc<dyn VisionProvider>,
    semaphore: Arc<tokio::sync::Semaphore>,
    timeout: Duration,
}
impl VisionRuntime {
    pub fn new(provider: Arc<dyn VisionProvider>, concurrency: usize, timeout: Duration) -> Self {
        Self {
            provider,
            semaphore: Arc::new(tokio::sync::Semaphore::new(concurrency)),
            timeout,
        }
    }
    pub async fn analyze(&self, request: VisionRequest) -> Result<VisionResponse, VisionError> {
        let permit = self
            .semaphore
            .clone()
            .try_acquire_owned()
            .map_err(|_| VisionError::Overloaded)?;
        let result = tokio::time::timeout(self.timeout, self.provider.analyze(request)).await;
        drop(permit);
        result.unwrap_or(Err(VisionError::Timeout))
    }
    pub fn provider(&self) -> Arc<dyn VisionProvider> {
        Arc::clone(&self.provider)
    }
}
