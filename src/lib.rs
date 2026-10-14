//! Exact-byte snapshots and strict selectors for agent copy/paste workflows.
pub mod context;
pub mod error;
pub mod handoff;
pub mod runner;
pub mod search;
pub mod select;
pub mod snapshot;
pub mod store;
pub mod verify;

pub const DEFAULT_MAX_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_BYTES: usize = 128 * 1024 * 1024;

pub mod edit;
pub mod redact;
