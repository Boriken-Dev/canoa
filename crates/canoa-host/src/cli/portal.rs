//! `canoa portal` and `canoa portal factor` CLI handlers.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a command line names what it uses from std"
)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use super::commands::print;
use super::{Failed, Front};

/// What `canoa portal` takes.
#[derive(clap::Args, Debug)]
pub(super) struct PortalArgs {
    /// The canoa.cfg files whose `[portal]` table is the portal's settings, as `serve --config` names them (a list split at `;` on
    /// Windows and `:` elsewhere, the last highest); every key has a default. Without it the portal reads the installed and the
    /// system's canoa.cfg: a `[portal]` in the account's own file is ignored, since a person must not set what the portal enforces.
    #[arg(long, value_name = "FILES", env = canoa::brand::CONFIG)]
    config: Option<String>,
    /// The web app's build to serve, in place of the file's `site`.
    #[arg(long, value_name = "DIR")]
    site: Option<PathBuf>,
    /// A loopback TCP address to listen on too, in place of the file's `listen`: for development, since any local user can reach
    /// it and what it says of the browser's address is not believed.
    #[arg(long, value_name = "ADDRESS")]
    listen: Option<SocketAddr>,
    /// The Unix socket a proxy reaches the portal at, in place of the file's `socket`.
    #[arg(long, value_name = "PATH")]
    socket: Option<PathBuf>,
    /// Something to do for the portal's accounts instead of serving.
    #[command(subcommand)]
    action: Option<PortalAction>,
}

/// What `canoa portal` does besides serving. Run as the account the portal runs as, which owns the files it keeps.
#[derive(clap::Subcommand, Debug)]
enum PortalAction {
    /// The one-time codes the portal keeps: who has one, setting one up for a person, taking one away.
    Factor {
        #[command(subcommand)]
        action: FactorAction,
    },
    /// The passkeys the portal keeps: who has one, and taking one away. A person adds one on the portal's Passkeys page.
    Passkeys {
        #[command(subcommand)]
        action: PasskeyAction,
    },
    /// The portal's settings, the `[portal]` table of the system's canoa.cfg (the last file `--config` names): read what the portal
    /// would run with, or change one key. Run as an account that may write that file.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

/// `canoa portal config`.
#[derive(clap::Subcommand, Debug)]
enum ConfigAction {
    /// Print the `[portal]` table the layers give, as one JSON object, or one key of it (`a.b` for a key inside a table).
    Get {
        /// The key; the whole table when it is left out.
        key: Option<String>,
    },
    /// Set one key of `[portal]` in the file that is written, after checking that the portal would take the whole table.
    Set {
        /// The key (`a.b` for a key inside a table).
        key: String,
        /// The value as JSON (`300`, `true`, `["a","b"]`, `{"k": 1}`), or text.
        #[arg(allow_hyphen_values = true)]
        value: String,
    },
    /// Take one key out of `[portal]`, so its default applies again.
    Unset {
        /// The key (`a.b` for a key inside a table).
        key: String,
    },
}

/// `canoa portal passkeys`.
#[derive(clap::Subcommand, Debug)]
enum PasskeyAction {
    /// Print one line for each passkey: its account, id, label, the times it was added and last used, and whether it syncs between
    /// devices. No key is printed.
    List,
    /// Take one passkey, or every passkey of an account, away. The sessions a passkey opened end at their next use.
    Remove {
        /// The account, by name, as `list` shows it (an account that is gone still has its name).
        name: String,
        /// The passkey's id as `list` shows it; every passkey of the account when it is left out. An id is base64url and may start
        /// with a hyphen.
        #[arg(allow_hyphen_values = true)]
        id: Option<String>,
    },
}

/// `canoa portal factor`.
#[derive(clap::Subcommand, Debug)]
enum FactorAction {
    /// Print one line for each account that has a code set up or a setup waiting for its first code. No secret is printed.
    List,
    /// Begin a setup for NAME and print its link and recovery codes, once. The person (or you) confirms it with the first code
    /// from the app: run this again with `--code`. Nothing is a factor until then.
    Enrol {
        /// The account, by name.
        name: String,
        /// The first code from the app: confirms the setup that is waiting.
        #[arg(long, value_name = "CODE", allow_hyphen_values = true)]
        code: Option<String>,
    },
    /// Clear what the portal counted of NAME's failed second-step tries, and the provider's delay of the account. For a person who
    /// was locked out by someone else's guesses and has changed their password. A running portal acts on it at the account's next
    /// try.
    Unlock {
        /// The account, by name.
        name: String,
    },
    /// Remove an account's code, its recovery codes and any setup waiting. The sessions made with the code end at their next use,
    /// and the account is marked: it sets no code up for itself until you set one up with `enrol`. Change the account's
    /// password too: a code that had to be reset may have been lost with the password.
    Reset {
        /// The account, by name.
        #[arg(required_unless_present = "id")]
        name: Option<String>,
        /// The code's id as `list` shows it (`1000:alice`), for an account that is gone.
        #[arg(
            long,
            value_name = "ID",
            conflicts_with = "name",
            allow_hyphen_values = true
        )]
        id: Option<String>,
    },
}

