//! Overlay reading, dependency/command preparation, skill fetching, layer composition, and harness vendor identification.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use canoa::brand::CONFIG_FILE;
use std::path::{Path, PathBuf};

use coqui_aos::{self as aos, Limit, OverlayTree, Settings, StdFs};
use serde_json::{Map, Value};

use super::super::{
    AosLayer, Change, Layer, LayerId, LayerKind, OverlayInput, Secret, Skill, overlay_layer,
};
use super::super::{commands, needs, overlay, prepare};
use super::{Item, State, cache, fetched, hooks};
use crate::config::HostProfile;

/// The longest initial context a session is given, in UTF-8 bytes, every layer's together.
pub(super) const MAX_CONTEXT: usize = 1024 * 1024;

/// The path a chosen skill's entry in the person's layer carries: not a path on disk, so what is delivered is only what
/// was checked.
pub(super) const CHOSEN_PREFIX: &str = "<chosen>/";

/// The files of the person's chosen skills that passed the checks, by name.
pub(super) type Chosen = BTreeMap<String, BTreeMap<String, Vec<u8>>>;

/// What one skill's directory may hold: 1024 files, 16 MiB.
pub(super) fn skill_limits() -> Settings {
    Settings::default()
        .tree_files(Limit::at(1024))
        .tree_bytes(Limit::at(16 * 1024 * 1024))
}

/// Whether two digests are the same, comparing the hexadecimal digits without regard to case; false when either is not
/// `sha256:` and 64 of them.
pub(super) fn same_digest(a: &str, b: &str) -> bool {
    matches!((aos::Digest::parse(a), aos::Digest::parse(b)), (Ok(a), Ok(b)) if a == b)
}

/// The operating system as AOS's `os` keys name it.
pub(super) const fn os() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

/// `name` is `other`, as the platform compares variable names.
pub(super) fn same_name(name: &str, other: &str) -> bool {
    name == other || cfg!(windows) && name.eq_ignore_ascii_case(other)
}

/// An installed overlay as read when a connection starts: its tree, never read again.
#[derive(Debug)]
pub(super) struct ReadOverlay {
    pub(super) identity: String,
    pub(super) root: String,
    /// The digest its install record names, which the files were checked against.
    pub(super) digest: String,
    pub(super) tree: OverlayTree,
    /// Whether its rules and context are served through the knowledge tools: the person's `kb` choice, else its own
    /// `[knowledge] serve` hint.
    pub(super) serve: bool,
    /// The skills it names from elsewhere that the app fetched and that passed the checks, by the name it gives each:
    /// their files by path.
    pub(super) fetched: BTreeMap<String, BTreeMap<String, Vec<u8>>>,
}

impl ReadOverlay {
    /// A member file's bytes, by `/`-separated path: what the digest covered.
    pub(super) fn file(&self, path: &str) -> Option<&[u8]> {
        self.tree.bytes(path)
    }

    /// What planning the skills needs of the tree: every path, and the text of each bundled skill's `SKILL.md`.
    fn skill_view(&self) -> OverlayTree {
        let mut view = OverlayTree::from_paths(self.tree.paths().map(str::to_owned));
        for path in self.tree.paths() {
            let skill_md = path
                .strip_prefix("skills/")
                .and_then(|rest| rest.strip_suffix("/SKILL.md"))
                .is_some_and(|name| !name.contains('/'));
            if let (true, Some(bytes)) = (skill_md, self.tree.bytes(path)) {
                view = view.with_bytes(path, bytes.to_vec());
            }
        }
        view
    }

    pub(super) fn under(&self, prefix: &str) -> BTreeMap<String, Vec<u8>> {
        if let Some(name) = prefix
            .strip_prefix(fetched::SKILLS_DIR)
            .and_then(|rest| rest.strip_prefix('/'))
        {
            return self.fetched.get(name).cloned().unwrap_or_default();
        }
        let prefix = format!("{prefix}/");
        self.tree
            .paths()
            .filter_map(|path| {
                Some((
                    path.strip_prefix(&prefix)?.to_owned(),
                    self.tree.bytes(path)?.to_vec(),
                ))
            })
            .collect()
    }
}

