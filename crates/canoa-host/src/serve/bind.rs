//! Binding host entries, accounts, control services, native door, and delegation host.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::AtomicBool;
use std::ffi::OsString;
use std::sync::Mutex;

use canoa::{Capability, ClientId, Registry, Service, ServiceId};
use coqui_acp_api::guatiao::library::Registry as GuatiaoRegistry;
use coqui_link::auth::{Identity, Proof, Trust};
use coqui_peer_webcrypto::TrustStore;
use indexmap::IndexMap;

use super::accounts::{
    entry_accounts, harness_settings, keeps_key_in_environment, other_accounts, unfit_accounts,
};
use super::admission::{DelegationHost, page_grants, read_admitted};
use super::connection::unproven_limits;
use super::door::Door;
use super::kept;
use super::{
    ACP_CAPABILITY, Agent, Agents, Bound, Gate, Listening, Run, ServeError, ServeOptions, Served,
    Serving, Shared, entry_profiles, home_from_environment, private_starter, refresh_of,
};
use crate::accounts::Accounts;
use crate::accounts::matching::Catalogs;
use crate::client_tools;
use crate::config::{Config, Entry, EntryKind, ImportSearch, Listen, Origin, Settings};
use crate::control;
use crate::delegate::{self, Delegation};
use crate::history;
use crate::kb_mcp;
use crate::kind;
use crate::pipe;
use crate::portal_link::PortalSockets;
use crate::profile::hosted;
use crate::provider::HostLine;
use crate::scope::Scope;
use crate::secrets;
use crate::sources;
use crate::store::mcp as store_mcp;
use crate::transport;
use crate::usage_splice::Launch;
use crate::worktree::Worktrees;

/// The registries of the libraries and sources canoa.cfg's entries and extensions load, which offer the usage readers beside the
/// bundled agents'.
fn readers_of_libraries(config: &Config) -> Vec<Arc<GuatiaoRegistry>> {
    crate::control::providers::libraries(config)
        .iter()
        .map(|(_, registry)| Arc::clone(registry))
        .collect()
}

/// The arguments that make `program` serve a provider entry on its standard streams. They hold paths and names only: the
/// settings go on its standard input.
pub(super) fn provider_args(
    before: &[OsString],
    entry: &Entry,
    origin: &Origin,
    provider: &str,
    directory: &std::path::Path,
    max_sessions: usize,
) -> Vec<OsString> {
    let (flag, file) = match origin {
        Origin::Library(library) => ("--library", library),
        Origin::Source { file, .. } => ("--source", file),
    };
    let mut args = before.to_vec();
    args.extend([
        OsString::from("provider"),
        OsString::from(flag),
        file.as_os_str().to_owned(),
        OsString::from("--provider"),
        OsString::from(provider),
        OsString::from("--max-sessions"),
        OsString::from(max_sessions.to_string()),
        OsString::from("--directory"),
        directory.as_os_str().to_owned(),
        OsString::from("--settings-on-stdin"),
    ]);
    if let Origin::Source { import_search, .. } = origin {
        args.push(OsString::from("--import-search"));
        args.push(OsString::from(import_search.as_str()));
    }
    for root in &entry.working_roots {
        args.push(OsString::from("--working-root"));
        args.push(root.as_os_str().to_owned());
    }
    args
}

/// The arguments that make `program` serve a tool entry: its name and the entry as JSON, which holds no setting a client
/// gave and nothing the tool's own command line does not show.
fn tool_args(before: &[OsString], name: &str, spec: &crate::tool::ToolSpec) -> Vec<OsString> {
    let mut args = before.to_vec();
    args.extend([
        OsString::from("tool"),
        OsString::from("--name"),
        OsString::from(name),
        OsString::from("--spec"),
        OsString::from(serde_json::to_string(spec).unwrap_or_default()),
    ]);
    args
}

fn portal_of(options: &ServeOptions) -> Option<&PortalSockets> {
    options.portal.as_ref()
}

/// Binds one entry's address or pipe, listing what is not the default of it in the log.
fn bind_listen(
    name: &str,
    table: &str,
    listen: &Listen,
    origins: &[String],
    shared: Option<&PortalSockets>,
) -> Result<(Bound, String), ServeError> {
    match listen {
        Listen::Address(address) => {
            let listener = super::plain_listener(*address, unproven_limits(), origins.to_vec())
                .map_err(|failure| ServeError::Listen {
                    entry: name.to_owned(),
                    failure,
                })?;
            let bound = listener
                .local_addr()
                .map_err(|failure| ServeError::Listen {
                    entry: name.to_owned(),
                    failure,
                })?;
            if listener.plain_beyond_loopback() {
                tracing::info!(
                    "non-default: host.{}.{}.listen is {bound}, default loopback: any machine that reaches it may try the proof",
                    table,
                    name
                );
            }
            Ok((Bound::Socket(listener), super::connection::url_of(bound)))
        }
        Listen::Pipe(pipe_name) => {
            // Every setting is the crate's default, the access control included; a receive sets its own read timeout.
            let bound = match shared {
                Some(sockets) => sockets.bind(pipe_name),
                None => pipe::Listener::bind(pipe_name),
            };
            let listener = bound.map_err(|error| ServeError::Pipe {
                entry: name.to_owned(),
                error,
            })?;
            for setting in listener.non_defaults() {
                tracing::info!(
                    "non-default: host.{}.{}.listen has {} at {}, default {}",
                    table,
                    name,
                    setting.setting,
                    setting.value,
                    setting.default
                );
            }
            Ok((Bound::Pipe(listener), pipe_name.to_string()))
        }
        Listen::Transport(spec) => {
            let with_origins = settings_with_origins(spec, origins, name, table);
            let listening = transport::Listening::start(
                &spec.id,
                spec.origin.as_ref(),
                with_origins.as_ref().or(spec.settings.as_ref()),
                &spec.endpoint,
            )
            .map_err(|what| ServeError::Transport {
                entry: name.to_owned(),
                what,
            })?;
            if let Some(origin) = &spec.origin {
                say_transport_origin(name, table, origin);
            }
            // The provider says what of its settings is not its default: a weakening, such as plain WebSocket beyond
            // loopback, is never silent.
            for (setting, value, default) in listening.non_defaults() {
                tracing::info!(
                    "non-default: host.{table}.{name}.transport has {setting} at {value}, default {default}"
                );
            }
            let address = if listening.address().is_empty() {
                spec.endpoint.clone()
            } else {
                listening.address().to_owned()
            };
            Ok((Bound::Transport(alloc::boxed::Box::new(listening)), address))
        }
    }
}

