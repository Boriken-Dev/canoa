//! The host's end of the `program_store` kind: the folder Canoa keeps programs in, the checks on what an install writes,
//! the tools a store fetches and unpacks with, and the stores built into this binary.

pub mod bitwarden;
pub mod builtin;
pub(crate) mod catalogs;
pub mod contract;
pub mod echo;
pub(crate) mod extension_catalog;
#[cfg(test)]
mod extension_catalog_tests;
pub mod footprint;
pub(crate) mod index;
pub(crate) mod listings;
pub(crate) mod mcp;
pub mod piper;
pub mod programs;
pub mod registry;
pub mod runtimes;
pub mod shelf;
pub mod shell_stores;
pub(crate) mod sources;
pub mod tools;
