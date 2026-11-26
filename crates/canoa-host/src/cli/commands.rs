//! Implementations of `canoa` subcommands (`serve`, `native`, `pair`, `accounts`, `service`, `check`, `options`, etc.).
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a command line names what it uses from std"
)]

use canoa::brand::CONFIG_FILE;
use canoa::brand::PROGRAM;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::args::{
    AccountsCommand, ConfigArgs, ForceArg, Format, MethodArg, NativeAction, NativeBrowser,
    OptionsArgs, ServeArgs, ServiceArgs,
};
use super::{Failed, Front, STATUS_HELD, STATUS_RESTART, bridge, home_of, store};
use crate::accounts::commands::Env as AccountsEnv;
use crate::config::Layers;
use crate::home::{Home, LockError};
use crate::{Config, PeerState, ServeOptions, Serving, WebConfig, WebError, WebServer, peers};

/// The files canoa.cfg is read from. With none named, the person's own is the user's, and a `Canoa.cfg` there with no
/// `canoa.cfg` beside it is renamed first, on the platforms whose file names are case sensitive.
pub(super) fn layers_of(
    args: &ConfigArgs,
    front: &dyn Front,
    must_exist: bool,
) -> Result<Layers, Failed> {
    let named = args.config.as_deref().filter(|value| !value.is_empty());
    if named.is_none()
        && !cfg!(windows)
        && let Ok(user) = anycfg::user_config_path(Some(Path::new(crate::config::layers::NAME)))
    {
        adopt_legacy_name(&user, &|line| front.say(line));
    }
    Layers::resolve(named, must_exist).map_err(Failed::usage)
}

/// Gives the configuration file's name spelled with a capital first letter (`Canoa.cfg`) beside `path` the name `path`, once, and
/// says so on standard error. A file already at `path`, a dangling link included, is never replaced. Returns whether it renamed.
pub(super) fn adopt_legacy_name(path: &Path, say: &dyn Fn(&str)) -> bool {
    let mut letters = CONFIG_FILE.chars();
    let capitalised: String = letters
        .next()
        .into_iter()
        .flat_map(char::to_uppercase)
        .chain(letters)
        .collect();
    let Some(old) = path.parent().map(|dir| dir.join(capitalised)) else {
        return false;
    };
    if path.symlink_metadata().is_ok() || !old.is_file() {
        return false;
    }
    // A hard link fails where the name is taken, which a rename would replace.
    let renamed = std::fs::hard_link(&old, path).and_then(|()| std::fs::remove_file(&old));
    match renamed {
        Ok(()) => {
            say(&format!(
                "{PROGRAM}: renamed {} to {}",
                old.display(),
                path.display()
            ));
            true
        }
        Err(error) => {
            say(&format!(
                "{PROGRAM}: {} was not renamed to {CONFIG_FILE}: {error}",
                old.display()
            ));
            false
        }
    }
}

pub(super) fn load(args: &ConfigArgs, front: &dyn Front) -> Result<Config, Failed> {
    Config::load_layers(&layers_of(args, front, true)?).map_err(Failed::usage)
}

/// canoa.cfg for `serve`. With `--allow-empty`, no file at all names nothing, as an empty one does: a person who has
/// written none yet has a host to run all the same. A host for a portal is always run so: the person's own file is the one
/// their settings are written to, and it does not exist until the first of them.
fn load_for_serving(
    args: &ServeArgs,
    front: &dyn Front,
) -> Result<(Config, crate::Layers), Failed> {
    let lenient = args.allow_empty || args.portal.is_some();
    let layers = layers_of(&args.config, front, !lenient)?;
    let config = match Config::load_layers(&layers) {
        Err(crate::ConfigError::Read { source, .. })
            if lenient && source.kind() == std::io::ErrorKind::NotFound =>
        {
            Config::from_str_in("", layers.own.parent().unwrap_or_else(|| Path::new("")))
                .map_err(Failed::usage)
        }
        loaded => loaded.map_err(Failed::usage),
    }?;
    Ok((config, layers))
}

pub(super) fn print(front: &dyn Front, line: &str) -> Result<(), Failed> {
    front.print(line).map_err(Failed::run)
}