/// Each overlay `host` names, read from `overlays/NAME` beside canoa.cfg and checked against the digest of the record
/// the app wrote when it installed it, `overlays/NAME.json`. One that cannot be read or does not match is left out.
pub(super) fn read_overlays(host: &HostProfile, notes: &mut Vec<Item>) -> Vec<ReadOverlay> {
    let mut out = Vec::new();
    let store = host.directory.join("overlays");
    for identity in &host.overlays {
        let layer = LayerId::new(LayerKind::Overlay, identity.clone());
        let mut leave_out = |why: &str| {
            notes.push(Item::new(
                "overlay",
                &layer,
                State::Refused,
                format!("left out: {why}"),
            ))
        };
        let record = std::fs::read(store.join(format!("{identity}.json")))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        let Some(expected) = record
            .as_ref()
            .and_then(|record| record.get("digest"))
            .and_then(Value::as_str)
        else {
            leave_out(
                "it is not installed, or its record holds no digest: install or sync it in Canoa",
            );
            continue;
        };
        let root = store.join(identity);
        let tree = cache::overlay(&root, expected, || {
            let (tree, digest) =
                aos::read_tree(&StdFs, &root, &Settings::default()).map_err(|error| {
                    format!(
                        "its files cannot be read ({}: {})",
                        error.class, error.subject
                    )
                })?;
            if same_digest(&digest.digest.to_string(), expected) {
                Ok(tree)
            } else {
                Err("its files are not those its record's digest names: sync it".to_owned())
            }
        });
        let tree = match tree {
            Ok(tree) => tree,
            Err(why) => {
                leave_out(&why);
                continue;
            }
        };
        let fetched = fetched_skills(&store, identity, &tree, &layer, &host.skill_pins, notes);
        let serve = aos::read_overlay(&tree, &Settings::default()).is_ok_and(|read| {
            aos::knowledge_plan(&read.manifest, &tree, host.kb_choice.get(identity).copied()).served
        });
        out.push(ReadOverlay {
            identity: identity.clone(),
            root: root.to_string_lossy().into_owned(),
            digest: expected.to_owned(),
            tree,
            serve,
            fetched,
        });
    }
    out
}

/// What each of `layers` needs of this computer, what preparing it made, and the commands it exposes, as report lines in `notes`;
/// and the folders of the launchers made, which go on the agent's `PATH`. Each overlay is reported once.
pub(super) fn overlay_items(
    layers: &[AosLayer],
    home: &prepare::Home,
    finder: &needs::Finder,
    notes: &mut Vec<Item>,
) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut inspected = Vec::new();
    for aos in layers {
        if !seen.insert(aos.identity.clone()) {
            continue;
        }
        let Some(each) = overlay::inspect(aos) else {
            continue;
        };
        let layer = LayerId::new(LayerKind::Overlay, aos.identity.clone());
        let mut lines = needs::check(&each.required, finder, &aos.os);
        lines.extend(prepare::lines_for(
            home,
            &aos.identity,
            &aos.digest,
            &each.dependencies,
            each.setup.as_ref(),
        ));
        for line in lines {
            let state = if line.met { State::Applied } else { State::Off };
            let mut item = Item::new(line.key, &layer, state, line.how);
            item.offer = line.offer;
            notes.push(item);
        }
        inspected.push((aos, each.commands));
    }
    // Two commands that differ only in the case of ASCII letters are exposed by neither.
    let named: Vec<(String, Vec<commands::Declared>)> = inspected
        .iter()
        .map(|(aos, commands)| (aos.identity.clone(), commands.clone()))
        .collect();
    let clashes = commands::collisions(&named);
    let mut dirs = Vec::new();
    for (aos, declared) in inspected {
        let layer = LayerId::new(LayerKind::Overlay, aos.identity.clone());
        let (clashing, free): (Vec<_>, Vec<_>) = declared
            .into_iter()
            .partition(|command| clashes.contains_key(&command.name.to_ascii_lowercase()));
        for command in clashing {
            let who = &clashes[&command.name.to_ascii_lowercase()];
            notes.push(Item::new(
                format!("scripts.{}", command.name),
                &layer,
                State::Refused,
                format!(
                    "the command is exposed by more than one overlay ({}): none of them is",
                    who.join(", ")
                ),
            ));
        }
        if free.is_empty() {
            continue;
        }
        let dir = home.commands(&aos.identity);
        let results = commands::expose(&dir, Path::new(&aos.root), &free, finder, cfg!(windows));
        let mut any = false;
        for (command, result) in results {
            any |= result.is_ok();
            let line = commands::line(&command, result.map(|_| ()));
            let state = if line.met { State::Applied } else { State::Off };
            let mut item = Item::new(line.key, &layer, state, line.how);
            item.offer = line.offer;
            notes.push(item);
        }
        if any {
            dirs.push(dir);
        }
    }
    dirs
}

