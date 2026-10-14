//! Exact-byte snapshots and strict selectors for agent copy/paste workflows.
pub mod error;
pub mod select;
pub mod store;

pub const DEFAULT_MAX_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_BYTES: usize = 128 * 1024 * 1024;
