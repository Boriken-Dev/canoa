// The `_meta` keys of `initialize` that name the account and the model a session starts on, and the one the host answers with the
// longest message it takes.
// Kept apart from the models of accounts so the connection code can use them without the app's UI libraries.

/// The `_meta` key of `initialize` through which the chat names the account a session runs on.
const accountMeta = 'dev.boriken.canoa/account';

/// The `_meta` key of `initialize` naming the model a session starts on, by the id its account lists: the host refuses a start on a
/// model the account does not list, naming both.
const modelMeta = 'dev.boriken.canoa/model';

/// The `_meta` key of the host's answer to `initialize` holding `[host] max_message_bytes`: the longest ACP message it takes, in bytes.
const maxMessageMeta = 'dev.boriken.canoa/maxMessageBytes';