/// Puts `dirs` first on the `PATH` in `env`, which is the host's own (`lookup`) when `env` sets none: the commands overlays expose
/// are found by the agent's processes, and never decide which program the agent is.
pub(super) fn put_first_on_path(
    env: &mut Vec<(String, Secret)>,
    dirs: &[PathBuf],
    lookup: &dyn Fn(&str) -> Option<String>,
) {
    if dirs.is_empty() {
        return;
    }
    let held = env
        .iter()
        .position(|(name, _)| name.eq_ignore_ascii_case("PATH"));
    let current = held
        .map(|at| env[at].1.expose().to_owned())
        .or_else(|| lookup("PATH"))
        .unwrap_or_default();
    let Ok(joined) =
        std::env::join_paths(dirs.iter().cloned().chain(std::env::split_paths(&current)))
    else {
        return;
    };
    let value = Secret(joined.to_string_lossy().into_owned());
    match held {
        Some(at) => env[at].1 = value,
        None => env.push(("PATH".to_owned(), value)),
    }
}

/// The skills the session's loadout names, as the app recorded them, checked again here. What fails is
/// reported and left out.
pub(super) fn read_chosen(host: &HostProfile, notes: &mut Vec<Item>) -> Chosen {
    if host.chosen_skills.is_empty() {
        return Chosen::new();
    }
    let layer = LayerId::new(LayerKind::User, "");
    let found = fetched::read_chosen(
        &host.directory.join("overlays"),
        &host.chosen_skills,
        &host.skill_pins,
    );
    for (name, state, how) in found.notes {
        notes.push(Item::new(format!("skills.{name}"), &layer, state, how));
    }
    found.files
}

/// The skills `tree`'s manifest names that it does not bundle, as the app fetched them: what passed the checks, and a
/// report of the rest.
fn fetched_skills(
    store: &Path,
    identity: &str,
    tree: &OverlayTree,
    layer: &LayerId,
    pins: &BTreeMap<String, String>,
    notes: &mut Vec<Item>,
) -> BTreeMap<String, BTreeMap<String, Vec<u8>>> {
    let Ok(read) = aos::read_overlay(tree, &Settings::default()) else {
        return BTreeMap::new();
    };
    let elsewhere: Vec<aos::SkillEntry> = aos::skill_entries(&read.manifest, tree)
        .into_iter()
        .filter(|entry| entry.origin != aos::SkillOrigin::Bundled)
        .collect();
    let found = fetched::read(store, identity, &elsewhere, pins);
    for (name, state, how) in found.notes {
        notes.push(Item::new(format!("skills.{name}"), layer, state, how));
    }
    found.files
}

/// Plans the skills of all the overlays read together, as the specification has a consumer do before it delivers
/// any, and gives each layer the fetched skills it is to deliver: a skill from elsewhere never takes a name another
/// skill holds. Said once, with the first harness-less pass.
pub(super) fn deliver_fetched(
    overlays: &[ReadOverlay],
    manifests: &BTreeMap<String, Map<String, Value>>,
    active: &[String],
    found: &mut BTreeMap<String, Layer>,
    harness: Option<&str>,
    notes: &mut Vec<Item>,
) {
    let views: BTreeMap<String, OverlayTree> = overlays
        .iter()
        .map(|overlay| (aos::normalize_name(&overlay.identity), overlay.skill_view()))
        .filter(|(key, _)| manifests.contains_key(key))
        .collect();
    let layer_of = |key: &str| LayerId::new(LayerKind::Overlay, key.to_owned());
    let plan = match aos::resolve_skills(
        manifests,
        active,
        &views,
        &[],
        &[],
        true,
        &Settings::default(),
    ) {
        Ok(plan) => plan,
        Err(error) => {
            if harness.is_none() {
                notes.push(Item::new(
                    "skills",
                    LayerId::new(LayerKind::User, ""),
                    State::Refused,
                    format!(
                        "no skill from elsewhere is delivered: the overlays' skills cannot be planned ({})",
                        error.class
                    ),
                ));
            }
            return;
        }
    };
    let holder = |name: &str| {
        plan.delivered
            .iter()
            .find(|(held, _)| held.eq_ignore_ascii_case(name))
            .map(|(_, overlay)| overlay.as_str())
    };
    for overlay in overlays {
        let key = aos::normalize_name(&overlay.identity);
        let Some(layer) = found.get_mut(&key) else {
            continue;
        };
        for name in overlay.fetched.keys() {
            if holder(name) == Some(key.as_str()) {
                layer.profile.skills.insert(
                    name.clone(),
                    Change::Set(Skill::new(format!("{}/{name}", fetched::SKILLS_DIR))),
                );
            }
        }
    }
    if harness.is_some() {
        return;
    }
    for (name, overlay) in &plan.shadowing {
        notes.push(Item::new(
            format!("skills.{name}"),
            layer_of(overlay),
            State::Refused,
            format!(
                "not delivered: a skill from elsewhere never takes a name another skill holds ({} holds it)",
                holder(name).unwrap_or("another overlay")
            ),
        ));
    }
    for name in &plan.collisions {
        if let Some(winner) = holder(name) {
            notes.push(Item::new(
                format!("skills.{name}.collision"),
                layer_of(winner),
                State::Applied,
                "more than one overlay offers a skill of this name; this overlay's is the one delivered",
            ));
        }
    }
    for (id, name, overlay) in &plan.non_defaults {
        let delivered = holder(name) == Some(overlay.as_str())
            && overlays.iter().any(|read| {
                aos::normalize_name(&read.identity) == *overlay && read.fetched.contains_key(name)
            });
        if delivered {
            let how = if id == "insecure-transport" {
                "its source is not over an encrypted transport"
            } else {
                "its source is on a private network address"
            };
            notes.push(Item::new(
                format!("skills.{name}.{id}"),
                layer_of(overlay),
                State::Applied,
                how,
            ));
        }
    }
}

