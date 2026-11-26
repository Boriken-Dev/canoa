# `canoa-mail`

The person's own mail as a Canoa session service: the `mail` provider of the `session_service` kind (`canoa::session_service`), which gives an agent's sessions the `canoa-mail` MCP server and answers the control methods `mail/accounts`, `mail/add`, `mail/test` and `mail/remove`, and the `mail` provider of the `settings_section` kind, the Mail page of Settings: a section with no form of its own whose `collection` names the `mail/*` methods, so the application draws the mailboxes, the add form of the chosen backend, the sign-in test and Remove from what they answer and holds no mail code. Mailboxes are opened through the `mail_backend` providers the host loaded (`canoa-microsoft-mail` offers `ews`, `graph` and `outlook`; any library listed in `[host.extensions]` can add more).

`canoa-host` carries it as a bundled library (`bundled::INCLUDED`); a host built without that line has no mail: no server, no `mail/*` methods, no Mail page. Nothing else in the host names mail.

```rust
pub const LIBRARY: &str = "canoa-mail";
pub fn library(host: Host) -> Option<LibraryParts>;     // offers MailService and MailSection
pub struct MailService;                                 // #[provider(SessionService, id = "mail")]
pub struct MailSection;                                 // #[provider(SettingsSection, id = "mail")]
```

## The table (`[host.services.mail]`)

```toml
[host.services.mail]
tools = true                          # false turns the tools off while the accounts stay (default true)
[host.services.mail.accounts.NAME]    # NAME: 1 to 32 lower-case letters, digits, - and _
backend = "ews"                       # a mail_backend provider id
id = "…"                              # the host's secret entry that holds the password; written by `mail/add`
password_env = "VAR"                  # a variable that holds it instead, read before `id`
timeout_secs = 60                     # at least 1, no ceiling
# …plus the backend's own settings, checked by the backend's `validate`
```

The table holds no password: a `password` key is refused. The service reads it through `ServiceHost::config("mail")` at each call and writes `accounts.NAME` through `set_config`, so canoa.cfg's comments and layout stay. An opened mailbox is kept per host and account while the account is as it was.

## The tools (`canoa-mail` MCP server)

Offered to the sessions of a connection whose key holds the host's `services` grant, when `tools` is not false and an account is named, with the built-in skill `canoa-mail`.

- `mail_list {account?}` (accounts, folders with counts), `mail_search {query?, from?, to?, subject?, folder?, after?, before?, has_attachment?, unread?, flagged?, limit?, offset?, account?}` (default 25, at most 100; with no account every account is searched and one that fails is reported in `errors`), `mail_read {id | message_id | item_id, account?, thread?, max_chars?}`, `mail_related {id | message_id | item_id, account?, limit?}` (each result has `related_by`: `conversation`, `subject`, `person`), `mail_draft {to, cc?, bcc?, subject?, body, account?}` or `{reply_to, reply_all?, body}`, `mail_move {ids, folder}`, `mail_archive {ids}`, `mail_update {ids, read?, flag?, add_categories?, remove_categories?}`.
- A message id is `ACCOUNT:` and the backend's own id; one change names at most 50 ids of one account. `message_id` (the Internet message id, tried with and without its brackets) and `item_id` are looked for in every account in the table's order, or only in `account`; the first account that holds it answers, and an account that cannot be searched is named in the error when none does.
- **Never sends.** There is no tool or `Mailbox` call that sends. A draft is saved in Drafts, and the answer says `"sent": false`.
- **Asked first.** `mail_move`, `mail_archive` and `mail_update` read the named messages from the mailbox and answer an `Outcome` whose `ask` is the change and those messages in plain words and a digest of both; the host puts it to the person and calls `resume`, and nothing changes until the answer is `Allowed` (or `Silent`, when the session is in a mode that asks for nothing, which the host then says in the session).
- **Mail is data.** Every string of a message is stripped of control, direction and zero-width characters and cut (subject and names 300 characters, body `max_chars`, default 20000, at most 200000, newlines kept); every answer carries the note and the skill says the same.
- Related mail (`canoa_mail::mail_backend::related`) and thread summaries run over a backend's `Mailbox` methods (`search`, `summaries`, `thread`).

## The control methods

Answered to a key holding the host's `services.setup` grant; an answer never carries a password.