/// The table of canoa.cfg an entry of this kind is in.
const fn table_of(kind: &EntryKind) -> &'static str {
    match kind {
        EntryKind::Agent { .. } | EntryKind::Harness { .. } => "agents",
        EntryKind::Provider { .. } => "providers",
        EntryKind::Tool { .. } => "tools",
    }
}

/// The settings of a built-in `websocket` transport with `[host] origins` as its `listen.origins`, when the host names origins and
/// the entry's settings list none.
fn settings_with_origins(
    spec: &crate::config::TransportListen,
    origins: &[String],
    name: &str,
    table: &str,
) -> Option<Settings> {
    if origins.is_empty() || spec.id != "websocket" || spec.origin.is_some() {
        return None;
    }
    let mut map = spec
        .settings
        .as_ref()
        .map(|settings| settings.as_map().clone())
        .unwrap_or_default();
    let serde_json::Value::Object(listen) = map
        .entry("listen".to_owned())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
    else {
        return None;
    };
    let listed = match listen.get("origins") {
        None => false,
        Some(serde_json::Value::Array(listed)) => !listed.is_empty(),
        Some(_) => true,
    };
    if listed {
        return None;
    }
    listen.insert(
        "origins".to_owned(),
        serde_json::Value::from(origins.to_vec()),
    );
    tracing::info!(
        "non-default: host.{table}.{name}.transport has listen.origins from [host] origins: {}",
        origins.join(", ")
    );
    Some(Settings::new(map))
}

fn say_origin(entry: &Entry, origin: &Origin) {
    say_origin_of(&entry.name, table_of(&entry.kind), "", origin);
}

fn say_transport_origin(name: &str, table: &str, origin: &Origin) {
    tracing::info!(
        "host.{table}.{name}.transport_{} loads an outside transport provider into the host's process",
        if matches!(origin, Origin::Library(_)) {
            "library"
        } else {
            "source"
        }
    );
    say_origin_of(name, table, "transport_", origin);
}

fn say_origin_of(name: &str, table: &str, prefix: &str, origin: &Origin) {
    let Origin::Source {
        settings,
        import_search,
        import_search_named,
        ..
    } = origin
    else {
        return;
    };
    if cfg!(windows) && *import_search == ImportSearch::Path {
        if *import_search_named {
            tracing::info!(
                "non-default: host.{table}.{name}.{prefix}import_search is path, default library: python3.dll                  and what else the source imports itself may come from the working directory or PATH"
            );
        } else {
            // The file does not say it: import_dirs names no directory, so the host fell back.
            tracing::warn!(
                "host.{table}.{name}.{prefix}import_search is path, because import_dirs names no directory:                  python3.dll and what else the source imports itself may come from the working                  directory or PATH; set import_dirs to where it is, or {prefix}import_search = \"library\""
            );
        }
    }
    if settings
        .as_map()
        .get("isolation")
        .and_then(|value| value.as_str())
        == Some("inherit")
    {
        tracing::info!(
            "non-default: host.{table}.{name}.{prefix}source_settings.isolation is inherit, default shipped"
        );
    }
}

struct BindState<'a> {
    config: &'a Config,
    options: &'a ServeOptions,
    items: &'a crate::profile::ItemSet,
    accounts: Option<&'a Arc<Accounts>>,
    usage_log: Option<Arc<crate::usage_log::UsageLog>>,
    agents: Arc<Mutex<Agents>>,
    library_readers: Option<Vec<Arc<GuatiaoRegistry>>>,
    registry: Registry,
    listeners: Vec<(Arc<Served>, Bound)>,
    local_only: Vec<Arc<Served>>,
    listening: Vec<Listening>,
    movers: BTreeMap<String, Arc<kind::Harness>>,
    missing_of: BTreeMap<&'a str, &'a Vec<String>>,
}

fn bind_proof(
    identity: Identity,
    store: &Arc<TrustStore>,
    options: &ServeOptions,
) -> Result<Proof, ServeError> {
    let mut builder = Proof::builder(identity).initiators(Trust::Store(TrustStore::clone(store)));
    if let Some(bytes) = options.key_limit {
        builder = builder.key_limit(bytes);
    }
    if !options.rekey {
        builder = builder.rekey(false);
    }
    let method = builder
        .build()
        .map_err(|error| ServeError::Proof(error.to_string()))?;
    for setting in method.non_defaults() {
        tracing::info!(
            "non-default: {} is {}, default {}",
            setting.setting,
            setting.value,
            setting.default
        );
    }
    Ok(method)
}

