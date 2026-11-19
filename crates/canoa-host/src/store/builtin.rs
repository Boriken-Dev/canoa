//! The echo stores the tests run: [`Recipe`](canoa::store::recipe::Recipe)s made into `program_store` providers.

canoa::store_provider!(
    /// The echo program's store, one file.
    EchoStore, super::echo::EchoRecipe, "echo_store"
);

canoa::store_provider!(
    /// The echo program's store, an archive with an extra step.
    EchoArchiveStore, super::echo::EchoArchiveRecipe, "echo_archive_store"
);

canoa::store_provider!(
    /// The echo program's store as Claude Code's is published: kept on the shelf as `claude`, for a run that starts an entry on it.
    EchoClaudeStore, super::echo::EchoClaudeRecipe, "echo_claude_store"
);