/// The files `[portal]` is read from, lowest first, and the one `portal config` writes. With `--config` every file named is the
/// administrator's and the last is written; without it the installed and the system's canoa.cfg are read, the system's is
/// written, and the account's own file is left out.
fn portal_layers(args: &PortalArgs) -> Result<(Vec<PathBuf>, PathBuf, Option<PathBuf>), Failed> {
    use crate::config::Layers;
    let named = args.config.as_deref().filter(|value| !value.is_empty());
    let layers = Layers::resolve(named, false).map_err(Failed::usage)?;
    if named.is_some() {
        return Ok((layers.files.clone(), layers.own.clone(), None));
    }
    let system = anycfg::system_config_path(Some(Path::new(crate::config::layers::NAME)));
    let read = layers
        .files
        .iter()
        .filter(|file| **file != layers.own || **file == system)
        .cloned()
        .collect();
    let own = (layers.own != system).then(|| layers.own.clone());
    Ok((read, system, own))
}

/// The `[portal]` table of one file; none when the file is not there or has none.
fn portal_table_of(file: &Path) -> Result<Option<toml::Table>, Failed> {
    use canoa::config_store::{ConfigStore as _, Key, json_of};
    if !file.exists() {
        return Ok(None);
    }
    let held = crate::config_store::FileStore::at(file)
        .get(Key::new(["portal"]))
        .map_err(|error| Failed::usage(format!("{}: {error}", file.display())))?;
    Ok(held
        .value
        .and_then(|value| json_of(&value))
        .and_then(|json| crate::accounts::commands::toml_value(&json))
        .and_then(|value| match value {
            toml::Value::Table(table) => Some(table),
            _ => None,
        }))
}

/// The `[portal]` tables of `files` merged, the one above replacing the one below key by key.
fn portal_table(files: &[PathBuf]) -> Result<toml::Table, Failed> {
    let mut merged = toml::Value::Table(toml::Table::new());
    for file in files {
        if let Some(table) = portal_table_of(file)? {
            crate::config::layers::merge_into(&mut merged, toml::Value::Table(table));
        }
    }
    match merged {
        toml::Value::Table(table) => Ok(table),
        _ => Ok(toml::Table::new()),
    }
}

/// Says that the account's own canoa.cfg holds a `[portal]` the portal does not read.
fn say_ignored(ignored: Option<PathBuf>, front: &dyn Front) -> Result<(), Failed> {
    if let Some(own) = ignored
        && portal_table_of(&own)?.is_some()
    {
        front.say(&format!(
            "canoa portal: ignoring [portal] in {}: it is read from the system's canoa.cfg only, or from the files --config names",
            own.display()
        ));
    }
    Ok(())
}

/// The settings `canoa portal` runs with: `[portal]` of the layers, then the command line over it.
fn portal_settings(
    args: &PortalArgs,
    front: &dyn Front,
) -> Result<crate::portal::Settings, Failed> {
    use crate::portal::Settings;
    let (files, _, ignored) = portal_layers(args)?;
    say_ignored(ignored, front)?;
    let mut settings = Settings::from_table(&portal_table(&files)?)
        .map_err(|why| Failed::usage(format!("[portal] {why}")))?;
    if let Some(site) = &args.site {
        settings.site = Some(site.clone());
    }
    if let Some(listen) = args.listen {
        settings.listen = Some(listen);
    }
    if let Some(socket) = &args.socket {
        settings.socket = Some(socket.clone());
    }
    Ok(settings)
}