fn bind_accounts_and_boxes(
    config: &Config,
    options: &ServeOptions,
) -> Result<(Option<Arc<Accounts>>, Arc<crate::mail::Mailboxes>), ServeError> {
    let mail_passwords = config
        .mail
        .accounts
        .values()
        .any(|account| account.id.is_some())
        && options.state_dir.is_some();
    let accounts = if config.accounts.is_empty() && !mail_passwords {
        None
    } else {
        let Some(home) = &options.state_dir else {
            return Err(ServeError::Accounts {
                what: "accounts need the host's directory to keep their state in".to_owned(),
            });
        };
        Some(
            Accounts::open(config, home).map_err(|error| ServeError::Accounts {
                what: error.to_string(),
            })?,
        )
    };
    let mail_boxes = Arc::new(crate::mail::Mailboxes::from_config(
        config,
        accounts.clone(),
    ));
    if let (Some(accounts), Some(file)) = (&accounts, &options.config_file) {
        accounts.config_from(file.clone());
    }
    if let Some(accounts) = &accounts {
        accounts.helper_program(crate::cli::Program::new(
            options.provider_program.clone(),
            options.program_args.clone(),
        ));
    }
    Ok((accounts, mail_boxes))
}

#[allow(clippy::too_many_arguments)]
fn bind_harness_entry(
    state: &mut BindState<'_>,
    entry: &Entry,
    harness: &str,
    origin: Option<&Origin>,
    executable: Option<&OsString>,
    asks: &crate::config::HarnessOptions,
    settings: Option<&Settings>,
    account_settings: Option<&Settings>,
    profiles: &[String],
    offered: &[String],
    local_profile: Option<&crate::config::HostProfile>,
) -> Result<(&'static str, Run), ServeError> {
    if let Some(origin) = origin {
        say_origin(entry, origin);
    }
    let refused = |what: String| ServeError::Harness {
        entry: entry.name.clone(),
        what,
    };
    // Each entry has a registry of its own: an id resolves from that entry's file, never another's.
    let registry = Arc::new(kind::registry(origin).map_err(|error| refused(error.to_string()))?);
    let configured = harness_settings(&registry, entry, harness, executable, asks, settings);
    let roots: Vec<String> = entry
        .working_roots
        .iter()
        .map(|root| root.to_string_lossy().into_owned())
        .collect();
    let build = |settings: &Settings| {
        kind::Harness::new(
            &registry,
            harness,
            (settings, &account_settings.cloned().unwrap_or_default()),
            // An agent whose tools a session may serve from an environment takes the host's plugin of the hook that turns them there.
            !profiles.is_empty()
                || local_profile.is_some()
                || (!state.config.environments.is_empty()
                    && crate::bundled::does_own_tool_work(harness)),
            roots.clone(),
            crate::makers::Makers::of(state.config),
        )
        .map(|built| {
            built
                .with_lifetime(asks.cache_lifetime)
                .with_entry(&entry.name)
        })
        .map_err(|error| refused(error.to_string()))
    };
    let mut provider = build(&configured)?;
    let listed = match state.accounts {
        Some(_) => entry_accounts(state.config, harness, offered, &provider).map_err(refused)?,
        None => None,
    };
    if let Some(listed) = &listed
        && kind::declares_setting(&registry, harness, "setting_sources")
        && keeps_key_in_environment(
            &crate::makers::Makers::of(state.config),
            listed,
            (harness, provider.catalog()),
        )
    {
        let named = asks.setting_sources.is_some()
            || settings.is_some_and(|own| own.as_map().contains_key("setting_sources"));
        if named {
            tracing::warn!(
                "host.agents.{}.setting_sources is set and the entry offers an account whose key is in the environment: a repository's settings can change the address the key is sent to",
                entry.name
            );
        } else {
            let mut map = configured.as_map().clone();
            map.insert("setting_sources".to_owned(), serde_json::json!([]));
            provider = build(&Settings::new(map))?;
        }
    }
    for (setting, value, default) in provider.non_defaults() {
        if matches!(
            setting.as_str(),
            "allow_bypass" | "allow_client_access" | "modes" | "access"
        ) && !settings.is_some_and(|own| own.as_map().contains_key(setting.as_str()))
        {
            continue;
        }
        tracing::info!(
            "non-default: host.agents.{}: {setting} is {value}, default {default}",
            entry.name
        );
    }
    if let Some(host) = state.accounts {
        let listed = listed.unwrap_or_default();
        let others = other_accounts(state.config, harness, &listed, &provider);
        if !listed.is_empty() || !others.is_empty() {
            let unfit = unfit_accounts(state.config, harness, &provider);
            let pools = match provider.catalog() {
                Some(_) => crate::pools::for_entry(
                    state.config,
                    &Catalogs::with(state.config, harness, &provider),
                    harness,
                    offered,
                )
                .map_err(refused)?,
                None => Vec::new(),
            };
            let default_pool = offered
                .first()
                .filter(|first| crate::pools::is_pool(state.config, first))
                .cloned();
            provider = provider
                .offering(host, listed, others)
                .with_unfit(unfit)
                .with_pools(pools, default_pool)
                .with_config(Arc::new(state.config.clone()));
        }
    }
    if let Some(setting) = entry.shell.as_deref().or(state.config.shell.as_deref()) {
        match crate::shells::agent_applied(setting, harness, &crate::shells::Lookup::here()) {
            Ok((shell, applied)) => {
                if let Some(words) = &applied.notice {
                    tracing::info!(
                        "host.agents.{}: shell {} is not passed: {words}",
                        entry.name,
                        shell.id
                    );
                }
                provider = provider.with_shell_env(applied.env);
            }
            Err(why) if entry.shell.is_some() => return Err(refused(why)),
            Err(why) => {
                tracing::warn!("host.agents.{}: [host] shell: {why}", entry.name)
            }
        }
    }
    let provider = Arc::new(provider);
    state
        .movers
        .insert(entry.name.clone(), Arc::clone(&provider));
    Ok((
        "agent",
        Run::Agent {
            agent: Agent::Harness(provider),
            scope: Scope::new(entry.working_roots.clone())
                .account(entry.name.clone())
                .limit_phrases(state.config.limit_phrases.clone())
                .usage_log(state.usage_log.clone(), harness),
            offer: hosted::offer(
                profiles,
                &state.config.profiles,
                local_profile,
                state.options.provider_program.as_os_str().to_owned(),
                state.options.program_args.clone(),
            ),
            refresh: refresh_of(entry, profiles, state.config, state.options, state.items)
                .map(Box::new),
            worktrees: Worktrees::new(entry.working_roots.clone(), entry.worktrees)
                .with_limits(state.config.worktrees),
        },
    ))
}

