//! `canoa mail list|add|remove`: the mail accounts of canoa.cfg and the passwords that rest in the host's account store. A
//! password is typed without echo (or read from standard input when that is not a terminal) and never taken from the command
//! line; canoa.cfg keeps the id it rests under and never the password.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "environment variables are std's"
)]

use zeroize::Zeroize as _;

use super::PASSWORD_KEY;
use crate::accounts::commands::{
    Cfg, Env, accounts_of, append_table, check_cfg, new_id, read_cfg, remove_table, with_table,
    write_cfg,
};
use crate::accounts::{Accounts, Record};
use crate::cli::{Failed, Format};
use crate::config::{MailAccount, MailAuth, MailBackend};

/// Where the accounts are in canoa.cfg.
const SECTION: &[&str] = &["host", "mail", "accounts"];

/// What `canoa mail add` is told about the account; the password is asked for apart.
#[derive(Clone, Debug, Default)]
pub(crate) struct NewAccount {
    /// `ews` or `outlook`.
    pub(crate) backend: Option<MailBackend>,
    pub(crate) email: Option<String>,
    pub(crate) server: Option<String>,
    pub(crate) autodiscover: Option<String>,
    /// A user name asks for Basic sign-in, which needs a password; none signs in with the Windows logon.
    pub(crate) user: Option<String>,
    pub(crate) profile: Option<String>,
    pub(crate) timeout_secs: Option<u64>,
}

impl NewAccount {
    /// Whether the account signs in with a password.
    fn password(&self) -> bool {
        self.backend == Some(MailBackend::Ews) && self.user.is_some()
    }
}

/// What an account added is: for the line that says so.
#[derive(Clone, Debug)]
pub(crate) struct Added {
    pub(crate) name: String,
    pub(crate) backend: &'static str,
    pub(crate) id: Option<String>,
}

impl Added {
    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::json!({"mail": {"name": self.name, "backend": self.backend, "id": self.id}})
    }
}

/// The name of a backend, as canoa.cfg spells it.
pub(crate) const fn backend_name(backend: MailBackend) -> &'static str {
    match backend {
        MailBackend::Ews => "ews",
        MailBackend::Graph => "graph",
        MailBackend::Outlook => "outlook",
    }
}

/// The table of an account as TOML text and as a table, from one list of keys.
fn block_of(name: &str, new: &NewAccount, id: Option<&str>) -> (String, toml::Table) {
    let mut keys: Vec<(&str, toml::Value)> = Vec::new();
    let text = |value: &Option<String>| value.clone().map(toml::Value::String);
    keys.push((
        "backend",
        toml::Value::String(new.backend.map_or("", backend_name).to_owned()),
    ));
    for (key, value) in [
        ("email", text(&new.email)),
        ("server", text(&new.server)),
        ("autodiscover", text(&new.autodiscover)),
        ("user", text(&new.user)),
        ("id", id.map(|id| toml::Value::String(id.to_owned()))),
        ("profile", text(&new.profile)),
        (
            "timeout_secs",
            new.timeout_secs
                .and_then(|secs| i64::try_from(secs).ok())
                .map(toml::Value::Integer),
        ),
    ] {
        if let Some(value) = value {
            keys.push((key, value));
        }
    }
    let mut block = format!("[{}.{name}]\n", SECTION.join("."));
    let mut table = toml::Table::new();
    for (key, value) in keys {
        let spelled = match &value {
            toml::Value::String(text) => crate::accounts::commands::quote(text),
            other => other.to_string(),
        };
        block.push_str(&format!("{key} = {spelled}\n"));
        table.insert(key.to_owned(), value);
    }
    (block, table)
}

/// `canoa mail add NAME`: asks for the password through `front` when the account signs in with one.
pub(crate) fn add(
    env: &Env<'_>,
    name: &str,
    new: &NewAccount,
    format: Format,
) -> Result<(), Failed> {
    let mut ask = || -> Result<String, Failed> {
        let prompt = format!(
            "password of {} for {name}: ",
            new.user.as_deref().unwrap_or_default()
        );
        env.front
            .ask(&prompt, true)
            .map_err(Failed::run)?
            .ok_or_else(|| Failed::usage("this front cannot ask for a password"))
    };
    let added = add_with(env.cfg(), name, new, &mut ask)?;
    let line = match format {
        Format::Json => added.json().to_string(),
        Format::Text => format!("added {name} ({})", added.backend),
    };
    env.front.print(&line).map_err(Failed::run)?;
    if added.id.is_some()
        && format == Format::Text
        && let Ok(accounts) = accounts_of(env.cfg())
        && let Some(note) = accounts.note()
    {
        env.front.print(note).map_err(Failed::run)?;
    }
    Ok(())
}