/// `[host.web]` with `--web` and `--web-listen` over it; `None` when neither the file nor the command line asks for it.
fn web_config(
    config: &Config,
    args: &ServeArgs,
    home: &Home,
    front: &dyn Front,
) -> Result<Option<WebConfig>, Failed> {
    if args.portal.is_some() {
        return Ok(None);
    }
    let mut web = config.web.clone();
    if let Some(dir) = &args.web {
        let web = web.get_or_insert_with(WebConfig::new);
        if let Some(dir) = dir {
            web.dir = Some(std::path::absolute(dir).unwrap_or_else(|_| dir.clone()));
        }
    }
    if let Some(listen) = args.web_listen {
        web.as_mut()
            .ok_or_else(|| Failed::usage("--web-listen needs --web or [host.web]"))?
            .listen = listen;
    }
    if let Some(web) = web.as_mut() {
        // The site may hold neither canoa.cfg nor this user's identity and trust store.
        if let Ok(layers) = layers_of(&args.config, front, false) {
            web.keep_out.extend(layers.files.iter().filter_map(|file| {
                std::path::absolute(file)
                    .ok()
                    .and_then(|file| file.parent().map(PathBuf::from))
            }));
        }
        web.keep_out.push(home.dir().to_owned());
    }
    Ok(web)
}

/// `canoa mcp-bridge`: the address and the bearer come from the environment, never from the command line.
pub(super) fn mcp_bridge() -> Result<(), Failed> {
    let var = |name: &str| std::env::var_os(name).and_then(|value| value.into_string().ok());
    let target = crate::delegate::bridge::Target::new(
        var(crate::delegate::bridge::URL_VAR).as_deref(),
        var(crate::delegate::bridge::TOKEN_VAR).as_deref(),
    )
    .map_err(Failed::usage)?;
    crate::delegate::bridge::serve(target, std::io::stdin().lock(), std::io::stdout())
        .map_err(Failed::run)
}

