//! A `[host.profiles]` profile applied to the agent an entry starts for one connection: the environment when the process
//! starts, then context, skills and MCP servers with each session, each part where the agent's harness takes it, and a
//! report of what applied sent to the client. The client names a profile only among those its entry offers, and never
//! anything inside one.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use std::ffi::OsString;
use std::path::Path;

use coqui_acp_harness::Target;
use coqui_aos::{self as aos};
use serde_json::{Value, json};

use super::items;
use super::needs;
use super::overlay::ScriptRuns;
use super::{
    Context, Effective, Layer, LayerId, LayerKind, Scalar, Secret, Session, Sourced, Who, merge,
    starting,
};
use crate::config::HostProfile;
use hooks::{Hooks, PreEnv};

mod cache;
mod channel;
pub(crate) mod fetched;
mod hooks;
mod offer;
mod overlays;
mod parts;
mod private;
mod template;

#[cfg(test)]
mod tests;

pub(crate) use channel::Hosted;
pub use channel::{APPROVE_META, APPROVE_METHOD};
pub(crate) use hooks::forward as forward_hook;
pub(crate) use offer::{
    Choices, Offer, choose, choose_client, choose_loadout, choose_overlays, choose_preset, offer,
};
#[cfg_attr(
    not(test),
    allow(unused_imports, reason = "re-exported for hosted submodules and tests")
)]
use overlays::os;
use overlays::{
    ReadOverlay, harness_of, layers_of, overlay_items, put_first_on_path, read_chosen,
    read_overlays, same_name,
};
pub(crate) use overlays::{harness_identity, harness_version, host_variables};
#[cfg_attr(
    not(test),
    allow(unused_imports, reason = "re-exported for hosted submodules and tests")
)]
use parts::{Written, access_table, merge_json, plugin_meta, program_of, runnable};

#[allow(unused_imports, reason = "used by hosted/hooks submodules and tests")]
use channel::SESSION_METHODS;
#[allow(unused_imports, reason = "used by hosted/hooks submodules and tests")]
use offer::error;

#[cfg(test)]
use alloc::sync::Arc;
#[cfg(test)]
use std::path::PathBuf;

#[cfg(test)]
use indexmap::IndexMap;
#[cfg(test)]
use serde_json::Map;

#[cfg(test)]
use super::prepare;
#[cfg(test)]
use offer::{INVALID_PARAMS, INVALID_REQUEST};
#[cfg(test)]
use parts::{Parts, Takes, add_parts, described, parts_for, parts_with};

/// The `_meta` key a client names a profile under in `initialize`, and the host's report under in its answers.
pub const PROFILE_META: &str = "dev.boriken.canoa/profile";

/// The `_meta` key a client names, in `initialize`, the overlays this connection starts with: an array of the names the
/// profile lists. It only narrows: a name the profile does not list adds nothing and is reported as ignored, and one the
/// client leaves out is reported as not chosen. Absent, the connection starts with every overlay the profile lists.
pub const OVERLAYS_META: &str = "dev.boriken.canoa/overlays";

/// The `_meta` key a client names, in `initialize`, the loadout this connection starts with: the name of one of the profile's
/// `[loadouts]`. The host resolves it once, when the connection starts, into the overlays, skills and servers it gives; a later
/// edit of canoa.cfg reaches new connections only. Absent, the profile's default loadout applies, if it has one. A name the
/// profile does not hold is refused.
pub const LOADOUT_META: &str = "dev.boriken.canoa/loadout";

/// The `_meta` key a client names, in `initialize`, the profile preset (`[profiles.NAME]`) this connection starts with: its
/// model, effort, mode and starting instructions become the session's layer, above the person's and the agent's, and its
/// loadout is the one the connection starts with unless the client names another. A name `[profiles]` does not hold is refused.
pub const PRESET_META: &str = "dev.boriken.canoa/preset";

