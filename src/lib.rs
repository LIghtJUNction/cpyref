//! Exact-byte snapshots and strict selectors for agent copy/paste workflows.
pub mod bundle;
pub mod context;
pub mod error;
pub mod find;
pub mod handoff;
pub mod inspect;
pub mod prepare;
pub mod runner;
pub mod search;
pub mod select;
pub mod snapshot;
pub mod store;
pub mod verify;

pub const DEFAULT_MAX_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_BYTES: usize = 128 * 1024 * 1024;

pub mod edit;
pub mod plan;
pub mod redact;
pub mod reference;
