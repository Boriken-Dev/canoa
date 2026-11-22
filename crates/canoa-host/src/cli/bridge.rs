//! The commands that are calls into `coqui-acp-bridge`: this file holds the glue (this host's home, the program that runs its helper
//! subcommands, and how a status ends the process) and nothing the library does.

use std::path::{Path, PathBuf};

use clap::Args as _;
use coqui_acp_bridge::helper::Helper;
use coqui_acp_bridge::{
    MethodArgs, dial, discover, forward, grant, keygen, mcp_shim, pairing, serve,
};
use coqui_link::auth::{self, Method};
use url::Url;

use crate::config::Config;
use crate::home::Home;
use coqui_peer_webcrypto::PeerId;

use super::args::{ForceArg, MethodArg, ServeArgs};
use super::commands::layers_of;
use super::{Failed, Front, home_of};

/// How a command of the library ends: a status of its own (an agent's) ends the process with that status and no message, and a
/// message is a usage failure.
fn ended(result: Result<u8, String>) -> Result<(), Failed> {
    match result {
        Ok(0) => Ok(()),
        Ok(status) => Err(Failed {
            status,
            why: String::new(),
        }),
        Err(why) => Err(Failed::usage(why)),
    }
}

/// This host's home directory, which the library's commands keep their identity, trust store and grants in.
fn home_dir(front: &dyn Front) -> Result<PathBuf, Failed> {
    Ok(home_of(front)?.dir().to_owned())
}

/// The program that runs `agent-guard` and `mcp-shim`: the one that runs this host's own commands.
fn helper(front: &dyn Front) -> Result<Helper, Failed> {
    let program = front
        .program()
        .map_err(|error| Failed::run(format!("this program cannot find itself: {error}")))?;
    Ok(Helper::new(
        program.path,
        program
            .args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect(),
    ))
}