/// The `_meta` key a client names, in `initialize`, which kind of client it is: one of [`crate::config::CLIENT_KINDS`]. A
/// connection that names no loadout and no preset that has one starts with the loadout `[defaults.clients]` gives that kind, else
/// `[defaults] loadout`. A kind the file gives no loadout is no more than a label; anything but text is refused.
pub const CLIENT_META: &str = "dev.boriken.canoa/client";

/// The name of the one profile a source of the host's own is offered, built from canoa.cfg's top level.
pub(crate) const LOCAL_PROFILE: &str = "default";

/// What became of one part of a profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Applied,
    Unsupported,
    Off,
    Refused,
}

impl State {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Unsupported => "not supported",
            Self::Off => "off",
            Self::Refused => "refused",
        }
    }
}

/// One part of a profile (`context`, `env.NAME`, `skills.NAME`, `mcp.NAME`, `model`, ...), its layer, what became of
/// it and how, as the app's report has it. Never a value.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Item {
    key: String,
    layer: String,
    state: State,
    how: String,
    /// An MCP server's definition digest, which `allow_mcp` names.
    digest: Option<String>,
    /// What `model`, `effort` or `mode` is set to: the session's options take it, as the host cannot. Never a secret.
    value: Option<String>,
    /// The Store runtime (a provider id) that installs the program this part needs and the computer lacks.
    offer: Option<String>,
}

impl Item {
    fn new(
        key: impl Into<String>,
        layer: impl ToString,
        state: State,
        how: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            layer: layer.to_string(),
            state,
            how: how.into(),
            digest: None,
            value: None,
            offer: None,
        }
    }

    fn json(&self) -> Value {
        let mut out = json!({"key": self.key, "layer": self.layer, "state": self.state.as_str(), "how": self.how});
        if let Some(digest) = &self.digest {
            out["digest"] = json!(digest);
        }
        if let Some(offer) = &self.offer {
            out["offer"] = json!(offer);
        }
        if let Some(value) = &self.value {
            out["value"] = json!(value);
        }
        out
    }
}

/// One profile, prepared for one connection when it starts: the overlays read, the environment resolved.
#[derive(Debug)]
pub(crate) struct Connection {
    name: String,
    entry: String,
    host: HostProfile,
    overlays: Vec<ReadOverlay>,
    /// The person's own chosen skills that passed the checks, by name: their files by path.
    chosen: overlays::Chosen,
    /// The overlays' hooks, and the means to run them.
    hooks: Hooks,
    /// The `pre-env` runs the environment was built with: the same for every environment built after.
    pre_env: PreEnv,
    /// What the overlays' env scripts gave, which every environment built after applies as it was.
    scripts: ScriptRuns,
    /// What was left out before any harness is known: overlays, and entries that read a variable not shared.
    notes: Vec<Item>,
    env: Vec<(String, Secret)>,
    env_items: Vec<Item>,
    /// The directory every skill directory and every plugin of this connection is written under, so the provider can be told
    /// to work in it: made once, when something is first written, and removed with the connection.
    skills: private::Private,
    /// Whether the client is this process's own link, the only one the host asks to approve servers.
    in_process: bool,
    /// Whether this connection's sessions are given the knowledge tools, so an overlay that is served through them is not
    /// put in the context.
    kb_tools: bool,
    /// The loadout this connection started with, the role the knowledge tools tell a template the agent is in.
    role: Option<String>,
    /// The variables the profile shares, with the values the host has, for the same templates.
    session_env: Vec<(String, String)>,
    /// The host's own program and what goes before its subcommand: how a harness's hooks run the forwarder.
    launcher: OsString,
    before: Vec<OsString>,
    /// Each layer's delegation suggestion, for the harness a session's agent turns out to be.
    delegation: Vec<(Layer, Option<aos::Document>)>,
    /// What every layer reads before it applies: the identity variables and the host's own for the session.
    session: Session,
    /// The program the host starts and gives the access as variables and arguments (Grok, Antigravity, OpenCode); the others take it in
    /// the session request.
    launched_as: Option<Target>,
    /// The arguments the access and the profile's items add to that program's command line, which `canoa launch` appends.
    launch_args: Vec<String>,
    /// How its programs reach the network, when an item of the profile says: the proxy the host runs for the session stops with
    /// the connection.
    session_network: Option<crate::network::session::SessionNetwork>,
}

