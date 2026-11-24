//! Executor-side V2 capability discovery.
//! Connection and RPC routing stay outside this module; legacy V1 scanning remains separate.

pub(crate) mod capability_discoveries;
pub(crate) mod capability_locations;
pub(crate) mod capability_manager;
