//! Provider Runtime Manager integration tests.
//!
//! Verifies:
//! - Acquisition & Provenance: concurrent deduplication, cold loads, timeouts, and idle lifetimes.
//! - Capacity & Lifecycle: quota sharing, resource draining, bounded queueing, and orderly shutdown.
//! - Resilience & Aliasing: quarantined resources, deployment default isolation, and cache pressure.
//! - Admission Enrollment: pilot cold-start serialization, draft isolation, and memory-pressure eviction.

#[path = "provider_runtime_manager/harness.rs"]
mod harness;

#[path = "provider_runtime_manager/acquisition.rs"]
mod acquisition;

#[path = "provider_runtime_manager/capacity_lifecycle.rs"]
mod capacity_lifecycle;

#[path = "provider_runtime_manager/resilience.rs"]
mod resilience;

#[path = "provider_runtime_manager/enrollment.rs"]
mod enrollment;