/// What the host knows of the session its connection's environment is made for.
#[derive(Clone, Debug, Default)]
pub(crate) struct Starting<'a> {
    /// Who the session is: harness, the entry's own model, account, folder.
    pub(crate) who: Who<'a>,
    /// Variables the host sets for the session itself, which layers read and may set their own from.
    pub(crate) host: Vec<(String, String)>,
    /// The variables the account the session runs on sets for the agent, with no credential among them: layers read them, and
    /// the provider applies them after the profile's own environment, so a layer that sets one of them is told it does not
    /// take effect.
    pub(crate) account: Vec<(String, String)>,
}

struct NarrowedChoices {
    narrowed: HostProfile,
    applied: Vec<items::Applied>,
    role: Option<String>,
}

impl Connection {
    /// What this connection's layers suggest about delegation for an agent that reports itself as `reported`, each part with
    /// its layer. An overlay's suggestion is the one it makes for that agent's harness.
    pub(crate) fn delegate_hints(&self) -> crate::delegate::Hints {
        let entry = self.entry.clone();
        let layers = self.delegation.clone();
        Box::new(move |reported| {
            let harness = harness_of(reported, &entry);
            let stripped: Vec<Layer> = layers
                .iter()
                .map(|(layer, manifest)| {
                    let mut profile = super::Profile::default();
                    match (manifest, harness) {
                        (Some(manifest), Some(harness)) => {
                            if let Some(hint) = aos::harness_delegate_of(manifest, harness) {
                                profile.delegate = super::Delegate {
                                    default: hint.default,
                                    sources: hint.sources,
                                    max_running: hint.max_running,
                                };
                            }
                        }
                        (Some(_), None) => {}
                        (None, _) => profile.delegate = layer.profile.delegate.clone(),
                    }
                    Layer::new(layer.id.clone(), None, profile)
                })
                .collect();
            merge(&stripped).delegate
        })
    }

    /// Reads the overlays `offer`'s profile `index` names and resolves its environment through `lookup`.
    ///
    /// # Errors
    ///
    /// The environment cannot be built, as [`super::ProfileError`] words it: no agent starts with it.
    #[cfg(test)]
    pub(crate) fn prepare(
        offer: &Offer,
        index: usize,
        entry: &str,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        Self::prepare_with(
            offer,
            index,
            entry,
            lookup,
            Choices::default(),
            Starting::default(),
        )
    }

