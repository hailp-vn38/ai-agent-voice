//! Statically linked, safe-read Agent Job operations. Not Plugin Store installations.
//! Risk 2 (network target/SSRF policy) is explicitly deferred; do not expose web.fetch
//! to untrusted tenants until a separate policy is implemented.

mod contracts;
mod execution;

pub(crate) use contracts::{contract, decode, frozen_choices, is_core, matches_frozen};
pub(crate) use execution::invoke;

#[cfg(test)]
mod tests;