/// The layers of `host` for `harness`: its overlays in composition order, then its own.
pub(super) fn layers_of(
    host: &HostProfile,
    overlays: &[ReadOverlay],
    chosen: &Chosen,
    harness: Option<&str>,
    notes: &mut Vec<Item>,
) -> Vec<Layer> {
    let mut found = BTreeMap::new();
    let mut manifests = BTreeMap::new();
    for overlay in overlays {
        let mut input = OverlayInput::new(&overlay.identity, &overlay.root, os(), &overlay.tree);
        input.harness = harness;
        input.pinned = true;
        match overlay_layer(&input) {
            Ok(layer)
                if layer
                    .aos
                    .as_ref()
                    .is_some_and(|aos| !hooks::activates(host, aos, overlay)) =>
            {
                // A guard that is off would fail open, so the overlay waits for its hooks to be allowed.
                if harness.is_none() {
                    notes.push(Item::new(
                        "overlay",
                        LayerId::new(LayerKind::Overlay, overlay.identity.clone()),
                        State::Refused,
                        format!("left out: a hook of it fails closed and its hooks are not allowed; {CONFIG_FILE}'s agents.NAME.hooks names the digest that allows them"),
                    ));
                }
            }
            Ok(mut layer) => {
                let key = aos::normalize_name(&overlay.identity);
                if let Some(aos) = layer.aos.as_mut() {
                    aos.allowed = host.allow_hooks.get(&overlay.identity).cloned();
                    let home = prepare::Home::new(host.directory.join("overlays"));
                    // What preparing made counts for the version it was made for, or for runtime dependencies whose manifest digest is unchanged.
                    if home
                        .record(&overlay.identity)
                        .is_some_and(|record| record.digest.eq_ignore_ascii_case(&aos.digest))
                    {
                        aos.packages = home.packages(&overlay.identity);
                        let state = home.state(&overlay.identity);
                        aos.state = state.is_dir().then(|| state.to_string_lossy().into_owned());
                    } else if let Some(inspected) = overlay::inspect(aos) {
                        let all_pkgs = home.packages(&overlay.identity);
                        let mut active_pkgs = BTreeMap::new();
                        for dep in &inspected.dependencies {
                            let rt = dep.runtime.as_str();
                            if home.runtime_ready(&overlay.identity, &aos.digest, dep)
                                && let Some(dir) = all_pkgs.get(rt)
                            {
                                active_pkgs.insert(rt.to_owned(), dir.clone());
                            }
                        }
                        if !active_pkgs.is_empty() {
                            aos.packages = active_pkgs;
                            let state = home.state(&overlay.identity);
                            aos.state =
                                state.is_dir().then(|| state.to_string_lossy().into_owned());
                        }
                    }
                }
                if let Some(aos) = &layer.aos {
                    manifests.insert(key.clone(), aos.manifest.clone());
                }
                found.insert(key, layer);
            }
            Err(refusal) => {
                // Read once per harness; said once.
                if harness.is_none() {
                    notes.push(Item::new(
                        "overlay",
                        LayerId::new(LayerKind::Overlay, overlay.identity.clone()),
                        State::Refused,
                        format!("left out: {refusal}"),
                    ));
                }
            }
        }
    }
    let active: Vec<String> = manifests.keys().cloned().collect();
    deliver_fetched(overlays, &manifests, &active, &mut found, harness, notes);
    let mut layers: Vec<Layer> = aos::composition_order(&manifests, &active)
        .into_iter()
        .filter_map(|key| found.remove(&key))
        .collect();
    // Canoa's own, below every other: the standard access, the same for every harness.
    layers.insert(
        0,
        Layer::new(
            LayerId::new(LayerKind::BuiltIn, ""),
            None,
            super::super::Profile {
                access: super::super::AccessSetting::standard(),
                ..super::super::Profile::default()
            },
        ),
    );
    let mut own = host.profile.clone();
    for name in chosen.keys() {
        // An entry the person wrote in `[profile.skills]` for the name is theirs and stays.
        own.skills
            .entry(name.clone())
            .or_insert_with(|| Change::Set(Skill::new(format!("{CHOSEN_PREFIX}{name}"))));
    }
    layers.push(Layer::new(
        LayerId::new(LayerKind::User, ""),
        Some(host.directory.clone()),
        own,
    ));
    if let Some((label, profile)) = &host.agent {
        layers.push(Layer::new(
            LayerId::new(LayerKind::Agent, label.clone()),
            Some(host.directory.clone()),
            profile.clone(),
        ));
    }
    if let Some(profile) = &host.session {
        layers.push(Layer::new(
            LayerId::new(LayerKind::Session, ""),
            Some(host.directory.clone()),
            profile.clone(),
        ));
    }
    layers
}