    /// Resolves the preset, client default loadout, and overlay narrowing choices onto a copy of `listed`.
    fn narrow_choices(
        listed: &HostProfile,
        choices: Choices<'_>,
        harness: Option<&str>,
        notes: &mut Vec<Item>,
    ) -> Result<NarrowedChoices, String> {
        let mut narrowed = listed.clone();
        // A session that names none starts with the default profile, when the file has one.
        let preset = match choices.preset {
            Some(asked) => Some(
                listed
                    .resolve_preset(asked)
                    .map(|held| (asked, held))
                    .map_err(|error| error.to_string())?,
            ),
            None => listed
                .presets
                .get(crate::config::DEFAULT_PRESET)
                .map(|held| (crate::config::DEFAULT_PRESET, held)),
        };
        let mut loadout = choices.loadout;
        let mut preset_decides = false;
        let mut applied: Vec<items::Applied> = Vec::new();
        if let Some((asked, held)) = preset {
            applied = listed
                .items
                .catalog()
                .apply_all(&held.item_values(), harness);
            let (layer, refused) = items::session_layer(&applied);
            narrowed.session = Some(layer);
            for (key, why) in refused {
                notes.push(Item::new(
                    format!("items.{key}"),
                    LayerId::new(LayerKind::Session, ""),
                    State::Unsupported,
                    why,
                ));
            }
            preset_decides = held.loadout.is_some();
            if loadout.is_none()
                && let Some(named) = held.loadout.as_deref()
            {
                if listed.loadouts.contains_key(named) {
                    loadout = Some(named);
                } else {
                    notes.push(Item::new(
                        "loadout",
                        LayerId::new(LayerKind::Session, ""),
                        State::Unsupported,
                        format!(
                            "the profile {asked} names the loadout {named}, which canoa.cfg no longer holds: the default applies"
                        ),
                    ));
                }
            }
        }
        // A client's kind gives a default of its own, below anything the client or its preset chose.
        if loadout.is_none()
            && !preset_decides
            && let Some(kind) = choices.client
            && let Some(named) = listed.client_loadout(kind)
        {
            if listed.loadouts.contains_key(named) {
                loadout = Some(named);
            } else {
                notes.push(Item::new(
                    "loadout",
                    LayerId::new(LayerKind::Session, ""),
                    State::Unsupported,
                    format!(
                        "canoa.cfg gives {kind} clients the loadout {named}, which it no longer holds: the default applies"
                    ),
                ));
            }
        }
        let resolved = listed
            .resolve_loadout(loadout)
            .map_err(|error| error.to_string())?;
        let role = resolved.as_ref().map(|resolved| resolved.name.clone());
        let from_loadout: Option<Vec<String>> = resolved.as_ref().map(|resolved| {
            resolved
                .overlays
                .iter()
                .chain(&resolved.missing_overlays)
                .cloned()
                .collect()
        });
        let only = choices.only.or(from_loadout.as_deref());
        if let Some(resolved) = &resolved {
            narrowed.chosen_skills.clone_from(&resolved.skills);
            for server in listed.profile.mcp.keys() {
                if !resolved.mcp.contains(server) {
                    narrowed.off_mcp.insert(server.clone());
                }
            }
            for server in &resolved.mcp {
                if !listed.profile.mcp.contains_key(server) {
                    notes.push(Item::new(
                        format!("mcp.{server}"),
                        LayerId::new(LayerKind::User, ""),
                        State::Unsupported,
                        "the loadout names it, and [profile.mcp] does not define it",
                    ));
                }
            }
        }
        if let Some(only) = only {
            narrowed.overlays.retain(|identity| {
                let chosen = only.contains(identity);
                if !chosen {
                    notes.push(Item::new(
                        "overlay",
                        LayerId::new(LayerKind::Overlay, identity.clone()),
                        State::Off,
                        "not chosen for this session",
                    ));
                }
                chosen
            });
            for asked in only.iter().filter(|asked| !listed.overlays.contains(asked)) {
                notes.push(Item::new(
                    "overlay",
                    LayerId::new(LayerKind::Overlay, asked.clone()),
                    State::Unsupported,
                    "this profile does not list it, so a client cannot add it",
                ));
            }
        }
        Ok(NarrowedChoices {
            narrowed,
            applied,
            role,
        })
    }