/// `canoa portal config`: reads and changes `[portal]` in the file the portal's administrator writes.
fn portal_config(
    args: &PortalArgs,
    action: &ConfigAction,
    front: &dyn Front,
) -> Result<(), Failed> {
    use canoa::config_store::{Changes, ConfigStore as _, Op, value_of};
    let (files, target, ignored) = portal_layers(args)?;
    say_ignored(ignored, front)?;
    let merged = portal_table(&files)?;
    let path_of = |key: &str| -> Result<Vec<String>, Failed> {
        let keys: Vec<String> = key.split('.').map(str::to_owned).collect();
        if keys.iter().any(String::is_empty) {
            return Err(Failed::usage("a key is words joined by dots"));
        }
        Ok(keys)
    };
    match action {
        ConfigAction::Get { key } => {
            // A table the portal would refuse to start with is not printed as if it were one it runs.
            let settings = crate::portal::Settings::from_table(&merged).map_err(|why| {
                Failed::usage(format!("[portal] {why}: the portal would refuse to start"))
            })?;
            settings
                .check_helper()
                .map_err(|why| Failed::usage(format!("{why}: the portal would refuse to start")))?;
            let mut here = serde_json::to_value(&merged).map_err(Failed::run)?;
            if let Some(key) = key {
                for word in path_of(key)? {
                    here = here
                        .get(&word)
                        .cloned()
                        .ok_or_else(|| Failed::run(format!("{key}: not set")))?;
                }
            }
            print(front, &here.to_string())
        }
        ConfigAction::Set { key, value } => {
            let keys = path_of(key)?;
            let json: serde_json::Value = serde_json::from_str(value)
                .unwrap_or_else(|_| serde_json::Value::String(value.clone()));
            let mut after = merged;
            let toml_value = crate::accounts::commands::toml_value(&json)
                .ok_or_else(|| Failed::usage("a null is no setting"))?;
            set_in(&mut after, &keys, toml_value);
            crate::portal::Settings::from_table(&after)
                .map_err(|why| Failed::usage(format!("{key}: not taken: {why}")))?;
            crate::config_store::FileStore::at(&target)
                .change(Changes::of(vec![Op::set(
                    portal_path(&keys),
                    value_of(&json),
                )]))
                .map_err(|error| Failed::run(format!("{}: {error}", target.display())))?;
            print(
                front,
                &serde_json::json!({"set": key, "file": target}).to_string(),
            )
        }
        ConfigAction::Unset { key } => {
            let keys = path_of(key)?;
            crate::config_store::FileStore::at(&target)
                .change(Changes::of(vec![Op::remove(portal_path(&keys))]))
                .map_err(|error| Failed::run(format!("{}: {error}", target.display())))?;
            print(
                front,
                &serde_json::json!({"unset": key, "file": target}).to_string(),
            )
        }
    }
}

/// The path of a key of `[portal]` from the top of canoa.cfg.
fn portal_path(keys: &[String]) -> impl Iterator<Item = &str> {
    core::iter::once("portal").chain(keys.iter().map(String::as_str))
}

/// Puts `value` at `keys` below `[portal]`'s table, making the tables on the way.
fn set_in(table: &mut toml::Table, keys: &[String], value: toml::Value) {
    let Some((first, rest)) = keys.split_first() else {
        return;
    };
    if rest.is_empty() {
        table.insert(first.clone(), value);
        return;
    }
    let inner = table
        .entry(first.clone())
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    if !inner.is_table() {
        *inner = toml::Value::Table(toml::Table::new());
    }
    if let toml::Value::Table(inner) = inner {
        set_in(inner, rest, value);
    }
}

/// `canoa portal factor`: an administrator's way to set up, list and reset one-time codes, in the files the portal reads.
fn portal_factor(
    settings: &crate::portal::Settings,
    action: &FactorAction,
    front: &dyn Front,
) -> Result<(), Failed> {
    use crate::portal::{Accounts as _, FactorError, FactorStanding};
    let access = crate::portal::Access::new(settings).map_err(Failed::usage)?;
    // Before anything is written: a code file made by another account is one the portal cannot read.
    access.check_code_files_owned().map_err(Failed::usage)?;
    if access.factor_standing("", 0) == FactorStanding::Unsupported {
        return Err(Failed::usage(
            "no authenticator keeps one-time codes: name a provider that does (`coqui_access_totp`) in `authenticators`",
        ));
    }
    let failed = |error: FactorError| match error {
        FactorError::Unsupported => Failed::usage("this portal keeps no one-time codes"),
        FactorError::NothingWaiting => Failed::run(
            "no setup is waiting for NAME, or it timed out: run `factor enrol NAME` without `--code` first",
        ),
        FactorError::Wrong => Failed::run("that code was not accepted"),
        FactorError::Failed(why) => Failed::run(why),
    };
    match action {
        FactorAction::List => {
            for entry in access.list_factors().map_err(Failed::run)? {
                print(
                    front,
                    &serde_json::json!({
                        "id": entry.id,
                        "user": entry.user,
                        "enrolled": entry.enrolled,
                        "disabled": entry.disabled,
                        "waiting": entry.waiting,
                        "reset": entry.reset,
                    })
                    .to_string(),
                )?;
            }
            Ok(())
        }
        FactorAction::Enrol { name, code } => {
            let (user, uid) = access.admitted(name).map_err(Failed::usage)?;
            if let Some(code) = code {
                access
                    .confirm_factor(&user, uid, code.trim())
                    .map_err(failed)?;
                // An administrator set it up: the account is no longer one that is waiting for them.
                access.clear_reset(uid).map_err(Failed::run)?;
                return print(
                    front,
                    &serde_json::json!({"enrolled": {"user": user}}).to_string(),
                );
            }
            let begun = access.begin_factor(&user, uid, false).map_err(failed)?;
            print(
                front,
                &serde_json::json!({"enrol": {
                    "user": user,
                    "uri": begun.uri,
                    "recovery_codes": begun.codes,
                }})
                .to_string(),
            )
        }
        FactorAction::Unlock { name } => {
            let (user, uid) = access.admitted(name).map_err(Failed::usage)?;
            access.unlock(&user, uid).map_err(Failed::run)?;
            print(
                front,
                &serde_json::json!({"unlock": {"user": user}}).to_string(),
            )
        }
        FactorAction::Reset { name, id } => {
            let (shown, had) = match (name, id) {
                (_, Some(id)) => (id.clone(), access.reset_by_id(id).map_err(Failed::usage)?),
                (Some(name), None) => {
                    let (user, uid) = access.admitted(name).map_err(Failed::usage)?;
                    (
                        user.clone(),
                        access.reset_factor(&user, uid).map_err(failed)?,
                    )
                }
                (None, None) => return Err(Failed::usage("name an account or `--id`")),
            };
            print(
                front,
                &serde_json::json!({"reset": {"account": shown, "had_one": had}}).to_string(),
            )
        }
    }
}

