//! The host's end of the `program_store` kind: the folder Canoa keeps programs in, the checks on what an install writes,
//! the tools a store fetches and unpacks with, and the stores built into this binary.

pub mod exec;
pub mod footprint;
pub mod shelf;
pub mod tools;
