//! `canoa portal` and `canoa portal factor` CLI handlers.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a command line names what it uses from std"
)]

use std::net::SocketAddr;
use std::path::PathBuf;

use super::commands::print;
use super::{Failed, Front};

/// What `canoa portal` takes.
#[derive(clap::Args, Debug)]
pub(super) struct PortalArgs {
    /// The portal's settings, a TOML file; every key has a default.
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,
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
        #[arg(long, value_name = "CODE")]
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
        #[arg(long, value_name = "ID", conflicts_with = "name")]
        id: Option<String>,
    },
}

/// The settings `canoa portal` runs with: the file, then the command line over it.
fn portal_settings(args: &PortalArgs) -> Result<crate::portal::Settings, Failed> {
    use crate::portal::Settings;
    let mut settings = match &args.config {
        Some(file) => {
            let text = std::fs::read_to_string(file)
                .map_err(|error| Failed::usage(format!("{}: {error}", file.display())))?;
            Settings::from_toml(&text).map_err(Failed::usage)?
        }
        None => Settings::default(),
    };
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

/// `canoa portal factor`: an administrator's way to set up, list and reset one-time codes, in the files the portal reads.
fn portal_factor(
    settings: &crate::portal::Settings,
    action: &FactorAction,
    front: &dyn Front,
) -> Result<(), Failed> {
    use crate::portal::{Accounts as _, FactorError, FactorStanding};
    // Before anything is opened: a code file made by another account is one the portal cannot read.
    crate::portal::check_code_files_owned(settings).map_err(Failed::usage)?;
    let access = crate::portal::Access::new(settings).map_err(Failed::usage)?;
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
                access.clear_reset(&user, uid).map_err(Failed::run)?;
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

pub(super) fn portal(args: &PortalArgs, front: &dyn Front) -> Result<(), Failed> {
    use crate::portal::{Portal, Systemd};
    let settings = portal_settings(args)?;
    if let Some(PortalAction::Factor { action }) = &args.action {
        return portal_factor(&settings, action, front);
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
