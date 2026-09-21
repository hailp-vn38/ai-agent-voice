/// Stable description of the adapters supplied to application-owned worker runtimes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub vad_adapter: &'static str,
    pub asr_adapter: &'static str,
}