/// The keys canoa.cfg limits to entries with `peers`: a key an entry names is that entry's and no program's.
fn limited_keys(
    args: &ServeArgs,
    front: &dyn Front,
) -> Result<std::collections::HashSet<PeerId>, Failed> {
    let layers = layers_of(&args.config, front, false)?;
    let config = match Config::load_layers(&layers) {
        Ok(config) => config,
        Err(crate::ConfigError::Read { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            return Ok(std::collections::HashSet::new());
        }
        // Unreadable is not "limits nobody": a rule that cannot be read is not dropped.
        Err(error) => return Err(Failed::usage(error)),
    };
    Ok(config
        .entries
        .iter()
        .filter_map(|entry| entry.peers.as_ref())
        .flatten()
        .copied()
        .collect())
}

/// `serve --program`: one stdio agent on the network, with no canoa.cfg.
pub(super) fn serve_program(args: &ServeArgs, front: &dyn Front) -> Result<(), Failed> {
    if args.web.is_some()
        || args.web_listen.is_some()
        || args.portal.is_some()
        || args.local_sources
        || args.allow_empty
        || args.supervised
        || args.until_stdin_closes
    {
        return Err(Failed::usage(
            "--program serves one program with no canoa.cfg: --web, --web-listen, --portal, --local-sources, --allow-empty, --supervised and --until-stdin-closes belong to a serve of canoa.cfg",
        ));
    }
    let mut bridge = args.bridge.clone();
    let home = home_of(front)?;
    if bridge.paired {
        method_unless_running(&home, bridge.method.method)?;
    }
    bridge.home = Some(home.dir().to_owned());
    bridge.helper = Some(helper(front)?);
    // A key paired for this host reaches an entry its canoa.cfg limits it to with `peers`, and not a program: whoever is limited
    // is not served this one.
    let limited = limited_keys(args, front)?;
    bridge.admit = Some(serve::Admit::new(move |peer| !limited.contains(peer)));
    ended(serve::run(&bridge))
}

/// The method a command names while a host runs on the home: the same refusal `id` and `pair` give.
fn method_unless_running(home: &Home, method: Option<Method>) -> Result<(), Failed> {
    MethodArg { method }
        .identity_unless_running(home, ForceArg { force: false })
        .map(drop)
}

/// Which of `serve`'s library flags a command line gave, by the way clap knows it: a flag named on the command line, even at its
/// default, and never a guess from what it holds.
#[derive(Clone, Debug, Default)]
pub(super) struct Given(pub(super) Vec<String>);

/// `serve`'s proof settings, which both forms take.
const SHARED: [&str; 3] = ["method", "key_limit", "no_rekey"];

impl clap::Args for Given {
    fn augment_args(command: clap::Command) -> clap::Command {
        command
    }

    fn augment_args_for_update(command: clap::Command) -> clap::Command {
        command
    }
}

impl clap::FromArgMatches for Given {
    fn from_arg_matches(matches: &clap::ArgMatches) -> Result<Self, clap::Error> {
        let known: Vec<String> = serve::Serve::augment_args(clap::Command::new("serve"))
            .get_arguments()
            .map(|argument| argument.get_id().as_str().to_owned())
            .filter(|id| !SHARED.contains(&id.as_str()))
            .collect();
        let given = known
            .iter()
            .filter(|id| matches.try_contains_id(id).unwrap_or(false))
            .filter(|id| matches.value_source(id) == Some(clap::parser::ValueSource::CommandLine))
            .map(|id| format!("--{}", id.replace('_', "-")))
            .collect();
        Ok(Self(given))
    }

    fn update_from_arg_matches(&mut self, matches: &clap::ArgMatches) -> Result<(), clap::Error> {
        *self = Self::from_arg_matches(matches)?;
        Ok(())
    }
}

/// A `serve` with the library's flags and no `--program`: they would be taken for settings of canoa.cfg's host, and are not.
pub(super) fn refuse_program_flags(args: &ServeArgs) -> Result<(), Failed> {
    if args.given.0.is_empty() {
        return Ok(());
    }
    Err(Failed::usage(format!(
        "{} belong to --program, which serves one program with no canoa.cfg",
        args.given.0.join(", ")
    )))
}

/// The keys this host dials, apart from the keys its entries serve: pairing with a server does not let that server in.
fn dialed_store(home: &Path) -> PathBuf {
    home.join("dialed")
}

pub(super) fn dial(mut args: dial::Dial, front: &dyn Front) -> Result<(), Failed> {
    let home = home_of(front)?;
    if args.paired {
        method_unless_running(&home, args.method.method)?;
    }
    args.home = Some(home.dir().to_owned());
    args.paired_store = Some(dialed_store(home.dir()));
    ended(dial::run(&args))
}

pub(super) fn forward(mut args: forward::Forward, front: &dyn Front) -> Result<(), Failed> {
    let home = home_of(front)?;
    if args.paired {
        method_unless_running(&home, args.method.method)?;
    }
    args.home = Some(home.dir().to_owned());
    args.paired_store = Some(dialed_store(home.dir()));
    ended(forward::run(&args))
}

pub(super) fn grant(
    peer: &str,
    protocol: &str,
    revoke: bool,
    front: &dyn Front,
) -> Result<(), Failed> {
    ended(grant::run(Some(&home_dir(front)?), peer, protocol, revoke))
}

pub(super) fn discover(seconds: u64) -> Result<(), Failed> {
    ended(discover::run(seconds))
}

pub(super) fn mcp_config(peer: &str, name: &str, front: &dyn Front) -> Result<String, Failed> {
    mcp_shim::config(&helper(front)?, Some(&home_dir(front)?), peer, name).map_err(Failed::usage)
}

pub(super) fn mcp_shim(peer: Option<&str>, front: &dyn Front) -> Result<(), Failed> {
    ended(mcp_shim::run(peer, Some(&home_dir(front)?)))
}

pub(super) fn agent_guard(pid: u32) {
    coqui_acp_bridge::agent::watch(pid, std::io::stdin().lock());
}

/// `id --identity FILE`: the key in the file, made when absent, and its peer id.
pub(super) fn identity_file(path: &Path, method: Option<Method>) -> Result<String, Failed> {
    if !path.exists() {
        return keygen::create(path, method).map_err(Failed::run);
    }
    let identity = auth::read_identity(path).map_err(Failed::run)?;
    let identity = match method {
        Some(named) => identity.of_method(named).map_err(Failed::run)?,
        None => identity,
    };
    Ok(identity.peer_id().to_string())
}

/// `pair --join URL`: pairs with a waiting `pair` under this home's own identity, and prints the other side's peer id.
pub(super) fn pair_join(
    url: &str,
    pin_file: Option<PathBuf>,
    ca_file: Option<PathBuf>,
    timeout_seconds: u64,
    method: Option<Method>,
    front: &dyn Front,
) -> Result<(), Failed> {
    let url =
        Url::parse(url).map_err(|error| Failed::usage(format!("--join is a URL: {error}")))?;
    let home = home_of(front)?;
    method_unless_running(&home, method)?;
    ended(pairing::join(&pairing::PairDial {
        url,
        identity: None,
        // The server joined is dialed, not served: it is not let in to this host's entries.
        store: Some(dialed_store(home.dir())),
        pin_file,
        ca_file,
        timeout_seconds,
        method: MethodArgs { method },
        home: Some(home.dir().to_owned()),
    }))
}
