pub mod assets;
pub(crate) mod config;
pub mod descriptor;
pub(crate) mod provider;

pub(crate) use provider::{UnavailableAsr, ZipformerAsrProvider};