pub(super) fn serve(args: &ServeArgs, front: &dyn Front) -> Result<(), Failed> {
    if args.program {
        if args.bridge.agent.is_empty() {
            return Err(Failed::usage(
                "--program needs the program to serve, after --",
            ));
        }
        return bridge::serve_program(args, front);
    }
    bridge::refuse_program_flags(args)?;
    // Before anything of the home is touched: a host for a portal runs only as the account whose folder it is.
    if let Some(dir) = &args.portal {
        crate::portal_link::check_account(dir, args.portal_min_uid, &args.portal_group)
            .map_err(Failed::usage)?;
    }
    let (config, layers) = load_for_serving(args, front)?;
    crate::addresses::log(&config);
    let home = home_of(front)?;
    let web = web_config(&config, args, &home, front)?;
    // Before anything of the home is touched or any address bound.
    let _lock = home.lock(front.holder()).map_err(|error| match error {
        LockError::Held { .. } => Failed {
            status: STATUS_HELD,
            why: error.to_string(),
        },
        _ => Failed::run(error),
    })?;
    let identity = MethodArg {
        method: args.bridge.method.method,
    }
    .identity(&home)?;
    let peer = identity.peer_id().to_string();
    let program = front
        .program()
        .map_err(|error| Failed::run(format!("this program cannot find itself: {error}")))?;
    let mut options = ServeOptions::new(program.path)
        .with_program_args(program.args)
        .with_rekey(!args.bridge.session.no_rekey)
        .with_nothing_to_serve(args.allow_empty || args.portal.is_some())
        .with_local_sources(args.local_sources || args.portal.is_some())
        .with_supervised(args.supervised)
        .with_layers(layers)
        .with_state_dir(home.dir());
    if let Ok(layers) = layers_of(&args.config, front, false) {
        options = options.with_config_file(layers.own);
    }
    if let Some(bytes) = args.bridge.session.key_limit {
        options = options.with_key_limit(bytes);
    }
    if let Some((programs, providers)) = front.programs() {
        options = options.with_programs(programs, providers);
    }
    if let Some(dir) = &args.portal {
        let user = args
            .portal_user
            .as_deref()
            .ok_or_else(|| Failed::usage("--portal needs --portal-user"))?;
        let uid = crate::portal_link::uid_of(user)
            .map_err(|error| Failed::usage(format!("--portal-user: {error}")))?;
        options = options.with_portal(crate::portal_link::PortalSockets::new(dir, uid));
    }
    if let Ok(layers) = layers_of(&args.config, front, false) {
        options = options.with_account_layers(layers);
    }
    let serving =
        Serving::bind(&config, identity, store(&home)?, &options).map_err(|error| match error {
            crate::ServeError::NothingToServe => Failed::usage(error),
            _ => Failed::run(error),
        })?;
    front.local(serving.local());
    let web = web
        .map(|web| WebServer::bind(&web, &serving))
        .transpose()
        .map_err(|error| match error {
            WebError::Listen { .. } => Failed::run(error),
            _ => Failed::usage(error),
        })?;
    for listening in serving.listening() {
        let mut line = listening.to_json();
        line.insert("peer".into(), peer.clone().into());
        line.insert("method".into(), serving.method().name().into());
        let line = serde_json::Value::Object(line);
        print(front, &line.to_string())?;
    }
    if let Some(web) = &web {
        serving.note_web(&web.local_url(), web.office(), web.office_local());
        print(front, &serde_json::json!({ "web": web.url() }).to_string())?;
    }
    print(
        front,
        &serde_json::json!({
            "ready": true,
            "peer": peer,
            "method": serving.method().name(),
        })
        .to_string(),
    )?;
    if args.until_stdin_closes {
        let stopper = serving.stopper();
        let web_stopper = web.as_ref().map(WebServer::stopper);
        front
            .when_input_closes(
                0,
                Box::new(move || {
                    // The web app is stopped whatever happens to the rest, or `serve` would wait on it for ever.
                    let _ =
                        std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| stopper.stop()));
                    if let Some(web) = web_stopper {
                        web.stop();
                    }
                }),
            )
            .map_err(Failed::run)?;
    }
    let web_stopper = web.as_ref().map(WebServer::stopper);
    let web = web
        .map(|web| {
            std::thread::Builder::new()
                .name("canoa-web".to_owned())
                .spawn(move || web.run())
        })
        .transpose()
        .map_err(Failed::run)?;
    let restart = serving.restart_flag();
    serving.run();
    let restarting = restart.load(Ordering::SeqCst);
    if restarting && let Some(web) = &web_stopper {
        web.stop();
    }
    if let Some(web) = web {
        let _ = web.join();
    }
    if restarting {
        return Err(Failed {
            status: STATUS_RESTART,
            why: "a paired key asked this host to restart".to_owned(),
        });
    }
    Ok(())
}

/// The canoa.cfg layers a `native` command records the launch in: `--config`, else `CANOA_CONFIG`, else the user's own.
fn native_layers(scope: &super::args::NativeScope) -> Result<Layers, Failed> {
    Layers::resolve(
        scope.config.as_deref().filter(|value| !value.is_empty()),
        false,
    )
    .map_err(Failed::usage)
}

/// `canoa native`.
pub(super) fn native(action: NativeAction, front: &dyn Front) -> Result<(), Failed> {
    let home = home_of(front)?;
    match action {
        NativeAction::Install(args) => {
            let program = std::env::current_exe().map_err(Failed::run)?;
            let layers = native_layers(&args.scope)?;
            let launch = match args.launch {
                Some(program) => Some(crate::native::Launch {
                    program,
                    args: args.launch_args,
                }),
                None => front.program().ok().map(|program| crate::native::Launch {
                    program: program.path,
                    args: program
                        .args
                        .iter()
                        .map(|arg| arg.to_string_lossy().into_owned())
                        .chain(["serve".to_owned()])
                        .collect(),
                }),
            };
            let browsers: Vec<_> = args
                .browser
                .iter()
                .map(|browser| match browser {
                    NativeBrowser::Chrome | NativeBrowser::ChromeForTesting => {
                        coqui_agentic_peer::companion::Browser::Chrome
                    }
                    NativeBrowser::Edge => coqui_agentic_peer::companion::Browser::Edge,
                    NativeBrowser::Chromium => coqui_agentic_peer::companion::Browser::Chromium,
                })
                .collect();
            let options = crate::native::Install {
                name: args.scope.name,
                extensions: args.extension_id,
                browsers,
                launch,
                profiles: args.scope.profile,
            };
            for written in
                crate::native::install(&home, &layers, &program, &options).map_err(Failed::run)?
            {
                print(front, &written)?;
            }
            Ok(())
        }
        NativeAction::Uninstall(scope) => crate::native::uninstall(
            &home,
            &native_layers(&scope)?,
            scope.name.as_deref(),
            &scope.profile,
        )
        .map_err(Failed::run),
        NativeAction::Status(scope) => {
            let status = crate::native::status(
                &home,
                &native_layers(&scope)?,
                scope.name.as_deref(),
                &scope.profile,
            )
            .map_err(Failed::run)?;
            print(
                front,
                &serde_json::json!({
                    "name": scope.name.as_deref().unwrap_or(crate::native::HOST_NAME),
                    "registered": status.registered,
                    "launch": status.launch,
                    "running": status.running,
                    "door_error": status.door_error,
                    "holder": status.holder.map(|holder| serde_json::json!({ "name": holder.name, "pid": holder.pid })),
                })
                .to_string(),
            )
        }
    }
}

