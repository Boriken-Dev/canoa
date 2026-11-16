//! The tools `canoa-kb` gives an agent: `kb_list`, `kb_read` and `kb_search`, answered by coqui-aos's knowledge session over
//! what this host's indexes hold. They read the knowledge an overlay ships and never change anything.

use std::time::Duration;

use coqui_aos::{KnowledgeSession, ToolCall};
use serde_json::{Value, json};

use super::Access;
use super::search::Backend;

/// How long a call waits for an index still being made.
pub(super) const WAIT: Duration = Duration::from_secs(20);

/// The tools and their input, as `tools/list` answers.
pub(super) fn list(access: &Access) -> Value {
    let backend = Backend(access);
    let session =
        KnowledgeSession::new(&access.overlays, &access.builder.knowledge, &access.caller)
            .search(&backend);
    json!({"tools": session.tools()})
}

/// Runs the tool `name` and answers as `tools/call` does: the text, and whether it is an error.
pub(super) fn call(access: &Access, name: &str, arguments: &Value) -> Value {
    let backend = Backend(access);
    KnowledgeSession::new(&access.overlays, &access.builder.knowledge, &access.caller)
        .search(&backend)
        .call(&ToolCall::new(name, arguments.clone()))
        .to_mcp()
}
