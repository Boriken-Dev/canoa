// The `_meta` keys of `initialize` that name the account and the model a session starts on.
// Kept apart from the models of accounts so the connection code can use them without the app's UI libraries.

/// The `_meta` key of `initialize` through which the chat names the account a session runs on.
const accountMeta = 'dev.boriken.canoa/account';

/// The `_meta` key of `initialize` naming the model a session starts on, by the id its account lists: the host refuses a start on a
/// model the account does not list, naming both.
const modelMeta = 'dev.boriken.canoa/model';