    fn apply_session_network(
        applied: &[items::Applied],
        lookup: &dyn Fn(&str) -> Option<String>,
        skills: &private::Private,
        env: &mut Vec<(String, Secret)>,
        notes: &mut Vec<Item>,
    ) -> Option<crate::network::session::SessionNetwork> {
        let asked = items::network_of(applied)?;
        let id = applied
            .iter()
            .rev()
            .find(|each| {
                matches!(&each.outcome, items::Outcome::Done(effect) if effect.network.is_some())
            })
            .map(|each| each.id.clone())
            .unwrap_or_default();
        let layer = LayerId::new(LayerKind::Session, "");
        // What the session's programs are given to trust, when the proxy ends TLS for some destinations, goes in the connection's
        // private directory, which is made only for that.
        let private = if asked.intercept.is_empty() {
            Ok(None)
        } else {
            skills.ensure().map(Some)
        };
        let services = match &private {
            Ok(dir) => crate::network::session::Services { private: *dir },
            Err(_) => crate::network::session::Services::default(),
        };
        let made = match &private {
            Err(error) => Err(format!(
                "the session's private directory could not be made: {error}"
            )),
            Ok(_) => crate::network::session::start(asked, lookup, &services),
        };
        match made {
            Ok(started) => {
                for (name, value) in &started.variables {
                    if !env.iter().any(|(each, _)| same_name(each, name)) {
                        env.push((name.clone(), Secret(value.clone())));
                    }
                }
                notes.push(Item::new(
                    format!("items.{id}.network"),
                    &layer,
                    State::Applied,
                    format!("its programs reach the network {}", started.description),
                ));
                Some(started)
            }
            Err(why) => {
                notes.push(Item::new(
                    format!("items.{id}.network"),
                    &layer,
                    State::Refused,
                    why,
                ));
                None
            }
        }
    }

    fn record_env_and_overlay_reports(
        effective: &Effective,
        env: &[(String, Secret)],
        account_names: &[String],
        applied: &[items::Applied],
        notes: &mut Vec<Item>,
    ) -> Vec<Item> {
        for item in applied {
            let layer = LayerId::new(LayerKind::Session, "");
            notes.push(match &item.outcome {
                items::Outcome::Done(_) => Item::new(
                    format!("items.{}", item.id),
                    &layer,
                    State::Applied,
                    item.how(),
                ),
                items::Outcome::Not(why) => Item::new(
                    format!("items.{}", item.id),
                    &layer,
                    State::Unsupported,
                    why.clone(),
                ),
            });
        }
        let mut env_items = Vec::new();
        for (key, value) in &effective.env {
            if account_names.iter().any(|name| same_name(name, key)) {
                env_items.push(Item::new(
                    format!("env.{key}"),
                    &value.layer,
                    State::Unsupported,
                    format!("the account the session runs on sets {key}, so its value is used, not this layer's"),
                ));
            } else if env.iter().any(|(name, _)| name == key) {
                env_items.push(Item::new(
                    format!("env.{key}"),
                    &value.layer,
                    State::Applied,
                    "the agent's process environment",
                ));
            } else {
                env_items.push(Item::new(
                    format!("env.{key}"),
                    &value.layer,
                    State::Off,
                    "an optional reference that is not set",
                ));
            }
        }
        for report in &effective.overlays {
            for (id, subject) in &report.reports {
                if id == "env-script-not-allowed" {
                    notes.push(Item::new(
                        format!("env_scripts.{subject}"),
                        &report.layer,
                        State::Off,
                        "env-script-not-allowed: its env scripts are off until canoa.cfg names the overlay's digest, which allows its hooks and env scripts; the rest of the overlay still applies",
                    ));
                }
            }
            for what in &report.not_applied {
                notes.push(Item::new(
                    what.clone(),
                    &report.layer,
                    State::Unsupported,
                    "Canoa does not apply it",
                ));
            }
        }
        env_items
    }