/// What a browser starts: relays native messages on this process's standard streams to the host of the home.
pub(super) fn native_host(origin: &str, front: &dyn Front) -> Result<(), Failed> {
    let home = home_of(front)?;
    let layers = Layers::resolve(
        std::env::var(canoa::brand::CONFIG)
            .ok()
            .as_deref()
            .filter(|value| !value.is_empty()),
        false,
    )
    .map_err(Failed::usage)?;
    crate::native::relay(
        &home,
        &layers,
        origin,
        &mut std::io::stdin().lock(),
        std::io::stdout(),
        || std::process::exit(0),
    )
    .map_err(Failed::run)
}

/// Runs `door` to its end for `canoa pair`: prints what is shown, then `{"paired"}`.
fn pair_at(
    door: crate::pair_run::Door,
    timeout: Duration,
    until_stdin_closes: bool,
    method: MethodArg,
    force: ForceArg,
    front: &dyn Front,
) -> Result<(), Failed> {
    let home = home_of(front)?;
    let identity = Arc::new(method.identity_unless_running(&home, force)?);
    let store = Arc::new(store(&home)?);
    let cancelled = Arc::new(AtomicBool::new(false));
    if until_stdin_closes {
        let cancel = Arc::clone(&cancelled);
        front
            .when_input_closes(1, Box::new(move || cancel.store(true, Ordering::SeqCst)))
            .map_err(Failed::run)?;
    }
    let run = crate::pair_run::Run {
        identity,
        store,
        home: home.dir().to_owned(),
        timeout,
        cancelled,
    };
    let peer = crate::pair_run::run(door, &run, &mut |shown| {
        front
            .print(&shown.to_string())
            .map_err(|error| error.to_string())
    })
    .map_err(|fail| match fail {
        crate::pair_run::Fail::Usage(why) => Failed::usage(why),
        crate::pair_run::Fail::Run(why) => Failed::run(why),
    })?;
    print(front, &serde_json::json!({ "paired": peer }).to_string())
}

/// `pair --native`: the same PIN pairing, on a pipe in the home that only the extension's native host reaches, for this user.
pub(super) fn pair_native(
    timeout: Duration,
    until_stdin_closes: bool,
    method: MethodArg,
    force: ForceArg,
    front: &dyn Front,
) -> Result<(), Failed> {
    pair_at(
        crate::pair_run::Door::Native,
        timeout,
        until_stdin_closes,
        method,
        force,
        front,
    )
}

pub(super) fn pair(
    listen: SocketAddr,
    timeout: Duration,
    until_stdin_closes: bool,
    origins: Vec<String>,
    method: MethodArg,
    force: ForceArg,
    front: &dyn Front,
) -> Result<(), Failed> {
    if origins.len() > 1
        || origins
            .iter()
            .any(|origin| !coqui_acp_bridge::valid_origin(origin))
    {
        return Err(Failed::usage(
            "--origin is one origin, scheme://host or scheme://host:port with no path, and never null",
        ));
    }
    pair_at(
        crate::pair_run::Door::Network {
            listen,
            origin: origins.into_iter().next(),
        },
        timeout,
        until_stdin_closes,
        method,
        force,
        front,
    )
}

