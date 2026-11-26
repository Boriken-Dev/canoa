//! Approvers: where an agent's permission request can be answered besides the window that runs the session.
//!
//! The person adds approvers in `[permissions.approvers.NAME]`, each a provider of the `approver` kind: the host's own `policy` (the
//! person's rules, answered on the spot) and any an extension offers. `chain` says the order a request is put to them in and what
//! the first answer does; `options` maps a verdict onto the options the agent offered, the same way a hook's answer is. With no
//! approver added the window alone answers, as before.
//!
//! What an approver may say is `allow` or `reject` once: it never names an option and never answers "always". A timeout never
//! allows.

pub(crate) mod catalog;
pub(crate) mod chain;
pub mod config;
pub(crate) mod notice;
pub(crate) mod options;
pub(crate) mod policy;
pub(crate) mod shared;

pub use config::{Approvers, Detail, Entry};
