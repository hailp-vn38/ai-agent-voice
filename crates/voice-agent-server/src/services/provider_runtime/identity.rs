/// Exact desired version. Database ids are AUTOINCREMENT and never accepted in Admin input.
/// Deployment identities occupy a separate namespace.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ProviderIdentity {
    Database(i64),
    Deployment { kind: String, key: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProviderVersion {
    pub identity: ProviderIdentity,
    pub revision: i64,
}

impl ProviderVersion {
    pub fn database(id: i64, revision: i64) -> Self {
        Self {
            identity: ProviderIdentity::Database(id),
            revision,
        }
    }
}

/// Opaque hash of an adapter-owned typed resource specification and verified artifact identity.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ResourceKey(pub [u8; 32]);