/// Adds the account `name`: the password `ask` gives rests in the account store under a new id, and canoa.cfg names the id.
///
/// # Errors
///
/// The name is taken, canoa.cfg would not be read with the account in it, no password was given, or the store or the file could
/// not be written.
pub(crate) fn add_with(
    cfg: Cfg<'_>,
    name: &str,
    new: &NewAccount,
    ask: &mut dyn FnMut() -> Result<String, Failed>,
) -> Result<Added, Failed> {
    let Some(backend) = new.backend else {
        return Err(Failed::usage("--backend is ews or outlook"));
    };
    if cfg.config.mail.accounts.contains_key(name) {
        return Err(Failed::usage(format!(
            "a mail account called {name} exists"
        )));
    }
    let id = new.password().then(new_id).transpose()?;
    // The file is read with the account in it before anything is asked or stored, so a name or an address it would refuse is
    // refused first.
    let (as_text, as_table) = block_of(name, new, id.as_deref());
    let before = read_cfg(cfg.config_path)?;
    check_cfg(
        cfg.config_path,
        &with_table(
            cfg.config_path,
            &before,
            SECTION,
            name,
            (&as_text, as_table.clone()),
        )?,
    )?;
    let mut stored = None;
    if let Some(id) = &id {
        let mut password = ask()?;
        if password.is_empty() {
            password.zeroize();
            return Err(Failed::usage("no password was given"));
        }
        let accounts = accounts_of(cfg)?;
        let saved = store_password(&accounts, id, &password);
        password.zeroize();
        saved?;
        stored = Some(accounts);
    }
    if let Err(failed) = append_table(cfg.config_path, SECTION, name, (&as_text, as_table)) {
        if let (Some(accounts), Some(id)) = (&stored, &id) {
            let _ = accounts.forget_credentials(id);
        }
        return Err(failed);
    }
    Ok(Added {
        name: name.to_owned(),
        backend: backend_name(backend),
        id,
    })
}

/// Keeps `password` in the account store under `id`.
///
/// # Errors
///
/// The store failed.
pub(crate) fn store_password(accounts: &Accounts, id: &str, password: &str) -> Result<(), Failed> {
    let mut record = Record::default();
    record.revision = 1;
    record
        .credentials
        .insert(PASSWORD_KEY.to_owned(), password.to_owned());
    accounts
        .save(id, &record)
        .map_err(|error| Failed::run(error.to_string()))
}

/// `canoa mail remove NAME`.
pub(crate) fn remove(env: &Env<'_>, name: &str) -> Result<(), Failed> {
    remove_with(env.cfg(), name)?;
    env.front
        .print(&format!("removed {name}"))
        .map_err(Failed::run)
}

/// Removes the account `name` from canoa.cfg and forgets the password the store holds for it. Nothing is changed in the
/// mailbox.
///
/// # Errors
///
/// No account has the name, canoa.cfg does not hold it as `add` writes it, or the store failed.
pub(crate) fn remove_with(cfg: Cfg<'_>, name: &str) -> Result<(), Failed> {
    let account = cfg
        .config
        .mail
        .accounts
        .get(name)
        .ok_or_else(|| Failed::usage(format!("no mail account is called {name}")))?;
    let before = read_cfg(cfg.config_path)?;
    remove_table(cfg.config_path, SECTION, name, "canoa mail add")?;
    if let Some(id) = &account.id {
        let forgotten = accounts_of(cfg).and_then(|accounts| {
            accounts
                .forget_credentials(id)
                .map_err(|error| Failed::run(error.to_string()))
        });
        if let Err(failed) = forgotten {
            let _ = write_cfg(cfg.config_path, &before);
            return Err(failed);
        }
    }
    Ok(())
}

/// What an account's password is, for a person: where it comes from and whether it is there. Never the password.
pub(crate) fn state_of(account: &MailAccount, store: Option<&Accounts>) -> String {
    match (account.backend, account.auth) {
        (MailBackend::Outlook, _) => "signed in by Outlook".to_owned(),
        (_, MailAuth::Windows) => "signs in with the Windows logon".to_owned(),
        (_, MailAuth::Basic) => {
            if let Some(variable) = account.password_env.as_deref().filter(|v| !v.is_empty()) {
                let set = std::env::var(variable).is_ok_and(|value| !value.is_empty());
                return if set {
                    format!("password from {variable}")
                } else {
                    format!("needs {variable} set")
                };
            }
            let (Some(id), Some(store)) = (account.id.as_deref(), store) else {
                return "needs a password".to_owned();
            };
            match store.load(id) {
                Ok(Some(record)) if record.credentials.contains_key(PASSWORD_KEY) => {
                    "password stored".to_owned()
                }
                Ok(_) => "needs a password".to_owned(),
                Err(_) => "the account store could not be read".to_owned(),
            }
        }
    }
}

/// One account as `canoa mail list` shows it.
#[derive(Clone, Debug)]
pub(crate) struct Row {
    pub(crate) name: String,
    pub(crate) backend: &'static str,
    pub(crate) server: Option<String>,
    pub(crate) email: Option<String>,
    pub(crate) user: Option<String>,
    pub(crate) state: String,
}

impl Row {
    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::json!({"name": self.name, "backend": self.backend, "server": self.server,
            "email": self.email, "user": self.user, "state": self.state})
    }
}

/// The accounts canoa.cfg names with their state.
pub(crate) fn rows(cfg: Cfg<'_>) -> Vec<Row> {
    let wants_store = cfg
        .config
        .mail
        .accounts
        .values()
        .any(|account| account.id.is_some() && account.password_env.is_none());
    let accounts = wants_store.then(|| accounts_of(cfg).ok()).flatten();
    cfg.config
        .mail
        .accounts
        .iter()
        .map(|(name, account)| Row {
            name: name.clone(),
            backend: backend_name(account.backend),
            server: account.server.clone(),
            email: account.email.clone(),
            user: account.user.clone(),
            state: state_of(account, accounts.as_deref()),
        })
        .collect()
}

/// `canoa mail list`.
pub(crate) fn list(env: &Env<'_>, format: Format) -> Result<(), Failed> {
    for row in rows(env.cfg()) {
        let line = match format {
            Format::Json => row.json().to_string(),
            Format::Text => format!(
                "{} {} {} {}",
                row.name,
                row.backend,
                row.server
                    .as_deref()
                    .or(row.email.as_deref())
                    .unwrap_or("-"),
                row.state
            ),
        };
        env.front.print(&line).map_err(Failed::run)?;
    }
    Ok(())
}