/// canoa.cfg for a command that may make it: a file that does not exist names nothing.
fn account_env<'a>(args: &ConfigArgs, front: &'a dyn Front) -> Result<AccountsEnv<'a>, Failed> {
    let layers = layers_of(args, front, false)?;
    let config_path = layers.own.clone();
    let config = match Config::load_layers(&layers) {
        Err(crate::ConfigError::Read { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            Config::from_str_in("", config_path.parent().unwrap_or_else(|| Path::new("")))
                .map_err(Failed::usage)?
        }
        loaded => loaded.map_err(Failed::usage)?,
    };
    Ok(AccountsEnv {
        front,
        config_path,
        config,
        home: home_of(front)?,
    })
}

pub(super) fn accounts(command: AccountsCommand, front: &dyn Front) -> Result<(), Failed> {
    use crate::accounts::commands as run;
    match command {
        AccountsCommand::List { config } => run::list(&account_env(&config, front)?, config.format),
        AccountsCommand::Providers { config } => {
            run::providers(&account_env(&config, front)?, config.format)
        }
        AccountsCommand::Add {
            name,
            provider,
            method,
            inputs,
            until_stdin_closes,
            config,
        } => run::add(
            &account_env(&config, front)?,
            &name,
            &provider,
            method.as_deref(),
            &run::inputs_of(&inputs)?,
            (config.format, until_stdin_closes),
        ),
        AccountsCommand::Login {
            name,
            method,
            inputs,
            until_stdin_closes,
            config,
        } => run::login(
            &account_env(&config, front)?,
            &name,
            method.as_deref(),
            &run::inputs_of(&inputs)?,
            (config.format, until_stdin_closes),
        ),
        AccountsCommand::Remove { name, config } => {
            run::remove(&account_env(&config, front)?, &name)
        }
    }
}

pub(super) fn service(args: ServiceArgs, front: &dyn Front) -> Result<(), Failed> {
    let ServiceArgs {
        id,
        method,
        params,
        config,
    } = args;
    let env = account_env(&config, front)?;
    let text = if params == "-" {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
            .map_err(|error| Failed::run(error.to_string()))?;
        text
    } else {
        params
    };
    let params: serde_json::Value = if text.trim().is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_str(&text).map_err(|_| Failed::usage("the parameters are not JSON"))?
    };
    let layers = layers_of(&config, front, false)?;
    let host = Arc::new(crate::services::HostServices::new(
        &env.config,
        Some(layers),
        Some(env.home.dir().to_path_buf()),
    ));
    let services = crate::services::Services::new(host);
    let answer = services
        .control(&format!("{id}/{method}"), &params)
        .map_err(|(why, message)| match why {
            crate::services::Failure::NoMethod | crate::services::Failure::BadParams => {
                Failed::usage(message)
            }
            crate::services::Failure::NotFound | crate::services::Failure::Refused => {
                Failed::run(message)
            }
        })?;
    print(front, &answer.to_string())
}

pub(super) fn layers(args: &ConfigArgs, front: &dyn Front) -> Result<(), Failed> {
    let layers = layers_of(args, front, true)?;
    match args.format {
        Format::Json => print(
            front,
            &serde_json::json!({
                "files": layers.files.iter().map(|file| file.display().to_string()).collect::<Vec<_>>(),
                "own": layers.own.display().to_string(),
            })
            .to_string(),
        ),
        Format::Text => {
            for file in &layers.files {
                print(front, &file.display().to_string())?;
            }
            print(front, &format!("own {}", layers.own.display()))
        }
    }
}

