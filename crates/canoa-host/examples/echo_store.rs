//! The echo program's stores as a Guatiao library: what a store shipped outside this binary looks like, and what
//! `canoa-store-check` is run against. This file is a `cdylib` and nothing else, as a crate calling `providers!` has to be.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "an example names what it uses from std"
)]

use canoa_host::store::builtin::{EchoArchiveStore, EchoStore};
use coqui_acp_api::guatiao;

guatiao::providers!(EchoStore, EchoArchiveStore);
