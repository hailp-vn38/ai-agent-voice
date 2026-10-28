//! Internal LLM Gateway hub managing OAuth 2.0 accounts (Anthropic, OpenAI Codex, Gemini, GitHub Copilot),
//! out-of-band proactive token renewal, and zero-SQL in-RAM lease acquisition (ADR-0130).

pub mod hub;
pub mod oauth_flow;
pub mod types;
pub mod vendors;

#[cfg(test)]
mod tests;

pub use hub::{LlmGatewayHub, SharedGatewayHub};
pub use types::{
    CreatePoolModelRequest, GatewayAccountQuota, GatewayAccountStatus, GatewayAccountView,
    GatewayBinding, GatewayBoundProviderRef, GatewayCallOutcome, GatewayError, GatewayLease,
    GatewayPoolDetailView, GatewayPoolModelView, GatewayQuotaWindow, GatewayVendor,
    GatewayVendorInfo, GatewayVendorModelPreset, OAuthCallbackSubmission, OAuthLoginChallenge,
    OAuthLoginMode, OAuthPollStatus, OAuthTokenBundle, StartOAuthLoginRequest,
    UpdateGatewayAccountPatch, UpdatePoolModelPatch, VendorQuotaSnapshot, valid_account_key,
};
pub use vendors::{
    DeviceCodeGrant, DevicePollOutcome, ExchangedOAuthAccount, FakeOAuthVendorDriver,
    OAuthVendorDriver, default_vendor_drivers,
};