/// `canoa portal passkeys`: an administrator's way to list and remove passkeys, in the files the portal reads.
fn portal_passkeys(
    settings: &crate::portal::Settings,
    action: &PasskeyAction,
    front: &dyn Front,
) -> Result<(), Failed> {
    use crate::portal::{Accounts as _, Method};
    let access = crate::portal::Access::new(settings).map_err(Failed::usage)?;
    if !access.methods().contains(&Method::Passkey) {
        return Err(Failed::usage(
            "this portal keeps no passkeys: name a provider that does (`coqui_access_passkey`) in `authenticators`",
        ));
    }
    // Before anything is written: a file made by another account is one the portal cannot read.
    access.check_passkey_files_owned().map_err(Failed::usage)?;
    match action {
        PasskeyAction::List => {
            for (account, passkeys) in access.list_passkeys().map_err(Failed::usage)? {
                for passkey in passkeys {
                    print(
                        front,
                        &serde_json::json!({
                            "user": account,
                            "id": passkey.id,
                            "label": passkey.label,
                            "added": passkey.made_unix,
                            "last_used": passkey.used_unix,
                            "synced": passkey.synced,
                            "disabled": passkey.disabled,
                        })
                        .to_string(),
                    )?;
                }
            }
            Ok(())
        }
        PasskeyAction::Remove { name, id } => {
            let removed = access
                .remove_passkeys_of(name, id.as_deref())
                .map_err(Failed::run)?;
            if removed == 0 {
                return Err(Failed::run(match id {
                    Some(_) => format!("{name} has no passkey with that id"),
                    None => format!("{name} has no passkeys"),
                }));
            }
            print(
                front,
                &serde_json::json!({"removed": {"account": name, "passkeys": removed}}).to_string(),
            )
        }
    }
}

pub(super) fn portal(args: &PortalArgs, front: &dyn Front) -> Result<(), Failed> {
    use crate::portal::{Portal, Systemd};
    // `config set` and `unset` change the table as it is, so they do not need it to be one the portal would run with.
    if let Some(PortalAction::Config { action }) = &args.action {
        return portal_config(args, action, front);
    }
    let settings = portal_settings(args, front)?;
    settings.check_helper().map_err(Failed::usage)?;
    match &args.action {
        Some(PortalAction::Factor { action }) => return portal_factor(&settings, action, front),
        Some(PortalAction::Passkeys { action }) => {
            return portal_passkeys(&settings, action, front);
        }
        Some(PortalAction::Config { .. }) | None => {}
    }
    let (socket, tcp) = (settings.socket.clone(), settings.listen);
    let accounts = crate::portal::Access::new(&settings).map_err(Failed::usage)?;
    let mut controller = Systemd::new();
    controller.systemctl.clone_from(&settings.systemctl);
    controller.elevate = settings.elevate.iter().map(Into::into).collect();
    controller.unit.clone_from(&settings.unit);
    let portal = Portal::bind(
        settings,
        std::sync::Arc::new(accounts),
        std::sync::Arc::new(controller),
    )
    .map_err(|error| match error {
        crate::portal::PortalError::Listen(_) => Failed::run(error),
        _ => Failed::usage(error),
    })?;
    let line = serde_json::json!({
        "portal": {
            "socket": socket,
            "listen": portal.local_addr().or(tcp).map(|address| address.to_string()),
        }
    });
    print(front, &line.to_string())?;
    portal.run();
    Ok(())
}
