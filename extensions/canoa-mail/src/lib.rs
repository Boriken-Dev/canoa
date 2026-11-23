//! The person's own mail as a Canoa session service: the `mail` provider of the `session_service` kind (the `canoa-mail` MCP
//! server an agent's sessions are given, and the control methods `mail/accounts`, `mail/add`, `mail/test` and `mail/remove`) and
//! the `mail` section of Settings. The mailboxes are opened through the `mail_backend` providers the host loaded.
#![forbid(unsafe_code)]

mod accounts;
#[cfg(test)]
mod accounts_tests;
mod config;
mod forms;
mod mailboxes;
mod service;
#[cfg(test)]
mod service_tests;
#[cfg(test)]
mod testkit;
mod tools;

pub use service::{MailSection, MailService, library};

/// The name this crate's library is registered under.
pub const LIBRARY: &str = "canoa-mail";