    /// As [`Self::prepare`], with what the client chose: of the overlays it names only those the profile lists are read, and
    /// each other overlay the profile lists is reported as not chosen for this connection.
    ///
    /// # Errors
    ///
    /// As [`Self::prepare`].
    pub(crate) fn prepare_with(
        offer: &Offer,
        index: usize,
        entry: &str,
        lookup: &dyn Fn(&str) -> Option<String>,
        choices: Choices<'_>,
        starting_with: Starting<'_>,
    ) -> Result<Self, String> {
        let (name, listed) = &offer.profiles[index];
        let mut notes = Vec::new();
        if offer.by_default {
            notes.push(Item::new(
                "profile",
                LayerId::new(LayerKind::User, ""),
                State::Applied,
                format!(
                    "the default profile ({name}): canoa.cfg's top level, which applies to an entry that names no profile"
                ),
            ));
        }
        let NarrowedChoices {
            narrowed: host,
            applied,
            role,
        } = Self::narrow_choices(listed, choices, starting_with.who.harness, &mut notes)?;
        let overlays = read_overlays(&host, &mut notes);
        let chosen = read_chosen(&host, &mut notes);
        let layers = layers_of(&host, &overlays, &chosen, None, &mut notes);
        let model = starting(&layers, Scalar::Model);
        let effort = starting(&layers, Scalar::Effort);
        let effective = merge(&layers);
        // The account's variables come before the host's own, since the provider applies them last.
        let account_names: Vec<String> = starting_with
            .account
            .iter()
            .map(|(name, _)| name.clone())
            .collect();
        let mut account_first = starting_with.account.clone();
        account_first.extend(starting_with.host.iter().cloned());
        let pre_lookup = |name: &str| {
            starting_with
                .host
                .iter()
                .find(|(key, _)| same_name(key, name))
                .map(|(_, value)| value.clone())
                .or_else(|| lookup(name))
        };
        let pre_finder = needs::Finder::new(&pre_lookup, &crate::sources::runtime_dirs());
        let python = needs::program_for(&pre_finder, "python", None)
            .ok()
            .map(|path| path.to_string_lossy().into_owned());
        let home = host.directory.to_string_lossy().into_owned();
        let session = Session::new(
            Who {
                model: model.as_deref().or(starting_with.who.model),
                effort: effort.as_deref().or(starting_with.who.effort),
                profile: Some(name.as_str()),
                loadout: role.as_deref(),
                python: python.as_deref(),
                home: Some(home.as_str()),
                access: Some(effective.access.resolved()),
                ..starting_with.who
            },
            account_first,
            lookup,
        );
        let skills = private::Private::new("canoa-profile");
        let (mut hooks, pre_env, scripts, mut env, session_network) = {
            let seen = session.lookup(lookup);
            let lookup: &dyn Fn(&str) -> Option<String> = &seen;
            let mut hooks = Hooks::new(&host, &effective.aos, &overlays, lookup);
            // Before the agent starts nobody can be asked, so only what canoa.cfg allows runs.
            let directory = host.directory.clone();
            let harness = harness_of(None, entry).unwrap_or("canoa");
            let pre_env = hooks.pre_env(&overlays, harness, &directory);
            let added: Vec<(String, String)> = session
                .identity()
                .iter()
                .chain(&starting_with.account)
                .chain(&starting_with.host)
                .cloned()
                .collect();
            let scripts = hooks.env_scripts(&overlays, &added, &pre_env.runs);
            let mut exported = pre_env.exported.clone();
            exported.extend(
                scripts
                    .exported
                    .iter()
                    .filter(|name| !pre_env.exported.contains(name))
                    .cloned(),
            );
            let resolved = effective
                .environment_with(lookup, &pre_env.runs, &scripts.runs, &exported)
                .map_err(|error| error.to_string())?;
            notes.extend(resolved.hook_reports.iter().map(hooks::report_item));
            notes.extend(pre_env.reported.iter().cloned());
            notes.extend(scripts.items.iter().cloned());
            let finder = needs::Finder::new(lookup, &crate::sources::runtime_dirs());
            let command_dirs = overlay_items(
                &effective.aos,
                &super::prepare::Home::new(host.directory.join("overlays")),
                &finder,
                &mut notes,
            );
            let mut env = resolved.env;
            put_first_on_path(&mut env, &command_dirs, lookup);
            let session_network =
                Self::apply_session_network(&applied, lookup, &skills, &mut env, &mut notes);
            (hooks, pre_env, scripts, env, session_network)
        };
        // An overlay prepared for this version tells its processes where its root and state directories are.
        for aos in &effective.aos {
            let root_var = aos::root_variable(&aos.identity);
            if !env
                .iter()
                .any(|(each, _)| each.eq_ignore_ascii_case(&root_var))
            {
                env.push((root_var, Secret(aos.root.clone())));
            }
            let Some(state) = &aos.state else { continue };
            let name = aos::state_variable(&aos.identity);
            if !env.iter().any(|(each, _)| each.eq_ignore_ascii_case(&name)) {
                env.push((name, Secret(state.clone())));
            }
        }
        let mut item_env: Vec<(String, String)> = Vec::new();
        for (id, variable) in items::variables_of(&applied) {
            if env.iter().any(|(each, _)| same_name(each, &variable.name)) {
                notes.push(Item::new(
                    format!("items.{id}.env.{}", variable.name),
                    LayerId::new(LayerKind::Session, ""),
                    State::Unsupported,
                    format!(
                        "{} is already set for the session, so its value stays",
                        variable.name
                    ),
                ));
            } else {
                env.push((variable.name.clone(), Secret(variable.value.clone())));
                item_env.push((variable.name.clone(), variable.value.clone()));
            }
        }
        // The identity variables come first, so a layer's own value for one replaces them.
        for (name, value) in session.identity().iter().rev() {
            if !env.iter().any(|(each, _)| each.eq_ignore_ascii_case(name)) {
                env.insert(0, (name.clone(), Secret(value.clone())));
            }
        }
        hooks.add_to_environment(session.identity().iter().cloned());
        hooks.add_to_environment(item_env);
        // The person's own layers' variables are part of the session's environment, which hooks run in; an overlay's reach
        // them through the stack.
        hooks.add_to_environment(effective.env.iter().filter_map(|(name, value)| {
            let from_overlay = value.layer.kind == LayerKind::Overlay
                && effective
                    .aos
                    .iter()
                    .any(|aos| aos.identity == value.layer.name);
            let (_, secret) = env
                .iter()
                .find(|(each, _)| each == name)
                .filter(|_| !from_overlay)?;
            Some((name.clone(), secret.expose().to_owned()))
        }));
        // A program the host starts itself is given the access as its own variables and arguments; Claude, Codex and Pi take
        // it in each session's request. A variable a layer already sets keeps its value, and the report says so.
        let launched_as = starting_with
            .who
            .harness
            .and_then(access_table)
            .filter(|(_, at_launch)| *at_launch)
            .map(|(target, _)| target);
        let mut launch_args = Vec::new();
        if let Some(target) = launched_as {
            let words = effective.access.resolved().translate(&target);
            launch_args = words.args().into_iter().map(str::to_owned).collect();
            for (name, value) in words.env() {
                if env.iter().any(|(each, _)| each.eq_ignore_ascii_case(name)) {
                    let layer = effective.env.get(name).map_or_else(
                        || LayerId::new(LayerKind::User, ""),
                        |held| held.layer.clone(),
                    );
                    notes.push(Item::new(
                        format!("access.env.{name}"),
                        &layer,
                        State::Unsupported,
                        format!("{name} is set by this layer, so the access does not set it"),
                    ));
                } else {
                    env.push((name.to_owned(), Secret(value.to_owned())));
                }
            }
        }
        for item in &applied {
            if let items::Outcome::Done(effect) = &item.outcome {
                launch_args.extend(effect.args.iter().cloned());
            }
        }
        let env_items = Self::record_env_and_overlay_reports(
            &effective,
            &env,
            &account_names,
            &applied,
            &mut notes,
        );
        Ok(Self {
            name: name.clone(),
            entry: entry.to_owned(),
            host,
            overlays,
            chosen,
            hooks,
            pre_env,
            scripts: scripts.runs,
            notes,
            env,
            env_items,
            skills,
            in_process: false,
            kb_tools: false,
            role,
            session_env: session.variables(),
            launcher: offer.launcher.clone(),
            before: offer.before.clone(),
            delegation: layers
                .iter()
                .map(|layer| {
                    (
                        Layer::new(layer.id.clone(), None, {
                            let mut profile = super::Profile::default();
                            profile.delegate = layer.profile.delegate.clone();
                            profile
                        }),
                        layer.aos.as_ref().map(|aos| aos.manifest.clone()),
                    )
                })
                .collect(),
            session,
            launched_as,
            launch_args,
            session_network,
        })
    }

