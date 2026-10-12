//! Native VieNeu v3 Turbo model preparation and contract validation.
//!
//! Not a registered production adapter yet: inference, phonemization and the retained
//! TTS worker must be implemented and verified before exposing it in the catalog.
pub mod assets;
pub mod config;
pub mod contract;
pub mod descriptor;
pub mod text;
