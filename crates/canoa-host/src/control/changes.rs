//! What the `canoa-store` tools change on this host, once the person has said yes: an overlay installed, updated or removed, a
//! program installed, updated, rolled back or removed, and the loadout every new session starts with.
//!
//! Each change is worked out first and kept under the session that asked, as a plan only that session can apply. The person is
//! shown what the host kept, and what is applied is what the host kept, found again by its token and checked against the digest
//! the person was shown: a plan that moved since, or one made for another session, is refused and nothing changes. Nothing here
//! is reachable by a catalog's words, and no source comes from the agent: an entry is looked up in the sources the person listed.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

use super::{Caller, Control, Grant};
use crate::profile::OverlaySource;
use crate::store::mcp::{Action, Change, Prepared};

/// What names a session's plans on the host.
const OWNER_PREFIX: &str = "canoa-store:";
/// Files or items listed on a card before it says how many more there are.
const LISTED: usize = 12;
/// The most steps or outside changes a card can show: they are never cut, so a plan with more is not asked about.
const NEVER_CUT: usize = 15;

type Made<T> = Result<T, String>;

fn why((_, message): (i32, String)) -> String {
    message
}

/// The caller a session's changes are made as: its own owner name, so one session's plans are never another's, and the grants of
/// the key behind the connection.
fn caller_of(session: &str, grants: &BTreeSet<Grant>) -> Caller {
    Caller::Peer {
        id: format!("{OWNER_PREFIX}{session}"),
        grants: grants.clone(),
    }
}

/// Refuses unless the key holds every one of `needed`, naming what is missing.
fn need(caller: &Caller, needed: &[Grant]) -> Made<()> {
    let missing: Vec<&str> = needed
        .iter()
        .filter(|grant| !caller.may(**grant))
        .map(|grant| grant.as_str())
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "this key lacks the {} grant{}, which this change takes",
            missing.join(" and "),
            if missing.len() == 1 { "" } else { "s" }
        ))
    }
}