fn bind_entry(state: &mut BindState<'_>, entry: &Entry) -> Result<(), ServeError> {
    let (listener, url) = if entry.local {
        (None, format!("local://{}", entry.name))
    } else {
        let (bound, url) = bind_listen(
            &entry.name,
            table_of(&entry.kind),
            &entry.listen,
            &state.config.origins,
            portal_of(state.options),
        )?;
        (Some(bound), url)
    };
    let local_profile = (entry.local || entry_profiles(entry).is_empty())
        .then(|| state.config.local_profiles.for_agent(&entry.name))
        .flatten()
        .map(|mut each| {
            each.items = state.items.clone();
            each
        });
    let (kind, run) = match &entry.kind {
        EntryKind::Agent {
            program,
            args,
            profiles,
            usage,
        } => {
            let harness = std::path::Path::new(program)
                .file_stem()
                .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
            let scope = Scope::new(entry.working_roots.clone())
                .account(entry.name.clone())
                .limit_phrases(state.config.limit_phrases.clone())
                .usage_log(state.usage_log.clone(), &harness);
            let scope = match usage.as_deref() {
                Some(id) => {
                    let config = state.config;
                    let reader = crate::usage_splice::Reader::named(id)
                        .or_else(|| {
                            crate::usage_splice::Reader::offered_by(
                                id,
                                state
                                    .library_readers
                                    .get_or_insert_with(|| readers_of_libraries(config)),
                            )
                        })
                        .ok_or_else(|| ServeError::Usage {
                            entry: entry.name.clone(),
                            what: format!(
                                "no usage reader {id} is offered by a bundled agent or by a library or source of canoa.cfg"
                            ),
                        })?;
                    scope.quota(Launch {
                        reader,
                        program: program.clone(),
                        args: args.clone(),
                        state_dir: state.options.state_dir.clone(),
                        private: profiles.is_empty() && local_profile.is_none(),
                        per_connection: !profiles.is_empty() || local_profile.is_some(),
                        starter: Some(private_starter(
                            &state.agents,
                            state.config.max_message_bytes,
                        )),
                    })
                }
                None => scope,
            };
            (
                "agent",
                Run::Agent {
                    agent: Agent::Program {
                        program: program.clone(),
                        args: args.clone(),
                    },
                    scope,
                    offer: hosted::offer(
                        profiles,
                        &state.config.profiles,
                        local_profile.as_ref(),
                        state.options.provider_program.as_os_str().to_owned(),
                        state.options.program_args.clone(),
                    ),
                    refresh: refresh_of(entry, profiles, state.config, state.options, state.items)
                        .map(Box::new),
                    worktrees: Worktrees::new(entry.working_roots.clone(), entry.worktrees)
                        .with_limits(state.config.worktrees),
                },
            )
        }
        EntryKind::Provider {
            origin,
            provider,
            settings,
            directory,
            max_sessions,
        } => {
            let mut line = HostLine::default();
            line.settings.clone_from(settings);
            say_origin(entry, origin);
            if let Origin::Source { settings, .. } = origin {
                line.source_settings = Some(settings.clone());
            }
            (
                "provider",
                Run::Provider {
                    program: state.options.provider_program.as_os_str().to_owned(),
                    args: provider_args(
                        &state.options.program_args,
                        entry,
                        origin,
                        provider,
                        directory,
                        *max_sessions,
                    ),
                    line,
                },
            )
        }
        EntryKind::Harness {
            harness,
            origin,
            executable,
            options: asks,
            settings,
            account_settings,
            profiles,
            accounts: offered,
        } => bind_harness_entry(
            state,
            entry,
            harness,
            origin.as_ref(),
            executable.as_ref(),
            asks,
            settings.as_ref(),
            account_settings.as_ref(),
            profiles,
            offered,
            local_profile.as_ref(),
        )?,
        EntryKind::Tool { spec } => (
            "tool",
            Run::Agent {
                agent: Agent::Program {
                    program: state.options.provider_program.as_os_str().to_owned(),
                    args: tool_args(&state.options.program_args, &entry.name, spec),
                },
                scope: Scope::new(entry.working_roots.clone())
                    .account(entry.name.clone())
                    .limit_phrases(state.config.limit_phrases.clone()),
                offer: None,
                refresh: None,
                worktrees: Worktrees::new(entry.working_roots.clone(), entry.worktrees)
                    .with_limits(state.config.worktrees),
            },
        ),
    };
    let id = ServiceId::new(entry.name.clone());
    state
        .registry
        .register(Service::new(id.clone(), entry.name.clone(), "acp", url.clone()).verified());
    for peer in entry.peers.iter().flatten() {
        state.registry.grant(
            ClientId::new(peer.to_string()),
            id.clone(),
            Capability::new(ACP_CAPABILITY),
        );
    }
    let missing = if entry.local {
        state
            .missing_of
            .get(entry.name.as_str())
            .map(|missing| (*missing).clone())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    state.listening.push(Listening {
        name: entry.name.clone(),
        kind,
        url,
        harness: match &entry.kind {
            EntryKind::Harness { harness, .. } => Some(harness.clone()),
            _ => None,
        },
        transport: match &entry.listen {
            Listen::Transport(spec) => Some(spec.id.clone()),
            _ => None,
        },
        title: entry.title.clone(),
        local: entry.local,
        missing,
    });
    let served = Arc::new(Served {
        name: entry.name.clone(),
        restricted: entry.peers.is_some(),
        delegable: kind == "agent",
        run,
        running: Gate::new(entry.max_peers),
        keep: entry.shared.then_some(super::sessions::Keep {
            idle: entry.shared_idle,
            permission_window: entry.shared_permission_window,
            keep_log: entry.shared_keep_log,
            log_entries: entry.shared_log_entries,
            log_bytes: entry.shared_log_bytes,
        }),
        environment: entry.environment.clone(),
    });
    match listener {
        Some(listener) => state.listeners.push((served, listener)),
        None => state.local_only.push(served),
    }
    Ok(())
}

fn bind_control_entry(state: &mut BindState<'_>) -> Result<(), ServeError> {
    let Some(settings) = &state.config.control else {
        return Ok(());
    };
    let Some(listen) = &settings.listen else {
        return Ok(());
    };
    let (bound, url) = bind_listen(
        control::NAME,
        "control",
        listen,
        &state.config.origins,
        portal_of(state.options),
    )?;
    let id = ServiceId::new(control::NAME.to_owned());
    state.registry.register(
        Service::new(id.clone(), control::NAME.to_owned(), "control", url.clone()).verified(),
    );
    for peer in settings.grants.keys() {
        state.registry.grant(
            ClientId::new(peer.to_string()),
            id.clone(),
            Capability::new(ACP_CAPABILITY),
        );
    }
    state.listening.push(Listening {
        name: control::NAME.to_owned(),
        kind: "control",
        url,
        harness: None,
        transport: None,
        title: None,
        local: false,
        missing: Vec::new(),
    });
    state.listeners.push((
        Arc::new(Served {
            name: control::NAME.to_owned(),
            restricted: portal_of(state.options).is_none(),
            delegable: false,
            run: Run::Control {
                grants: settings.grants.clone(),
                everyone: portal_of(state.options).is_some().then(|| {
                    control::Grant::all()
                        .into_iter()
                        .filter(|grant| *grant != control::Grant::Discover)
                        .collect()
                }),
            },
            running: Gate::new(crate::config::DEFAULT_MAX_PEERS),
            keep: None,
            environment: None,
        }),
        bound,
    ));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_control_hub(
    config: &Config,
    options: &ServeOptions,
    stopped: &Arc<AtomicBool>,
    trust: &Arc<TrustStore>,
    hosted: &Arc<super::sessions::Hosted>,
    pairing_identity: &Arc<Identity>,
    usage_log: Option<&Arc<crate::usage_log::UsageLog>>,
    secrets: Arc<secrets::Hub>,
    recorded: Arc<history::Store>,
    movers: BTreeMap<String, Arc<kind::Harness>>,
    mail_boxes: Arc<crate::mail::Mailboxes>,
    environments: &Arc<crate::environments::Environments>,
) -> Result<control::Control, ServeError> {
    let kept = control::Control::open(
        options
            .state_dir
            .as_ref()
            .map(|dir| dir.join("control"))
            .as_deref(),
        config
            .control
            .as_ref()
            .map_or(control::DEFAULT_MAX_RECORDS, |c| c.max_records),
        config
            .control
            .as_ref()
            .map_or(control::DEFAULT_MAX_RECORD_BYTES, |c| c.max_record_bytes),
        config
            .entries
            .iter()
            .filter(|entry| entry.worktrees && !entry.working_roots.is_empty())
            .map(|entry| control::EntryRoots {
                peers: entry
                    .peers
                    .as_ref()
                    .map(|peers| peers.iter().map(ToString::to_string).collect()),
                roots: entry.working_roots.clone(),
            })
            .collect(),
    )
    .map_err(ServeError::Control)?;
    let kept = if options.supervised {
        kept.with_stop(Arc::clone(stopped))
    } else {
        kept
    };
    let kept = kept
        .with_index(crate::store::index::StoreIndex::new(
            options
                .state_dir
                .as_ref()
                .map(|dir| dir.join("cache").join("store")),
            config.store.index_ttl.map_or(
                crate::store::index::DEFAULT_TTL,
                core::time::Duration::from_secs,
            ),
            config
                .store
                .index_cache_bytes
                .unwrap_or(crate::store::index::DEFAULT_CACHE_BYTES),
        ))
        .with_registry(control::Registry::new(
            options
                .state_dir
                .as_ref()
                .map(|dir| dir.join("cache").join("contents")),
        ))
        .with_overlays(
            control::Overlays::new(
                options
                    .config_file
                    .as_ref()
                    .and_then(|file| file.parent())
                    .map(|dir| dir.join("overlays")),
                control::OverlayLimits::default(),
            )
            .with_skill_folders(config.store.skill_folders.clone()),
        )
        .with_extensions(control::ExtensionInstall::new(
            options.state_dir.as_ref().map(|dir| dir.join("extensions")),
        ))
        .with_terminals(
            crate::terminal::Terminals::new(
                config.shell.clone(),
                options.home_dir.clone().or_else(home_from_environment),
            )
            .in_environments(Arc::clone(environments)),
        )
        .with_shells(crate::shells::Plan::new(
            config.shell.clone(),
            config
                .entries
                .iter()
                .filter_map(|entry| match &entry.kind {
                    EntryKind::Harness { harness, .. } => Some(crate::shells::AgentShell {
                        entry: entry.name.clone(),
                        harness: harness.clone(),
                        own: entry.shell.clone(),
                    }),
                    _ => None,
                })
                .collect(),
        ))
        .with_files(
            control::Files::new(
                options.home_dir.clone().or_else(home_from_environment),
                config
                    .entries
                    .iter()
                    .filter(|entry| !entry.working_roots.is_empty())
                    .map(|entry| control::EntryRoots {
                        peers: entry
                            .peers
                            .as_ref()
                            .map(|peers| peers.iter().map(ToString::to_string).collect()),
                        roots: entry.working_roots.clone(),
                    })
                    .collect(),
                options.config_file.clone(),
                options
                    .state_dir
                    .as_ref()
                    .map(|dir| dir.join("control").join("uploads")),
                control::UploadLimits {
                    file_bytes: config
                        .control
                        .as_ref()
                        .map_or(control::DEFAULT_MAX_UPLOAD_BYTES, |c| c.max_upload_bytes),
                    total_bytes: config
                        .control
                        .as_ref()
                        .map_or(control::DEFAULT_MAX_UPLOAD_TOTAL, |c| {
                            c.max_upload_total_bytes
                        }),
                    files: config
                        .control
                        .as_ref()
                        .map_or(control::DEFAULT_MAX_UPLOADS, |c| c.max_uploads),
                },
            )
            .with_openers(config.openers.clone())
            .with_protected(config.protected_paths.clone()),
        );
    let kept = kept
        .with_worktree_limits(config.worktrees)
        .with_trust(Arc::clone(trust))
        .with_sessions(control::RunningSessions(Arc::new({
            let hosted = Arc::clone(hosted);
            move |who| hosted.running(who)
        })));
    let kept = match &options.state_dir {
        Some(home) => kept.with_pairing(control::hosting::Pairing {
            identity: Arc::clone(pairing_identity),
            home: home.clone(),
        }),
        None => kept,
    };
    let kept = match usage_log {
        Some(log) => kept.with_usage(Arc::clone(log)),
        None => kept,
    };
    let kept = kept
        .with_sealer(control::Sealer::new(Arc::clone(&secrets), options.ask_wait))
        .with_history(control::Moves {
            store: recorded,
            targets: movers,
            homes: control::Homes::of_this_computer(),
        });
    let kept = match (&options.account_layers, &options.state_dir) {
        (Some(layers), Some(home)) => kept.with_accounts(control::accounts::Accounts::new(
            layers.clone(),
            home.clone(),
            secrets,
            options.ask_wait,
            portal_of(options).is_some(),
        )),
        _ => kept,
    };
    let kept = match (&options.account_layers, &options.state_dir) {
        (Some(layers), Some(home)) => kept.with_mail(control::mail_setup::MailSetup::new(
            layers.clone(),
            home.clone(),
            mail_boxes,
        )),
        _ => kept,
    };
    Ok(match &options.programs {
        Some((programs, providers)) => kept.with_stores(
            control::Stores::new(Arc::clone(programs), providers.clone())
                .extended(&config.extensions),
        ),
        None if !config.extensions.is_empty() => match control::Stores::built_in() {
            Ok(stores) => kept.with_stores(stores.extended(&config.extensions)),
            Err(_) => kept,
        },
        None => kept,
    })
}

impl Serving {
    /// Binds every entry of `config` on its loopback address, proving peers with `identity`, by its method, against `store`.
    /// Nothing is accepted until [`Serving::run`].
    ///
    /// # Errors
    ///
    /// [`ServeError::NothingToServe`] for a configuration with no entry, unless the options allow it;
    /// [`ServeError::Listen`] for an address that cannot be bound.
    pub fn bind(
        config: &Config,
        identity: Identity,
        store: TrustStore,
        options: &ServeOptions,
    ) -> Result<Self, ServeError> {
        crate::network::install(&config.network).map_err(|error| ServeError::Network {
            what: error.to_string(),
        })?;
        // The agents the harness starts in this process are found on this process's PATH and in the Store's runtimes.
        coqui_acp_harness::set_program_dirs(crate::sources::runtime_dirs);
        let adapted;
        let config = match &options.portal {
            Some(sockets) => {
                adapted = crate::portal_link::adapt(config, sockets)?;
                &adapted
            }
            None => config,
        };
        if let Some(home) = &options.state_dir {
            crate::scripts::use_home(home);
        }
        let items = crate::profile::ItemSet::of(config);
        let mut sources = if options.local_sources {
            sources::entries(config)
        } else {
            Vec::new()
        };
        if let Some(sockets) = portal_of(options) {
            for source in &mut sources {
                source.entry.local = false;
                source.entry.listen = Listen::Pipe(sockets.entry(&source.entry.name)?);
            }
        }
        let controlled = config
            .control
            .as_ref()
            .is_some_and(|control| control.listen.is_some());
        if config.entries.is_empty()
            && sources.is_empty()
            && !controlled
            && !options.nothing_to_serve
        {
            return Err(ServeError::NothingToServe);
        }
        let store = Arc::new(store);
        let dialer = identity.clone();
        let pairing_identity = Arc::new(identity.clone());
        let secrets = secrets::Hub::new(
            &identity,
            &store,
            delegate::BridgeCommand::new(&options.provider_program, &options.program_args),
        );
        secrets.vault().configure(
            secrets::catalog::Catalog::of(config, options.state_dir.as_deref()),
            secrets::settings::source_of(options.config_file.clone()),
        );
        if let Some(home) = &options.state_dir {
            secrets.vault().set_home(home);
        }
        let method = bind_proof(identity, &store, options)?;
        let (accounts, mail_boxes) = bind_accounts_and_boxes(config, options)?;
        let usage_log = options.state_dir.as_ref().map(|home| {
            Arc::new(crate::usage_log::UsageLog::open(
                crate::usage_log::dir_of(home),
                config.usage.keep_days,
                crate::usage_splice::unix_now(),
            ))
        });
        let agents = Arc::new(Mutex::new(Agents::default()));
        let missing_of: BTreeMap<&str, &Vec<String>> = sources
            .iter()
            .map(|source| (source.entry.name.as_str(), &source.missing))
            .collect();
        let mut state = BindState {
            config,
            options,
            items: &items,
            accounts: accounts.as_ref(),
            usage_log: usage_log.clone(),
            agents: Arc::clone(&agents),
            library_readers: None,
            registry: Registry::new(),
            listeners: Vec::new(),
            local_only: Vec::new(),
            listening: Vec::new(),
            movers: BTreeMap::new(),
            missing_of,
        };
        for entry in config
            .entries
            .iter()
            .chain(sources.iter().map(|source| &source.entry))
        {
            bind_entry(&mut state, entry)?;
        }
        bind_control_entry(&mut state)?;
        let delegation = Delegation::start(
            &config.delegate,
            delegate::BridgeCommand::new(&options.provider_program, &options.program_args),
        );
        let recorded = Arc::new(history::Store::open(
            options.state_dir.as_ref().map(|dir| dir.join("history")),
            config.history,
        ));
        if config.keep_awake {
            tracing::info!("non-default: settings.power.keep_awake is true");
        }
        if config.max_message_bytes != crate::config::DEFAULT_MAX_MESSAGE_BYTES {
            tracing::info!(
                "non-default: host.max_message_bytes is {}, default {}",
                config.max_message_bytes,
                crate::config::DEFAULT_MAX_MESSAGE_BYTES
            );
        }
        let stopped = Arc::new(AtomicBool::new(false));
        let trust = Arc::clone(&store);
        let kept = options
            .state_dir
            .as_ref()
            .map(|dir| Arc::new(kept::Kept::new(dir.join("shared_sessions"))));
        if let Some(kept) = &kept {
            kept.sweep(
                &config
                    .entries
                    .iter()
                    .chain(sources.iter().map(|source| &source.entry))
                    .filter(|entry| entry.shared && entry.shared_keep_log)
                    .map(|entry| (entry.name.clone(), entry.shared_idle))
                    .collect(),
            );
        }
        let hosted = Arc::new(super::sessions::Hosted::new(method.peer_id(), kept));
        let office_dir = options.state_dir.as_ref().map(|dir| dir.join("office"));
        let environments = crate::environments::Environments::of(config);
        let control = build_control_hub(
            config,
            options,
            &stopped,
            &trust,
            &hosted,
            &pairing_identity,
            usage_log.as_ref(),
            Arc::clone(&secrets),
            recorded.clone(),
            state.movers,
            Arc::clone(&mail_boxes),
            &environments,
        )?;
        let mut serving = Self {
            shared: Arc::new(Shared {
                history: recorded,
                secrets,
                store_tools: store_mcp::Hub::new(
                    config.store.tools.unwrap_or(true),
                    delegate::BridgeCommand::new(&options.provider_program, &options.program_args),
                ),
                kb_tools: kb_mcp::Hub::new(
                    &config.kb,
                    options.state_dir.clone(),
                    delegate::BridgeCommand::new(&options.provider_program, &options.program_args),
                ),
                env_tools: crate::environments::mcp::Hub::new(
                    Arc::clone(&environments),
                    delegate::BridgeCommand::new(&options.provider_program, &options.program_args),
                    config.protected_paths.as_deref(),
                ),
                client_tools: client_tools::Hub::new(delegate::BridgeCommand::new(
                    &options.provider_program,
                    &options.program_args,
                )),
                hosted,
                max_line: config.max_message_bytes,
                mcp_list: config.mcp_list,
                keep_awake: crate::awake::KeepAwake::new(config.keep_awake),
                control_grants: config
                    .control
                    .as_ref()
                    .map(|settings| settings.grants.clone())
                    .unwrap_or_default(),
                control_everyone: portal_of(options).is_some().then(|| {
                    control::Grant::all()
                        .into_iter()
                        .filter(|grant| {
                            !matches!(
                                grant,
                                control::Grant::Discover
                                    | control::Grant::Mail
                                    | control::Grant::MailSetup
                            )
                        })
                        .collect()
                }),
                admitted: Arc::new(Mutex::new(match (&office_dir, &config.web) {
                    (Some(dir), Some(web)) if web.office && web.office_local => {
                        read_admitted(&dir.join("admitted.json"), web.office_secret, &store)
                    }
                    _ => BTreeSet::new(),
                })),
                page_grants: page_grants(config),
                office_dir,
                method,
                store,
                registry: state.registry,
                unproven: Gate::new(super::MAX_UNPROVEN),
                agents,
                ending: Mutex::new(()),
                stopped,
                program_args: options.program_args.clone(),
                launcher: options.provider_program.as_os_str().to_owned(),
                portal_uid: options.portal.as_ref().map(|sockets| sockets.uid),
                control: Arc::new(control),
                delegation,
            }),
            listeners: state.listeners,
            local_only: state.local_only,
            listening: state.listening,
            door: None,
        };
        serving
            .shared
            .store_tools
            .serve_with(Arc::new(control::ToolsBackend(Arc::clone(
                &serving.shared.control,
            ))));
        if config.mail.tools != Some(false) {
            serving
                .shared
                .store_tools
                .serve_extra(Arc::new(crate::mail::tools::MailTools::new(mail_boxes)));
        }
        serving.door = serving.open_door(config, options);
        serving.attach_delegation(config, dialer, usage_log);

        if let Some(sockets) = &options.portal {
            let peer = serving.shared.method.peer_id().to_string();
            let method_name = serving.method().name();
            let index = crate::portal_link::index_of(&peer, method_name, &serving.listening);
            crate::portal_link::introduce(
                sockets,
                Arc::clone(&serving.shared.store),
                index,
                Arc::clone(&serving.shared.stopped),
            )?;
        }
        Ok(serving)
    }

    /// Binds the native door in the home, when the configuration and the options leave one. A door that cannot be bound is
    /// logged and left out: the host serves its entries without it.
    fn open_door(&self, config: &Config, options: &ServeOptions) -> Option<Door> {
        let home = options.state_dir.as_ref()?;
        if !config.native.enabled || options.portal.is_some() {
            return None;
        }
        let name = match crate::native::door_name(home) {
            Ok(name) => name,
            Err(error) => {
                tracing::warn!(
                    "the native door has no name, so a browser cannot reach this host: {error}"
                );
                crate::native::note_door(
                    home,
                    Some(&format!("the door for browsers has no name: {error}")),
                );
                return None;
            }
        };
        let listener = match pipe::Listener::bind(&name) {
            Ok(listener) => listener,
            Err(error) => {
                tracing::warn!(
                    "the native door could not be opened, so a browser cannot reach this host: {error}"
                );
                crate::native::note_door(
                    home,
                    Some(&format!(
                        "the door for browsers could not be opened: {error}"
                    )),
                );
                return None;
            }
        };
        crate::native::note_door(home, None);
        let entries: Vec<serde_json::Value> = self
            .listening
            .iter()
            .filter(|listening| listening.kind != "control")
            .map(|listening| {
                serde_json::json!({
                    "name": listening.name,
                    "label": listening.title.clone().unwrap_or_else(|| listening.name.clone()),
                    "kind": listening.kind,
                    "local": listening.local,
                })
            })
            .collect();
        let mut hello = serde_json::json!({
            "name": "Canoa",
            "version": env!("CARGO_PKG_VERSION"),
            "peer": self.peer().to_string(),
            "method": self.method().name(),
            "entries": entries,
            "control": true,
        });
        if let (Some(claim), Some(web)) = (
            hello.as_object_mut(),
            config.web.as_ref().and_then(super::door::web_origin),
        ) {
            claim.insert("web".to_owned(), web.into());
        }
        if let (Some(claim), Some(names)) = (
            hello.as_object_mut(),
            crate::native::this_computer().as_object(),
        ) {
            claim.extend(
                names
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
        }
        let control = Arc::new(Served {
            name: control::NAME.to_owned(),
            restricted: false,
            delegable: false,
            run: Run::Control {
                grants: IndexMap::new(),
                everyone: Some(config.native.control.clone()),
            },
            running: Gate::new(crate::config::DEFAULT_MAX_PEERS),
            keep: None,
            environment: None,
        });
        let tools = Arc::new(Served {
            name: client_tools::NAME.to_owned(),
            restricted: false,
            delegable: false,
            run: Run::Tools {
                grants: config.native.control.clone(),
            },
            running: Gate::new(crate::config::DEFAULT_MAX_PEERS),
            keep: None,
            environment: None,
        });
        Some(Door::new(
            listener,
            hello,
            self.local().served,
            (control, tools),
            home.clone(),
        ))
    }

    /// Gives the delegation the agents this host serves and the way to open one for a task.
    fn attach_delegation(
        &self,
        config: &Config,
        identity: Identity,
        usage: Option<Arc<crate::usage_log::UsageLog>>,
    ) {
        let Some(delegation) = &self.shared.delegation else {
            return;
        };
        let served = self.local().served;
        let mut agents: Vec<delegate::Source> = self
            .listening
            .iter()
            .filter(|listening| listening.kind == "agent")
            .map(|listening| delegate::Source {
                name: listening.name.clone(),
                title: listening.title.clone(),
                harness: listening.harness.clone(),
                roots: served
                    .iter()
                    .find(|served| served.name == listening.name)
                    .and_then(|served| match &served.run {
                        Run::Agent { scope, .. } => Some(scope.roots().to_vec()),
                        _ => None,
                    })
                    .unwrap_or_default(),
                accounts: match config.entry(&listening.name).map(|entry| &entry.kind) {
                    Some(EntryKind::Harness { accounts, .. }) => accounts.clone(),
                    _ => Vec::new(),
                },
                profiles: match config.entry(&listening.name).map(|entry| &entry.kind) {
                    Some(
                        EntryKind::Agent { profiles, .. } | EntryKind::Harness { profiles, .. },
                    ) => profiles
                        .iter()
                        .map(|name| {
                            let loadouts = config
                                .profiles
                                .get(name)
                                .map(|profile| profile.loadouts.keys().cloned().collect())
                                .unwrap_or_default();
                            (name.clone(), loadouts)
                        })
                        .collect(),
                    _ => Vec::new(),
                },
                missing: listening.missing.clone(),
                remote: None,
            })
            .collect();
        agents.extend(config.remote_agents.iter().map(|agent| delegate::Source {
            name: agent.name.clone(),
            title: agent.title.clone(),
            harness: None,
            roots: Vec::new(),
            accounts: Vec::new(),
            profiles: Vec::new(),
            missing: Vec::new(),
            remote: Some(delegate::Remote {
                url: agent.url.clone(),
                peer: agent.peer,
                source: agent.source.clone(),
            }),
        }));
        delegation.attach(Arc::new(DelegationHost {
            shared: Arc::downgrade(&self.shared),
            served,
            agents,
            identity,
            usage,
        }));
    }
}
