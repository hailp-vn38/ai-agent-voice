use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct VisionRequest {
    pub question: Arc<str>,
    pub image: Arc<[u8]>,
    pub mime_type: Arc<str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisionResponse {
    pub text: String,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum VisionError {
    #[error("vision request is invalid")]
    InvalidRequest,
    #[error("vision provider timed out")]
    Timeout,
    #[error("vision provider is overloaded")]
    Overloaded,
    #[error("vision upstream rejected the request")]
    UpstreamRejected,
    #[error("vision upstream response is invalid")]
    InvalidResponse,
    #[error("vision transport failed")]
    Transport,
}

#[async_trait::async_trait]
pub trait VisionProvider: Send + Sync {
    fn adapter(&self) -> &'static str;
    async fn analyze(&self, request: VisionRequest) -> Result<VisionResponse, VisionError>;
}
