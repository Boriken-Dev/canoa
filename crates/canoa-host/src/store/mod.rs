//! The host's end of the `program_store` kind: the folder Canoa keeps programs in, the checks on what an install writes,
//! the tools a store fetches and unpacks with, and the stores built into this binary.

pub mod builtin;
pub mod contract;
pub mod echo;
pub mod engine;
pub mod exec;
pub mod extensions;
pub mod footprint;
pub mod programs;
pub mod protect;
pub mod registry;
pub mod shelf;
pub mod tools;