fn text<'a>(entry: &'a Value, key: &str) -> Option<&'a str> {
    entry
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn same_digest(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// Whether the pin canoa.cfg gives an overlay may be moved to `new` by a change the person approves: it is the pin the store
/// wrote itself (the digest now installed), or already the digest to install. A pin the person set by hand is theirs to move.
fn pin_may_move(pin: &str, installed: Option<&str>, new: &str) -> bool {
    same_digest(pin, new) || installed.is_some_and(|installed| same_digest(pin, installed))
}

/// A list of at most [`LISTED`] lines, then how many more.
fn listed(lines: &mut Vec<String>, prefix: &str, items: impl IntoIterator<Item = String>) {
    let items: Vec<String> = items.into_iter().collect();
    for item in items.iter().take(LISTED) {
        lines.push(format!("  {prefix} {item}"));
    }
    if items.len() > LISTED {
        lines.push(format!("  …and {} more", items.len() - LISTED));
    }
}

/// Lines that are the host's own word on what a change does: all of them or no card.
fn listed_whole(
    lines: &mut Vec<String>,
    prefix: &str,
    items: impl IntoIterator<Item = String>,
) -> Made<()> {
    let items: Vec<String> = items.into_iter().collect();
    if items.len() > NEVER_CUT {
        return Err(format!(
            "this change has {} items the person must see in full, more than a card holds: ask the person to use the Store",
            items.len()
        ));
    }
    for item in items {
        lines.push(format!("  {prefix} {item}"));
    }
    Ok(())
}

fn names(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

impl Control {
    /// Works out `change` for the session and keeps it for that session alone. Nothing is installed, removed or written.
    pub(crate) fn tool_prepare(
        &self,
        session: &str,
        grants: &BTreeSet<Grant>,
        change: &Change,
    ) -> Made<Prepared> {
        let caller = caller_of(session, grants);
        match (change.kind.as_str(), change.action) {
            ("overlay", Action::Install | Action::Update) => self.overlay_install(&caller, change),
            ("overlay", Action::Remove) => self.overlay_remove(&caller, change),
            ("overlay" | "skill", Action::Enable | Action::Disable) => {
                self.loadout_change(&caller, change)
            }
            ("skill", _) => Err(
                "a skill a catalog offers is fetched by the Store in the Canoa app, which checks it against its pin; \
                 this host has no path for it from a tool. Tell the person which skill to install there."
                    .to_owned(),
            ),
            ("program", _) => self.program_change(&caller, change),
            _ => Err("that kind of change is not one this host makes".to_owned()),
        }
    }

    /// Applies what [`Control::tool_prepare`] kept, if it still holds and is what `prepared.digest` says.
    pub(crate) fn tool_commit(
        &self,
        session: &str,
        grants: &BTreeSet<Grant>,
        prepared: &Prepared,
        cancel: &Arc<AtomicBool>,
    ) -> Made<Value> {
        let caller = caller_of(session, grants);
        let detail = &prepared.detail;
        match detail.get("do").and_then(Value::as_str) {
            Some("overlay-install") => self.overlay_apply(&caller, prepared),
            Some("overlay-remove") => self.overlay_removed(&caller, prepared),
            Some("loadout") => self.loadout_apply(&caller, prepared),
            Some("program-install") => self.program_apply(&caller, prepared, cancel),
            Some("program-remove" | "program-rollback") => self.program_switch(&caller, prepared),
            _ => Err("the host holds nothing to apply for that".to_owned()),
        }
    }

    /// Gives up what [`Control::tool_prepare`] kept.
    pub(crate) fn tool_discard(
        &self,
        session: &str,
        grants: &BTreeSet<Grant>,
        prepared: &Prepared,
    ) {
        let caller = caller_of(session, grants);
        match prepared.detail.get("do").and_then(Value::as_str) {
            Some("overlay-install") => {
                self.overlays.forget_plan(&caller, &prepared.token);
                if let Some(name) = text(&prepared.detail, "name") {
                    let _ = self.overlays.discard(&json!({"name": name}));
                }
            }
            Some("program-install") => {
                if let (Ok(stores), Some(provider)) =
                    (self.stores(), text(&prepared.detail, "provider"))
                {
                    stores.forget(caller.owner(), provider, &prepared.token);
                }
            }
            _ => {}
        }
    }

    // ---- overlays -------------------------------------------------------------------------------------------------

    /// How canoa.cfg stands towards the overlay `name` from the source `repo` `reference` `path`: the key it is under, if any, and the
    /// pin it gives. An overlay named there by hand, or from another source, is the person's and is refused.
    fn overlay_standing(
        &self,
        name: &str,
        source: &(String, String, String),
        catalog: Option<&str>,
    ) -> Made<(Option<String>, Option<String>)> {
        let section = self.config_section("overlays");
        let Some((key, present)) = section
            .as_object()
            .and_then(|table| table.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)))
        else {
            return Ok((None, None));
        };
        let spec = present
            .get("source")
            .and_then(Value::as_str)
            .or_else(|| present.as_str());
        let from = spec.and_then(|spec| match OverlaySource::parse_git(spec) {
            Ok(OverlaySource::Git {
                repo,
                reference,
                path,
            }) => Some((repo, reference, path)),
            _ => None,
        });
        if from.as_ref() != Some(source) {
            return Err(format!(
                "canoa.cfg names {key} from another source or by hand: remove it there first to let the store manage it"
            ));
        }
        let record = self
            .overlays
            .installed(&json!({"name": key}))
            .map_err(why)?;
        let recorded = record
            .pointer(&format!(
                "/overlays/{}/catalog",
                key.replace('~', "~0").replace('/', "~1")
            ))
            .and_then(Value::as_str);
        let Some(recorded) = recorded else {
            return Err(format!(
                "canoa.cfg names {key} by hand: remove it there first to let the store manage it"
            ));
        };
        if let Some(catalog) = catalog
            && catalog != recorded
        {
            return Err(format!(
                "{key} was installed from the catalog {recorded}, and this entry is from {catalog}: remove it first to take it from there"
            ));
        }
        Ok((
            Some(key.clone()),
            present
                .get("pin")
                .and_then(Value::as_str)
                .map(str::to_owned),
        ))
    }

    /// The first of canoa.cfg and the overlays folder that is on the person's protected list, if one is.
    fn protected_place(&self, overlays_too: bool) -> Option<String> {
        let stores = self.stores().ok()?;
        let mut places = vec![self.files.config_file()];
        if overlays_too {
            places.push(self.overlays.root_path());
        }
        places
            .into_iter()
            .flatten()
            .find(|place| stores.protects(place))
            .map(|place| place.display().to_string())
    }

    fn config_section(&self, name: &str) -> Value {
        self.files
            .config_get(&json!({"section": name}))
            .ok()
            .and_then(|answer| answer.get("value").cloned())
            .unwrap_or(Value::Null)
    }

    fn overlay_install(&self, caller: &Caller, change: &Change) -> Made<Prepared> {
        need(
            caller,
            &[
                Grant::OverlayWrite,
                Grant::ConfigWrite,
                Grant::ConfigWriteSessions,
            ],
        )?;
        let entry = change
            .entry
            .as_ref()
            .ok_or("the Store lists no such overlay")?;
        let name = text(entry, "name").ok_or("the entry has no name")?;
        let origin = entry.get("origin").ok_or("the entry names no source")?;
        if text(origin, "kind") != Some("git") {
            return Err("this entry's source is not a git source the host fetches".to_owned());
        }
        let repo = text(origin, "location").ok_or("the entry's source has no location")?;
        let reference = text(origin, "ref").ok_or("the entry's source has no ref")?;
        let path = text(origin, "path").unwrap_or("");
        let repo = repo.strip_prefix("git+").unwrap_or(repo);
        let spec = if path.is_empty() {
            format!("git+{repo}@{reference}")
        } else {
            format!("git+{repo}@{reference}#{path}")
        };
        let source = match OverlaySource::parse_git(&spec) {
            Ok(OverlaySource::Git {
                repo,
                reference,
                path,
            }) => (repo, reference, path),
            _ => return Err("this entry's source is not a git source the host fetches".to_owned()),
        };
        let (key, config_pin) = self.overlay_standing(name, &source, text(entry, "catalog"))?;
        let mut params = json!({"name": name, "repo": repo, "ref": reference, "path": path});
        if let Some(pin) = text(entry, "pin") {
            params["pin"] = json!(pin);
        }
        let checked = self
            .overlays
            .check(caller, &params, &|repo| self.may_read_source(caller, repo))
            .map_err(why)?;
        let digest = text(&checked, "digest")
            .ok_or("the host read no digest")?
            .to_owned();
        let token = text(&checked, "token")
            .ok_or("the host kept no plan")?
            .to_owned();
        let discard = || {
            self.overlays.forget_plan(caller, &token);
            let _ = self.overlays.discard(&json!({"name": name}));
        };
        let before = checked.get("fromDigest").and_then(Value::as_str);
        if let Some(pin) = config_pin.as_deref()
            && !pin_may_move(pin, before, &digest)
        {
            discard();
            return Err(format!(
                "canoa.cfg pins {pin} for {name}, which is neither what the store installed nor what was fetched ({digest}): a pin set by hand is the person's to move, so nothing was changed"
            ));
        }
        let protected = self.protected_place(true);
        let prepared = |nothing: Option<String>| Prepared {
            token: token.clone(),
            digest: digest.clone(),
            title: format!(
                "{} the overlay {name}",
                if before.is_some() {
                    "Update"
                } else {
                    "Install"
                }
            ),
            lines: self.overlay_lines(
                entry,
                &checked,
                &spec,
                name,
                key.is_some(),
                protected.as_deref(),
            ),
            protected: protected.clone(),
            nothing,
            detail: json!({"do": "overlay-install", "name": name, "spec": spec,
                "catalog": text(entry, "catalog"), "key": key}),
        };
        if key.is_some() && before.is_some_and(|before| same_digest(before, &digest)) {
            discard();
            return Ok(prepared(Some(format!(
                "{name} is already installed at this digest"
            ))));
        }
        Ok(prepared(None))
    }

    fn overlay_lines(
        &self,
        entry: &Value,
        checked: &Value,
        spec: &str,
        name: &str,
        managed: bool,
        protected: Option<&str>,
    ) -> Vec<String> {
        let digest = text(checked, "digest").unwrap_or_default();
        let mut lines = vec![
            format!(
                "{} the overlay {name}",
                if managed { "Update" } else { "Install" }
            ),
            format!("Catalog: {}", text(entry, "catalog").unwrap_or("none")),
            format!("Source: {spec}"),
            format!("Commit: {}", text(checked, "commit").unwrap_or_default()),
            format!("Tree digest: {digest}"),
            match text(entry, "pin") {
                Some(pin) => format!("The catalog pins: {pin} (the files fetched match it)"),
                None => "The catalog gives no pin: only the digest above is checked".to_owned(),
            },
        ];
        if let Some(from) = text(checked, "fromDigest") {
            lines.push(format!("Now installed: {from}"));
            if !same_digest(from, digest) {
                lines.push(format!(
                    "It moves the pin in canoa.cfg from {from} to {digest}."
                ));
            }
        }
        if let Some(contents) = entry
            .get("contents")
            .filter(|contents| contents.is_object())
        {
            for (label, key) in [
                ("skills", "skills"),
                ("MCP servers", "mcp_servers"),
                ("hooks", "hooks"),
                ("environment scripts", "env_scripts"),
            ] {
                let listed_names = names(contents.get(key));
                if !listed_names.is_empty() {
                    lines.push(format!(
                        "The catalog says it holds {label}: {}",
                        listed_names
                            .iter()
                            .take(LISTED)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
        }
        lines.push(
            "Hooks, environment scripts and MCP servers it holds stay off until you allow them at their digest.".to_owned(),
        );
        lines.push(
            "Skills it names from other catalogs are not fetched by this change. It reaches sessions started after it; running ones keep what they have.".to_owned(),
        );
        let added = names(checked.get("added"));
        let changed = names(checked.get("changed"));
        let removed = names(checked.get("removed"));
        let total = checked
            .get("files")
            .and_then(Value::as_object)
            .map_or(0, Map::len);
        lines.push(format!(
            "Files: {} in all; {} added, {} changed, {} removed",
            total,
            added.len(),
            changed.len(),
            removed.len()
        ));
        listed(&mut lines, "+", added);
        listed(&mut lines, "~", changed);
        listed(&mut lines, "-", removed);
        lines.push(
            "It writes the overlay's folder beside canoa.cfg and its entry in [overlays]."
                .to_owned(),
        );
        if let Some(place) = protected {
            lines.push(format!("It writes a place on your protected list: {place}"));
        }
        lines
    }

    fn overlay_apply(&self, caller: &Caller, prepared: &Prepared) -> Made<Value> {
        need(
            caller,
            &[
                Grant::OverlayWrite,
                Grant::ConfigWrite,
                Grant::ConfigWriteSessions,
            ],
        )?;
        let detail = &prepared.detail;
        let name = text(detail, "name").ok_or("the host holds no name for this change")?;
        let spec = text(detail, "spec").ok_or("the host holds no source for this change")?;
        // What is applied is the plan the host kept, and it is the one the person was shown.
        let kept = self
            .overlays
            .kept_digest(caller, &prepared.token)
            .map_err(why)?;
        if !same_digest(&kept, &prepared.digest) {
            self.overlays.forget_plan(caller, &prepared.token);
            return Err(
                "the plan is not what the person was shown: nothing was changed".to_owned(),
            );
        }
        let source = match OverlaySource::parse_git(spec) {
            Ok(OverlaySource::Git {
                repo,
                reference,
                path,
            }) => (repo, reference, path),
            _ => return Err("the source is not a git source the host fetches".to_owned()),
        };
        // canoa.cfg may have changed while the person decided.
        self.overlay_standing(name, &source, text(detail, "catalog"))
            .inspect_err(|_| {
                self.overlays.forget_plan(caller, &prepared.token);
            })?;
        let record = self
            .overlays
            .installed(&json!({"name": name}))
            .ok()
            .and_then(|answer| answer.pointer(&format!("/overlays/{name}")).cloned());
        let installed_now = record
            .as_ref()
            .and_then(|record| text(record, "digest"))
            .map(str::to_owned);
        // The pin goes first, under the lock every write of canoa.cfg takes: if the files then fail to be placed the pin is put
        // back, and a failed write of canoa.cfg leaves the files as they were.
        let mut before: Option<(String, Option<Value>)> = None;
        self.files
            .config_update("overlays", true, |current| {
                let mut table = match current {
                    Value::Object(table) => table,
                    _ => Map::new(),
                };
                let key = table
                    .keys()
                    .find(|key| key.eq_ignore_ascii_case(name))
                    .cloned()
                    .unwrap_or_else(|| name.to_owned());
                let old = table.get(&key).cloned();
                if let Some(pin) = old
                    .as_ref()
                    .and_then(|old| old.get("pin"))
                    .and_then(Value::as_str)
                    && !pin_may_move(pin, installed_now.as_deref(), &prepared.digest)
                {
                    return Err("canoa.cfg now pins another digest: nothing was changed".to_owned());
                }
                let mut entry = Map::new();
                if let Some(options) = old
                    .as_ref()
                    .and_then(|old| old.get("options"))
                    .and_then(Value::as_object)
                    .filter(|options| !options.is_empty())
                {
                    entry.insert("options".to_owned(), Value::Object(options.clone()));
                }
                entry.insert("source".to_owned(), json!(spec));
                entry.insert("pin".to_owned(), json!(prepared.digest));
                table.insert(key.clone(), Value::Object(entry));
                before = Some((key, old));
                Ok(Value::Object(table))
            })
            .map_err(|error| {
                self.overlays.forget_plan(caller, &prepared.token);
                why(error)
            })?;
        let mut params = json!({"token": prepared.token});
        if let Some(label) = text(detail, "catalog") {
            params["catalog"] = json!(label);
        }
        let record = match self.overlays.apply(caller, &params) {
            Ok(record) => record,
            Err(error) => {
                let restored = self.files.config_update("overlays", true, |current| {
                    let mut table = match current {
                        Value::Object(table) => table,
                        _ => Map::new(),
                    };
                    if let Some((key, old)) = before {
                        match old {
                            Some(old) => table.insert(key, old),
                            None => table.remove(&key),
                        };
                    }
                    Ok(if table.is_empty() {
                        Value::Null
                    } else {
                        Value::Object(table)
                    })
                });
                let mut message = why(error);
                if let Err(error) = restored {
                    message.push_str(&format!(
                        "; canoa.cfg's entry could not be put back: {}",
                        why(error)
                    ));
                }
                return Err(message);
            }
        };
        let installed = text(&record, "digest")
            .unwrap_or(&prepared.digest)
            .to_owned();
        tracing::info!(overlay = name, digest = %installed, "an overlay was installed by a store tool");
        Ok(json!({
            "overlay": name,
            "digest": installed,
            "commit": record.get("commit"),
            "reaches": "sessions started from now on",
        }))
    }

    fn overlay_remove(&self, caller: &Caller, change: &Change) -> Made<Prepared> {
        need(
            caller,
            &[
                Grant::OverlayWrite,
                Grant::ConfigWrite,
                Grant::ConfigWriteSessions,
            ],
        )?;
        let name = change
            .entry
            .as_ref()
            .and_then(|entry| text(entry, "name"))
            .unwrap_or(&change.id);
        let answer = self
            .overlays
            .installed(&json!({"name": name}))
            .map_err(why)?;
        let Some(record) = answer.pointer(&format!("/overlays/{name}")) else {
            return Ok(Prepared {
                token: String::new(),
                digest: String::new(),
                title: String::new(),
                lines: Vec::new(),
                nothing: Some(format!("{name} is not installed")),
                detail: Value::Null,
                protected: None,
            });
        };
        let spec = text(record, "source").unwrap_or_default();
        let source = match OverlaySource::parse_git(spec) {
            Ok(OverlaySource::Git {
                repo,
                reference,
                path,
            }) => (repo, reference, path),
            _ => {
                return Err(format!(
                    "{name} was not installed by the store: remove it where it was added"
                ));
            }
        };
        let (key, _) = self.overlay_standing(name, &source, None)?;
        if key.is_none() || text(record, "catalog").is_none() {
            return Err(format!(
                "canoa.cfg does not name {name} as the store installed it: the store does not remove it"
            ));
        }
        let digest = text(record, "digest").unwrap_or_default().to_owned();
        let files = record
            .get("files")
            .and_then(Value::as_object)
            .map_or(0, Map::len);
        let protected = self.protected_place(true);
        let mut lines = vec![
            format!("Remove the overlay {name}"),
            format!("Source: {spec}"),
            format!("Installed digest: {digest}"),
            format!("{files} files in it"),
            "It deletes the overlay's folder, the history fetched for it, and its entry in [overlays].".to_owned(),
            "New sessions start without it; running sessions keep what they have. Skills fetched for it stay, listed as named by no overlay, until the person removes them in the Store.".to_owned(),
        ];
        if let Some(place) = &protected {
            lines.push(format!("It writes a place on your protected list: {place}"));
        }
        Ok(Prepared {
            protected,
            token: String::new(),
            digest: digest.clone(),
            title: format!("Remove the overlay {name}"),
            lines,
            nothing: None,
            detail: json!({"do": "overlay-remove", "name": name, "spec": spec, "key": key}),
        })
    }

    fn overlay_removed(&self, caller: &Caller, prepared: &Prepared) -> Made<Value> {
        need(
            caller,
            &[
                Grant::OverlayWrite,
                Grant::ConfigWrite,
                Grant::ConfigWriteSessions,
            ],
        )?;
        let detail = &prepared.detail;
        let name = text(detail, "name").ok_or("the host holds no name for this change")?;
        let answer = self
            .overlays
            .installed(&json!({"name": name}))
            .map_err(why)?;
        let now = answer
            .pointer(&format!("/overlays/{name}"))
            .and_then(|record| text(record, "digest"))
            .unwrap_or_default();
        if !same_digest(now, &prepared.digest) {
            return Err(
                "the overlay changed since the person was shown it: nothing was removed".to_owned(),
            );
        }
        self.overlays.remove(&json!({"name": name})).map_err(why)?;
        self.files
            .config_update("overlays", true, |current| {
                let mut table = match current {
                    Value::Object(table) => table,
                    _ => Map::new(),
                };
                table.retain(|key, _| !key.eq_ignore_ascii_case(name));
                Ok(if table.is_empty() {
                    Value::Null
                } else {
                    Value::Object(table)
                })
            })
            .map_err(|error| {
                format!(
                    "the overlay's files were removed but its entry in canoa.cfg could not be: {}",
                    why(error)
                )
            })?;
        tracing::info!(overlay = name, "an overlay was removed by a store tool");
        Ok(json!({"overlay": name, "removed": true, "reaches": "sessions started from now on"}))
    }

    // ---- the loadout ----------------------------------------------------------------------------------------------

    fn loadout_digest(section: &Value) -> String {
        format!(
            "loadout:{}",
            hex(&Sha256::digest(section.to_string().as_bytes()))
        )
    }

    fn loadout_change(&self, caller: &Caller, change: &Change) -> Made<Prepared> {
        need(caller, &[Grant::ConfigWrite, Grant::ConfigWriteSessions])?;
        let on = change.action == Action::Enable;
        let name = change.id.as_str();
        let (list, what) = if change.kind == "skill" {
            if !super::overlays::valid_skill_name(name) {
                return Err("that is not a skill name".to_owned());
            }
            ("skills", "a skill you chose")
        } else {
            let section = self.config_section("overlays");
            let named = section
                .as_object()
                .is_some_and(|table| table.keys().any(|key| key.eq_ignore_ascii_case(name)));
            if !named {
                return Err(format!("[overlays] does not name {name}"));
            }
            ("off", "an overlay")
        };
        if change.kind == "skill" && on && !self.overlays.chosen_skill(name) {
            return Err(format!(
                "no skill named {name} is recorded as one the person chose: it is chosen in the Store first"
            ));
        }
        let section = self.config_section("loadout");
        let current = names(section.get(list));
        let held = current.iter().any(|item| item.eq_ignore_ascii_case(name));
        // Skills are on when listed; overlays are off when listed.
        let listed_wanted = if change.kind == "skill" { on } else { !on };
        if held == listed_wanted {
            return Ok(Prepared {
                token: String::new(),
                digest: String::new(),
                title: String::new(),
                lines: Vec::new(),
                nothing: Some(format!(
                    "{name} is already {} in the loadout",
                    if on { "on" } else { "off" }
                )),
                detail: Value::Null,
                protected: None,
            });
        }
        let digest = Self::loadout_digest(&section);
        Ok(Prepared {
            token: String::new(),
            digest: digest.clone(),
            title: format!("Turn {what} {}: {name}", if on { "on" } else { "off" }),
            lines: vec![
                format!("Turn {} the {} {name} in the loadout", if on { "on" } else { "off" }, change.kind),
                "It is kept in canoa.cfg's [loadout] and applies to sessions started after it; running ones keep what they have.".to_owned(),
            ],
            nothing: None,
            protected: self.protected_place(false),
            detail: json!({"do": "loadout", "kind": change.kind, "name": name, "on": on, "list": list}),
        })
    }

    fn loadout_apply(&self, caller: &Caller, prepared: &Prepared) -> Made<Value> {
        need(caller, &[Grant::ConfigWrite, Grant::ConfigWriteSessions])?;
        let detail = &prepared.detail;
        let name = text(detail, "name").ok_or("the host holds no name for this change")?;
        let list = text(detail, "list").ok_or("the host holds no list for this change")?;
        let on = detail.get("on").and_then(Value::as_bool).unwrap_or(false);
        let skill = text(detail, "kind") == Some("skill");
        self.files
            .config_update("loadout", true, |section| {
                if Self::loadout_digest(&section) != prepared.digest {
                    return Err(
                        "the loadout changed since the person was shown it: nothing was changed"
                            .to_owned(),
                    );
                }
                let mut table = match section {
                    Value::Object(table) => table,
                    _ => Map::new(),
                };
                let mut items = names(table.get(list));
                items.retain(|item| !item.eq_ignore_ascii_case(name));
                if skill == on {
                    items.push(name.to_owned());
                }
                items.sort();
                if items.is_empty() {
                    table.remove(list);
                } else {
                    table.insert(list.to_owned(), json!(items));
                }
                Ok(if table.is_empty() {
                    Value::Null
                } else {
                    Value::Object(table)
                })
            })
            .map_err(why)?;
        Ok(json!({"name": name, "on": on, "reaches": "sessions started from now on"}))
    }

    // ---- programs -------------------------------------------------------------------------------------------------

    fn program_change(&self, caller: &Caller, change: &Change) -> Made<Prepared> {
        let entry = change
            .entry
            .as_ref()
            .ok_or("the Store lists no such program")?;
        let provider = text(entry, "provider")
            .ok_or("the entry names no store")?
            .to_owned();
        let title = text(entry, "name").unwrap_or(&provider).to_owned();
        let stores = self.stores().map_err(why)?;
        let managed = stores.managed_version(&provider).map_err(why)?;
        match change.action {
            Action::Install | Action::Update => {
                need(caller, &[Grant::StorePrograms, Grant::StoreRead])?;
                let action = if change.action == Action::Install {
                    "install"
                } else {
                    "update"
                };
                let plan = stores
                    .plan_kept_for(caller.owner(), &provider, action, change.version.as_deref())
                    .map_err(why)?;
                let token = text(&plan, "token").unwrap_or_default().to_owned();
                let digest = format!("plan:{}", hex(&Sha256::digest(plan.to_string().as_bytes())));
                let mut prepared = Prepared {
                    token: token.clone(),
                    digest,
                    title: format!(
                        "{} {title}{}",
                        if change.action == Action::Install {
                            "Install"
                        } else {
                            "Update"
                        },
                        text(&plan, "version")
                            .map_or_else(String::new, |version| format!(" {version}"))
                    ),
                    lines: program_lines(&title, &provider, &plan, managed.as_deref())?,
                    protected: None,
                    nothing: None,
                    detail: json!({"do": "program-install", "provider": provider}),
                };
                if token.is_empty()
                    || plan
                        .get("downloads")
                        .and_then(Value::as_array)
                        .is_none_or(Vec::is_empty)
                {
                    stores.forget(caller.owner(), &provider, &token);
                    prepared.nothing = Some(format!(
                        "{title} has nothing to {action}: it is already current"
                    ));
                }
                Ok(prepared)
            }
            Action::Remove | Action::Rollback => {
                need(caller, &[Grant::StorePrograms])?;
                let Some(version) = managed else {
                    return Ok(Prepared {
                        token: String::new(),
                        digest: String::new(),
                        title: String::new(),
                        lines: Vec::new(),
                        nothing: Some(format!("the host keeps no copy of {title}")),
                        detail: Value::Null,
                        protected: None,
                    });
                };
                let remove = change.action == Action::Remove;
                let previous = stores.previous_version(&provider).map_err(why)?;
                if !remove && previous.is_none() {
                    return Ok(Prepared {
                        token: String::new(),
                        digest: String::new(),
                        title: String::new(),
                        lines: Vec::new(),
                        nothing: Some(format!("{title} has no earlier copy to go back to")),
                        detail: Value::Null,
                        protected: None,
                    });
                }
                let lines = if remove {
                    vec![
                        format!("Remove the copy of {title} the host manages (version {version})"),
                        format!("Store: {provider}"),
                        "What an install changed outside the host's own folder is left as it is."
                            .to_owned(),
                    ]
                } else {
                    vec![
                        format!(
                            "Switch {title} from version {version} back to version {}",
                            previous.as_deref().unwrap_or_default()
                        ),
                        format!("Store: {provider}"),
                    ]
                };
                Ok(Prepared {
                    token: String::new(),
                    digest: match (&previous, remove) {
                        (Some(previous), false) => format!("copy:{version}->{previous}"),
                        _ => format!("copy:{version}"),
                    },
                    title: format!("{} {title}", if remove { "Remove" } else { "Roll back" }),
                    lines,
                    protected: None,
                    nothing: None,
                    detail: json!({"do": if remove { "program-remove" } else { "program-rollback" }, "provider": provider}),
                })
            }
            Action::Enable | Action::Disable => {
                Err("a program is not part of the loadout".to_owned())
            }
        }
    }

    fn program_apply(
        &self,
        caller: &Caller,
        prepared: &Prepared,
        cancel: &Arc<AtomicBool>,
    ) -> Made<Value> {
        need(caller, &[Grant::StorePrograms])?;
        let provider =
            text(&prepared.detail, "provider").ok_or("the host holds no store for this change")?;
        let stores = self.stores().map_err(why)?;
        let kept = stores
            .kept(caller.owner(), provider, &prepared.token)
            .ok_or("the host holds no such plan any more: nothing was changed")?;
        let digest = format!("plan:{}", hex(&Sha256::digest(kept.to_string().as_bytes())));
        if digest != prepared.digest {
            stores.forget(caller.owner(), provider, &prepared.token);
            return Err(
                "the plan is not what the person was shown: nothing was changed".to_owned(),
            );
        }
        stores
            .apply_kept(
                caller.owner(),
                provider,
                &prepared.token,
                Arc::clone(cancel),
            )
            .map_err(why)
    }

    fn program_switch(&self, caller: &Caller, prepared: &Prepared) -> Made<Value> {
        need(caller, &[Grant::StorePrograms])?;
        let provider =
            text(&prepared.detail, "provider").ok_or("the host holds no store for this change")?;
        let stores = self.stores().map_err(why)?;
        let now = stores.managed_version(provider).map_err(why)?;
        let rolling = text(&prepared.detail, "do") == Some("program-rollback");
        let now = match (now, rolling) {
            (Some(version), true) => {
                let previous = stores.previous_version(provider).map_err(why)?;
                previous.map(|previous| format!("copy:{version}->{previous}"))
            }
            (Some(version), false) => Some(format!("copy:{version}")),
            (None, _) => None,
        };
        if now.as_deref() != Some(prepared.digest.as_str()) {
            return Err(
                "the copy changed since the person was shown it: nothing was changed".to_owned(),
            );
        }
        let method = if text(&prepared.detail, "do") == Some("program-remove") {
            "store/remove"
        } else {
            "store/rollback"
        };
        stores
            .change(method, &json!({"provider": provider}))
            .map_err(why)
    }
}

/// What a program's plan says it will do, in lines.
pub(super) fn program_lines(
    title: &str,
    provider: &str,
    plan: &Value,
    managed: Option<&str>,
) -> Made<Vec<String>> {
    let mut lines = vec![
        format!(
            "{} {title}{}",
            text(plan, "action").map_or("Change", |action| match action {
                "install" => "Install",
                "update" => "Update",
                _ => "Change",
            }),
            text(plan, "version")
                .map_or_else(String::new, |version| format!(" to version {version}"))
        ),
        format!("Store: {provider}"),
    ];
    if let Some(from) = text(plan, "from_version").or(managed) {
        lines.push(format!("Now installed: {from}"));
    }
    // What the plan does to the computer comes first and is shown whole: a card that cannot hold it is not shown.
    let changes: Vec<String> = plan
        .get("changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|change| {
            format!(
                "{}: {} ({})",
                text(change, "kind").unwrap_or_default(),
                text(change, "target").unwrap_or_default(),
                text(change, "detail").unwrap_or_default()
            )
        })
        .collect();
    if changes.is_empty() {
        lines.push("It changes nothing outside the host's own folder.".to_owned());
    } else {
        lines.push("It changes, outside the host's own folder:".to_owned());
        listed_whole(&mut lines, "", changes)?;
    }
    let steps = names(plan.get("steps"));
    if !steps.is_empty() {
        lines.push("It runs, as this user:".to_owned());
        listed_whole(&mut lines, "", steps)?;
    }
    let notices = names(plan.get("notices"));
    if !notices.is_empty() {
        lines.push("Notices from the store:".to_owned());
        listed_whole(&mut lines, "", notices)?;
    }
    if let Some(bytes) = plan.get("bytes").and_then(Value::as_u64) {
        lines.push(format!("Downloads about {bytes} bytes"));
    }
    let downloads = plan
        .get("downloads")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    lines.push(format!("It fetches {} file(s):", downloads.len()));
    listed(
        &mut lines,
        "",
        downloads.iter().map(|download| {
            format!(
                "{} ({})",
                text(download, "url").unwrap_or_default(),
                download
                    .get("check")
                    .map_or_else(|| "unchecked".to_owned(), Value::to_string)
            )
        }),
    );
    Ok(lines)
}