    /// Delivers one more skill with this connection's sessions, as the person's own. A skill of the same name the person chose
    /// stays theirs.
    pub(crate) fn with_skill(mut self, name: &str, files: BTreeMap<String, Vec<u8>>) -> Self {
        self.chosen.entry(name.to_owned()).or_insert(files);
        self
    }

    /// This connection's sessions are given the knowledge tools.
    pub(crate) fn with_kb_tools(mut self) -> Self {
        self.kb_tools = true;
        self
    }

    /// The overlays this connection applies, as the knowledge tools read them: each with the digest its files were checked
    /// against and whether its rules and context are served through the tools.
    pub(crate) fn kb_sources(&self) -> Vec<crate::kb_mcp::Source> {
        self.overlays
            .iter()
            .map(|overlay| crate::kb_mcp::Source {
                identity: overlay.identity.clone(),
                digest: overlay.digest.clone(),
                tree: overlay.tree.clone(),
                served: overlay.serve,
                options: self
                    .host
                    .options
                    .get(&overlay.identity)
                    .cloned()
                    .unwrap_or_default(),
            })
            .collect()
    }

    /// Who is asking, for the templates of the knowledge the tools serve: the profile, the loadout as the role, and the
    /// session's own variables.
    pub(crate) fn kb_caller(&self) -> crate::kb_mcp::Caller {
        crate::kb_mcp::Caller {
            role: self.role.clone(),
            profile: Some(self.name.clone()),
            env: self.session_env.clone(),
        }
    }

