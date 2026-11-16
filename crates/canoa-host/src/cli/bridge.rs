//! The commands that are calls into `coqui-acp-bridge`: this file holds the glue (this host's home, the program that runs its helper
//! subcommands, and how a status ends the process) and nothing the library does.

use std::path::{Path, PathBuf};

use clap::{Args as _, FromArgMatches as _};
use coqui_acp_bridge::helper::Helper;
use coqui_acp_bridge::{
    MethodArgs, dial, discover, forward, grant, keygen, mcp_shim, pairing, serve,
};
use coqui_agentic_peer::auth::{self, Method};
use url::Url;

use super::{Failed, Front, ServeArgs, home_of};

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
    bridge.home = Some(home_dir(front)?);
    bridge.helper = Some(helper(front)?);
    ended(serve::run(&bridge))
}

/// A `serve` with the library's flags and no `--program`: they would be taken for settings of canoa.cfg's host, and are not.
pub(super) fn refuse_program_flags(args: &ServeArgs) -> Result<(), Failed> {
    let defaults =
        serve::Serve::augment_args(clap::Command::new("serve")).get_matches_from(["serve"]);
    let Ok(mut baseline) = serve::Serve::from_arg_matches(&defaults) else {
        return Ok(());
    };
    // What `serve` has of its own, which the library's arguments share.
    baseline.method = args.bridge.method;
    baseline.session = args.bridge.session;
    if format!("{baseline:?}") == format!("{:?}", args.bridge) {
        return Ok(());
    }
    Err(Failed::usage(
        "those flags belong to --program, which serves one program with no canoa.cfg",
    ))
}

pub(super) fn dial(mut args: dial::Dial, front: &dyn Front) -> Result<(), Failed> {
    args.home = Some(home_dir(front)?);
    ended(dial::run(&args))
}

pub(super) fn forward(mut args: forward::Forward, front: &dyn Front) -> Result<(), Failed> {
    args.home = Some(home_dir(front)?);
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

pub(super) fn mcp_shim(peer: Option<&str>) -> Result<(), Failed> {
    ended(mcp_shim::run(peer))
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
    ended(pairing::join(&pairing::PairDial {
        url,
        identity: None,
        store: None,
        pin_file,
        ca_file,
        timeout_seconds,
        method: MethodArgs { method },
        home: Some(home_dir(front)?),
    }))
}
