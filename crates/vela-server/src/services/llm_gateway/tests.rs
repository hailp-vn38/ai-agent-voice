use std::{collections::BTreeMap, sync::Arc};

use url::Url;

use crate::{
    config::DatabaseConfig,
    database::{Database, credentials::CredentialCipher},
};

use super::{
    CreatePoolModelRequest, FakeOAuthVendorDriver, GatewayAccountStatus, GatewayBinding,
    GatewayCallOutcome, GatewayError, GatewayVendor, ImportCookieAccountRequest, LlmGatewayHub,
    OAuthCallbackSubmission, OAuthLoginMode, OAuthPollStatus, OAuthVendorDriver,
    StartOAuthLoginRequest, UpdateGatewayAccountPatch, UpdatePoolModelPatch,
};

async fn test_hub_with_fakes() -> (
    Arc<LlmGatewayHub>,
    BTreeMap<GatewayVendor, Arc<FakeOAuthVendorDriver>>,
    Database,
) {
    let db = Database::connect(&DatabaseConfig {
        url: "sqlite::memory:".into(),
        max_connections: 1,
        ..Default::default()
    })
    .await
    .expect("in-memory sqlite connect");

    let cipher: Arc<dyn crate::database::secrets::SecretResolver> =
        Arc::new(CredentialCipher::new(1, &[0x42; 32]).expect("cipher"));
    let mut fake_map = BTreeMap::new();
    let mut dyn_map: BTreeMap<GatewayVendor, Arc<dyn OAuthVendorDriver>> = BTreeMap::new();
    for vendor in GatewayVendor::ALL {
        let fake = Arc::new(FakeOAuthVendorDriver::new(vendor));
        fake_map.insert(vendor, Arc::clone(&fake));
        dyn_map.insert(vendor, fake);
    }

    let hub = Arc::new(LlmGatewayHub::with_drivers(
        Some(db.pool().clone()),
        cipher,
        dyn_map,
    ));
    (hub, fake_map, db)
}

mod account_lifecycle;
mod muse_runtime;
mod oauth;
mod pool_models;
mod quotas;
mod refresh_failures;