- `mail/accounts {}` -> `{accounts: [{name, backend, backend_title, state, details, settings, server?, email?, user?}], backends: [{id, title, description, no_secret_state, schema, form}], tools}`. `state` is `password stored`, `needs a password`, `password from VAR`, `needs VAR set`, or the backend's own words.
- `mail/add {name, backend, settings.., password?, id?, password_env?, timeout_secs?}` -> `{account}`: validates through `MailBackend::validate`, keeps the password with the host (`put_secret`) under a new id when the backend needs one and none was given, writes the table, and forgets the password again if the table cannot be written. A backend that needs a secret and got none is a usage error.
- `mail/test {name}` -> `{ok: true, folders, message}` or `{ok: false, problem}`: opens the mailbox and lists its folders; a sign-in that fails is an answer, not an error (`-32003` for an unknown name).
- `mail/remove {name}` -> `{removed: true}`: takes the table out and forgets the stored password.

## Tests

`cargo test -p canoa-mail` runs the service against a host kept in memory (`testkit.rs`) and the loopback Exchange stand-in of `canoa-microsoft-mail` (feature `standin`). The host's own tests (`cargo test -p canoa-host --lib -- store::mcp::extra_tests control::services_tests serve::tests`) drive the same library through the MCP link, the grants and the approval the person gives.

- `[host.disabled] mail_backend = ["ews"]` turns a backend off inside this service: the catalog of backends (`BackendCatalog::of`) leaves out each id `ServiceHost::is_off("mail_backend", id)` answers for, so the library's other backends stay, an account of the one that is off cannot be added, and the accounts already in canoa.cfg are kept and listed.

## `mail_backend` (`canoa_mail::mail_backend`)

The Guatiao extension kind for mailboxes (`mail_backend`, paired with the stateful Guatiao object `mailbox`), so mailbox connectors live in libraries outside `canoa-host` and `canoa` (the kind belongs to this crate; `canoa-microsoft-mail` implements it) and describe their add-account form via `guatiao-intake` schemas.

```rust
pub const KIND: &str = "mail_backend";   pub const KIND_VERSIONS: [&str; 1] = ["1"];
#[guatiao::kind(name = "mail_backend")]
pub trait MailBackend: Send + Sync {
    fn describe(&self) -> Result<BackendInfo, ProviderError>;
    fn validate(&self, request: ValidateRequest) -> Result<PreparedAccount, ProviderError>;
    fn open(&self, request: OpenMailbox) -> Result<Object<dyn Mailbox>, ProviderError>;
}
#[guatiao::kind(object, name = "mailbox")]
pub trait Mailbox: Send + Sync {
    fn folders(&self) -> Result<Vec<Folder>, ProviderError>;
    fn search(&self, query: Query) -> Result<Page, ProviderError>;
    fn read(&self, id: String) -> Result<Message, ProviderError>;
    fn find_by_message_id(&self, message_id: String) -> Result<Option<String>, ProviderError>;
    fn summaries(&self, ids: Vec<String>) -> Result<Vec<Summary>, ProviderError>;
    fn thread(&self, id: String) -> Result<Vec<Summary>, ProviderError>;
    fn draft(&self, draft: Draft) -> Result<String, ProviderError>;
    fn change(&self, ids: Vec<String>, change: Change) -> Result<(), ProviderError>;
}
```

- `BackendInfo` (`new(id, title)`, `.taking::<S: SettingsSchema>()`, `.problem()`, `description`, `no_secret_state`, `schema`, `form`), `ValidateRequest` (`new(settings, has_secret)`), `PreparedAccount` (`needs_secret`, `state`, `details`, `check_urls`, `no_secret_state`), `OpenMailbox` (`new(settings, timeout_secs)`, `secret`, `no_store`), `MailError` (`Bad`, `Auth`, `Unreachable`, `Backend`, converting to and from `ProviderError` without losing variant meaning), `Person`, `Summary`, `Attachment`, `Message`, `Folder`, `Query`, `Page`, `Draft` (`new_message`, `reply`, `kind()`), `Change` (`move_to`, `archive`, `read`, `flag`, `categories`, `kind()`), `FlagState`, `Related`, and shared helpers `WELL_KNOWN_FOLDERS`, `is_valid_id`, `well_known`, `find_folder`, `plain_subject`, `related`, `default_summaries`, `default_thread`.