/// The AOS harness id of what an agent reports as its name, else of an entry named as a loaded agent is. A translator of Canoa's own
/// (`coqui-acp-...`) that no description names says what it takes itself, so the entry's name does not decide for it.
pub(super) fn harness_of(reported: Option<&str>, entry: &str) -> Option<&'static str> {
    if let Some(name) = reported {
        if let Some(known) = crate::vendors::by_name(name) {
            return known.aos;
        }
        if name.starts_with("coqui-acp-") {
            return None;
        }
    }
    crate::vendors::by_title(entry).and_then(|known| known.aos)
}

/// The AOS harness id of a session's agent for the identity variables: the id of the provider an entry runs in this process,
/// else of an entry named as a loaded agent is.
pub(crate) fn harness_identity(provider: Option<&str>, entry: &str) -> Option<&'static str> {
    provider
        .and_then(crate::vendors::by_agent)
        .and_then(|known| known.aos)
        .or_else(|| harness_of(None, entry))
}

/// The version of the program the Store installed for the provider an entry runs in this process.
pub(crate) fn harness_version(provider: Option<&str>) -> Option<String> {
    match provider {
        Some(harness) => {
            crate::bundled::managed_program_name(harness).and_then(crate::sources::managed_version)
        }
        None => None,
    }
}

/// The variables the host sets for a session of the provider it runs in this process: what the Store's copy of the program
/// is given, which a layer may read and set its own from. The provider's own settings for an account (its endpoint, its
/// state directory) are applied inside the provider, after the request's environment, and are not these.
pub(crate) fn host_variables(provider: Option<&str>) -> Vec<(String, String)> {
    match provider {
        Some(harness) => crate::bundled::managed_program_name(harness)
            .and_then(crate::sources::managed_program)
            .map(|(_, environment)| environment)
            .unwrap_or_default(),
        None => Vec::new(),
    }
}

/// What the agent whose registry id is `harness` says of why it does not take something, from its description provider.
pub(super) fn not_taken(
    harness: Option<&str>,
    ask: impl FnOnce(
        &dyn canoa::vendor::AgentVendor,
    ) -> Result<Option<String>, coqui_acp_api::guatiao::library::ProviderError>,
) -> Option<String> {
    let known = crate::vendors::by_aos(harness?)?;
    crate::vendors::behave(known.agent, ask)?.ok().flatten()
}

/// What the agent whose registry id is `harness` says of the MCP servers it is given: why it starts none, and what it does with one.
pub(super) fn mcp_words(harness: Option<&str>) -> (Option<String>, Option<String>) {
    (
        not_taken(harness, |vendor| vendor.mcp_not_taken()),
        not_taken(harness, |vendor| vendor.mcp_note()),
    )
}

pub(super) fn context_not_taken(harness: Option<&str>) -> String {
    not_taken(harness, |vendor| vendor.context_not_taken())
        .unwrap_or_else(|| "the agent does not say it takes a system prompt".to_owned())
}

pub(super) fn skills_not_taken(harness: Option<&str>) -> String {
    not_taken(harness, |vendor| vendor.skills_not_taken()).unwrap_or_else(|| {
        "ACP has no field for skills, and Canoa knows no other way this agent takes them".to_owned()
    })
}