/// `control`: one request of the control channel, answered as the owner over canoa.cfg's files through the configuration store.
pub(super) fn control(args: &super::args::ControlArgs, front: &dyn Front) -> Result<(), Failed> {
    let layers = layers_of(&args.config, front, false)?;
    let request = match args.request.strip_prefix('@') {
        Some(file) => std::fs::read_to_string(file)
            .map_err(|error| Failed::usage(format!("{file}: {error}")))?,
        None => args.request.clone(),
    };
    let files = crate::control::Files::new(
        None,
        Vec::new(),
        Some(layers.own.clone()),
        None,
        crate::control::UploadLimits::default(),
    )
    .with_store(Arc::new(crate::config_store::FileStore::of(&layers)));
    let control = crate::control::Control::open(
        None,
        crate::control::DEFAULT_MAX_RECORDS,
        crate::control::DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .map_err(Failed::run)?
    .with_files(files);
    let answer = control
        .handle(&crate::control::Caller::Owner, request.trim().as_bytes())
        .ok_or_else(|| Failed::usage("that is a notification, which has no answer"))?;
    print(front, &String::from_utf8_lossy(&answer))
}

pub(super) fn check(args: &ConfigArgs, front: &dyn Front) -> Result<(), Failed> {
    let config = load(args, front)?;
    for entry in &config.entries {
        let kind = match entry.kind {
            crate::EntryKind::Agent { .. } | crate::EntryKind::Harness { .. } => "agent",
            crate::EntryKind::Provider { .. } => "provider",
            crate::EntryKind::Tool { .. } => "tool",
        };
        let profile = crate::serve::applied_profile(&config, entry);
        let line = match args.format {
            Format::Json => serde_json::json!({
                "name": entry.name,
                "kind": kind,
                "listen": entry.listen.to_string(),
                "profile": profile,
            })
            .to_string(),
            Format::Text => format!("{} {kind} {} profile: {profile}", entry.name, entry.listen),
        };
        print(front, &line)?;
    }
    // An address is never refused for being plain http; the person is told what it costs.
    for note in crate::addresses::warnings(&config) {
        let line = match args.format {
            Format::Json => serde_json::json!({"warning": note}).to_string(),
            Format::Text => format!("non-default: {note}"),
        };
        print(front, &line)?;
    }
    Ok(())
}

pub(super) fn options(args: &OptionsArgs, front: &dyn Front) -> Result<(), Failed> {
    let choices = if args.entry {
        let config = load(&args.config, front)?;
        let mut asking = crate::options::Asking {
            wait: core::time::Duration::from_secs(args.wait),
            any_account: true,
            ..crate::options::Asking::default()
        };
        if let Some(account) = &args.account {
            let home = home_of(front)?;
            let store = crate::accounts::Accounts::open(&config, home.dir())
                .map_err(|error| Failed::run(error.to_string()))?;
            asking.account = Some((account.clone(), store));
        }
        crate::options::entry_with(&config, &args.source, &asking)
            .ok_or_else(|| Failed::usage(format!("{CONFIG_FILE} has no entry of that name")))?
    } else {
        if !crate::config::is_built_in_harness(&args.source) {
            return Err(Failed::usage(
                "SOURCE is the id of a bundled agent, or an entry's name with --entry",
            ));
        }
        crate::options::preset(&args.source, args.executable.as_deref())
    };
    match args.config.format {
        Format::Json => print(front, &choices.to_json().to_string()),
        Format::Text => {
            if let Some(why) = &choices.unknown_because {
                print(front, &format!("unknown: {why}"))?;
            }
            if choices.signed_out {
                print(front, "signed out")?;
            }
            for model in &choices.models {
                print(
                    front,
                    &format!(
                        "{}	{}	{}",
                        model.id,
                        model.name.as_deref().unwrap_or(&model.id),
                        model.efforts.join(",")
                    ),
                )?;
            }
            Ok(())
        }
    }
}

pub(super) fn list_peers(format: Format, front: &dyn Front) -> Result<(), Failed> {
    for peer in peers::list(&store(&home_of(front)?)?).map_err(Failed::run)? {
        let (state, source) = match peer.state {
            PeerState::Accepted => ("accepted", None),
            PeerState::Waiting { source } => ("waiting", source),
        };
        let line = match format {
            Format::Json => serde_json::json!({
                "peer": peer.id.to_string(),
                "state": state,
                "source": source.map(|source| source.to_string()),
            })
            .to_string(),
            Format::Text => match source {
                Some(source) => format!("{state} {} from {source}", peer.id),
                None => format!("{state} {}", peer.id),
            },
        };
        print(front, &line)?;
    }
    Ok(())
}

pub(super) fn peers_failure(error: peers::PeersError) -> Failed {
    match error {
        peers::PeersError::Store(_) => Failed::run(error),
        _ => Failed::usage(error),
    }
}
