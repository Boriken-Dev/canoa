# canoa-ffi

The Canoa host as a library inside an application. CanoaUI loads it, so it hosts agents in its own process with no `canoa`
program on disk: the host's threads run in the application, and agents stay child processes.

It is not published: the only consumer is the app's own plugin, `app/native`, which builds it with the app. The contract
is in [`AGENTS.md`](AGENTS.md).