    /// Whether `context` is an overlay's file that is served through the knowledge tools and so not put in the context.
    fn served_as_index(&self, context: &Sourced<Context>) -> bool {
        self.kb_tools
            && context.layer.kind == LayerKind::Overlay
            && self
                .overlays
                .iter()
                .any(|overlay| overlay.identity == context.layer.name && overlay.serve)
            && matches!(context.value, Context::File(_) | Context::Template(_))
    }

    /// This connection's client is (or is not) this process's own link.
    pub(crate) fn asking(mut self, in_process: bool) -> Self {
        self.in_process = in_process;
        self
    }

    /// Where the connection's skills and plugins are written under: made when the first is written, removed with the
    /// connection.
    pub(crate) fn skills(&self) -> &Path {
        self.skills.path()
    }

    /// The directory of [`Self::skills`], made if it is not yet.
    ///
    /// # Errors
    ///
    /// When a directory is already there (it is never taken, whoever made it) or cannot be made private.
    pub(crate) fn private(&self) -> std::io::Result<&Path> {
        self.skills.ensure()
    }

    /// The network the host's own requests for this connection's sessions use, when an item of the profile chose one.
    pub(crate) fn network(&self) -> Option<alloc::sync::Arc<canoa::network::Network>> {
        self.session_network
            .as_ref()
            .map(|session| alloc::sync::Arc::clone(&session.network))
    }

    /// The profile's environment, resolved: what the agent's process is given.
    pub(crate) fn env(&self) -> &[(String, Secret)] {
        &self.env
    }

    /// The line `canoa launch` reads first.
    pub(crate) fn launch_line(&self) -> Vec<u8> {
        super::launch::launch_line(&self.env, &self.launch_args)
    }
}
