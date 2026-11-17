//! `store/read_ahead`: what the entries of a catalog provide, read here so the app asks once for a batch and holds none of the
//! fetching, reading or discarding. Nothing is installed and nothing runs; each overlay is fetched as a review fetches it and
//! discarded when nothing is installed under its name.

use std::collections::BTreeMap;

use coqui_aos::{
    Document, INTERPRETERS, OverlayTree, Settings, SkillOrigin, default_on, dependencies_of,
    hooks_of, read_front_matter, read_overlay, rules_of, skill_entries,
};
use serde_json::{Value, json};

use super::{Caller, Outcome, Overlays, PoisonError, bad, refused, text_param};

/// The most entries one call reads, so one answer and one request stay a size a link carries.
const MAX_ENTRIES: usize = 32;

const SCRIPT_EXTENSIONS: [&str; 7] = [".py", ".js", ".mjs", ".cjs", ".ps1", ".bash", ".sh"];

impl Overlays {
    /// `store/read_ahead` `{"os"?, "entries": [{"id", "kind": "overlay"|"skill", "name", "repo", "ref", "path"?}]}` ->
    /// `{"entries": {id: {"contents": {...}} | {"problem": text}}}`. An entry that cannot be read is noted against it and the rest go
    /// on. `local` says whether a source on this computer may be read.
    pub(crate) fn read_ahead(
        &self,
        caller: &Caller,
        params: &Value,
        local: &dyn Fn(&str) -> bool,
    ) -> Outcome {
        let os = text_param(params, "os")?.unwrap_or("linux");
        let entries = params
            .get("entries")
            .and_then(Value::as_array)
            .ok_or_else(|| bad("entries is a list"))?;
        if entries.len() > MAX_ENTRIES {
            return Err(bad(&format!("entries holds at most {MAX_ENTRIES}")));
        }
        let mut answer = serde_json::Map::new();
        for entry in entries {
            let id = text_param(entry, "id")?.ok_or_else(|| bad("an entry needs an id"))?;
            let kind = text_param(entry, "kind")?.ok_or_else(|| bad("an entry needs a kind"))?;
            let name = text_param(entry, "name")?.ok_or_else(|| bad("an entry needs a name"))?;
            let read = match kind {
                "overlay" => self.read_overlay_entry(caller, entry, name, os, local),
                "skill" => self.read_skill_entry(entry, name, local),
                _ => Err(bad("kind is overlay or skill")),
            };
            let said = match read {
                Ok(contents) => json!({"contents": contents}),
                Err((_, problem)) => json!({"problem": problem}),
            };
            answer.insert(id.to_owned(), said);
        }
        Ok(json!({"entries": answer}))
    }

    /// One overlay: fetched and checked under its name, read, then discarded. Reading the same name is one at a time.
    fn read_overlay_entry(
        &self,
        caller: &Caller,
        entry: &Value,
        name: &str,
        os: &str,
        local: &dyn Fn(&str) -> bool,
    ) -> Result<Value, (i32, String)> {
        let gate = self.gate(&format!("read-ahead:{name}"));
        let _one = gate.lock().unwrap_or_else(PoisonError::into_inner);
        let mut asked = json!({"name": name});
        for key in ["repo", "ref", "path"] {
            if let Some(value) = entry.get(key) {
                asked[key] = value.clone();
            }
        }
        let planned = self.check(caller, &asked, local)?;
        let token = planned
            .get("token")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let digest = planned
            .get("digest")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let read = self.read(caller, &json!({"token": token}));
        self.forget_plan(caller, &token);
        let _ = self.discard(&json!({"name": name}));
        let files = decoded(read?)?;
        let label = entry_name(entry, name);
        contents_of(&files, label, os, &digest)
    }

    /// One skill: its files fetched from its own source and read for the description its SKILL.md gives.
    fn read_skill_entry(
        &self,
        entry: &Value,
        name: &str,
        local: &dyn Fn(&str) -> bool,
    ) -> Result<Value, (i32, String)> {
        let mut asked = json!({});
        for key in ["repo", "ref", "path"] {
            if let Some(value) = entry.get(key) {
                asked[key] = value.clone();
            }
        }
        let fetched = self.skill_fetch(&asked, local)?;
        let files = decoded(fetched)?;
        let description = files
            .get("SKILL.md")
            .and_then(|bytes| description_of(bytes));
        Ok(json!({"skills": [skill_json(name, description)]}))
    }
}

fn entry_name<'a>(entry: &'a Value, fallback: &'a str) -> &'a str {
    entry
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or(fallback)
}

/// The `files` of an answer, decoded.
fn decoded(answer: Value) -> Result<BTreeMap<String, Vec<u8>>, (i32, String)> {
    let files = answer
        .get("files")
        .and_then(Value::as_object)
        .ok_or_else(|| refused("the host gave no files"))?;
    files
        .iter()
        .map(|(path, data)| {
            let bytes = data
                .as_str()
                .and_then(|text| data_encoding::BASE64.decode(text.as_bytes()).ok())
                .ok_or_else(|| refused("a file came back that is not base64"))?;
            Ok((path.clone(), bytes))
        })
        .collect()
}

fn skill_json(name: &str, description: Option<String>) -> Value {
    match description {
        Some(text) => json!({"name": name, "description": text}),
        None => json!({"name": name}),
    }
}

/// The description a SKILL.md's front matter gives, when it has one.
fn description_of(file: &[u8]) -> Option<String> {
    let read = read_front_matter(file, &Settings::default());
    let text = read.data?.get("description")?.as_str()?.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// The skill folders a source with no manifest holds by the conventions of skill repositories: `skills/NAME/SKILL.md`, or
/// `NAME/SKILL.md` at the root, or a SKILL.md at the root alone (named `fallback`).
fn skill_directories(files: &BTreeMap<String, Vec<u8>>, fallback: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for path in files.keys() {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.last() != Some(&"SKILL.md") {
            continue;
        }
        match parts.as_slice() {
            ["skills", name, _] | [name, _] => found.push(((*name).to_owned(), path.clone())),
            [_] => found.push((fallback.to_owned(), path.clone())),
            _ => {}
        }
    }
    found
}

/// The overlay.cfg Canoa generates for a source that has none, from what it holds.
fn generated_manifest(name: &str, skills: &[(String, Option<String>)]) -> String {
    let quote = |text: &str| {
        format!(
            "\"{}\"",
            text.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', " ")
        )
    };
    let first = skills
        .iter()
        .find_map(|(_, description)| description.clone());
    let summary = first.unwrap_or_else(|| {
        if skills.is_empty() {
            name.to_owned()
        } else {
            format!("{} skills", skills.len())
        }
    });
    [
        "# Generated by Canoa: this source has no overlay.cfg. It is read, never written into the source.".to_owned(),
        format!("name = {}", quote(name)),
        String::new(),
        "[about]".to_owned(),
        format!("summary = {}", quote(&summary)),
        String::new(),
    ]
    .join("\n")
}

fn texts(manifest: &Document, key: &str) -> Vec<String> {
    manifest
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("path")?.as_str().map(str::to_owned))
        .collect()
}

/// What the overlay whose files are `files` provides, as the document the app keeps: its skills, rules and context, knowledge,
/// hooks, environment scripts, setup, MCP servers, `bin` commands and requirements. `name` is the entry's name, for the manifest
/// generated when the files hold none.
fn contents_of(
    files: &BTreeMap<String, Vec<u8>>,
    name: &str,
    os: &str,
    digest: &str,
) -> Result<Value, (i32, String)> {
    let mut tree = OverlayTree::from_paths(files.keys().cloned());
    for (path, bytes) in files {
        tree = tree.with_bytes(path, bytes.clone());
    }
    let paths = OverlayTree::from_paths(files.keys().cloned());
    let has_manifest = files.contains_key("OVERLAY.md") || files.contains_key("overlay.cfg");
    let mut manifest = Document::new();
    let mut context = Vec::new();
    let mut issues = Vec::new();
    if has_manifest {
        let read = read_overlay(&tree, &Settings::default()).map_err(|error| {
            refused(format!(
                "Its files cannot be read as an overlay ({})",
                error.class.as_str()
            ))
        })?;
        manifest = read.manifest;
        context = read.context;
        issues = read.issues;
    }
    let skill_of = |path: &str| files.get(path).and_then(|bytes| description_of(bytes));

    let conventions = skill_directories(files, name);
    let skills: Vec<(String, Option<String>)> = if has_manifest || conventions.is_empty() {
        skill_entries(&manifest, &paths)
            .into_iter()
            .map(|entry| {
                let description = (entry.origin == SkillOrigin::Bundled)
                    .then(|| skill_of(&format!("skills/{}/SKILL.md", entry.name)))
                    .flatten();
                (entry.name, description)
            })
            .collect()
    } else {
        conventions
            .iter()
            .map(|(skill, path)| (skill.clone(), skill_of(path)))
            .collect()
    };

    let served = manifest
        .get("knowledge")
        .and_then(|knowledge| knowledge.get("serve"))
        == Some(&Value::Bool(true));
    let rules = rules_of(&manifest, &paths);
    let mut knowledge: Vec<String> = Vec::new();
    for path in rules.iter().chain(context.iter()) {
        if path != "OVERLAY.md" && !knowledge.contains(path) {
            knowledge.push(path.clone());
        }
    }

    let interpreters: Vec<String> = INTERPRETERS.iter().map(|each| (*each).to_owned()).collect();
    let (found, not_run) = hooks_of(&manifest, &paths, os, &interpreters);
    let mut hooks: Vec<String> = found
        .iter()
        .map(|hook| match &hook.path {
            Some(path) => format!("{}: {path}", hook.event.as_str()),
            None => hook.event.as_str().to_owned(),
        })
        .collect();
    hooks.extend(
        not_run
            .iter()
            .map(|path| format!("{path} (no interpreter to run it)")),
    );

    let mut scripts = texts(&manifest, "env_scripts");
    let mut setup = texts(&manifest, "setup");
    if default_on(&manifest, "env_scripts") {
        for extension in SCRIPT_EXTENSIONS {
            let path = format!("env{extension}");
            if files.contains_key(&path) && !scripts.contains(&path) {
                scripts.push(path);
            }
        }
    }
    if default_on(&manifest, "setup") {
        for extension in SCRIPT_EXTENSIONS {
            let path = format!("setup{extension}");
            if files.contains_key(&path) && !setup.contains(&path) {
                setup.push(path);
            }
        }
    }

    let mcp: Vec<String> = manifest
        .get("mcp")
        .and_then(|mcp| mcp.get("servers"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|server| server.is_object())
        .map(|server| {
            let field = |key: &str, otherwise: &str| {
                server
                    .get(key)
                    .and_then(Value::as_str)
                    .unwrap_or(otherwise)
                    .to_owned()
            };
            format!("{} ({})", field("name", ""), field("transport", "stdio"))
        })
        .collect();

    let bin: Vec<String> = files
        .keys()
        .filter_map(|path| path.strip_prefix("bin/"))
        .filter(|rest| !rest.contains('/'))
        .map(str::to_owned)
        .collect();

    let mut requirements: Vec<String> = manifest
        .get("requires")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|item| match item.as_str() {
            Some(text) => format!("overlay {text}"),
            None => format!("overlay {item}"),
        })
        .collect();
    if let Ok(dependencies) = dependencies_of(&manifest, &paths, os) {
        requirements.extend(
            dependencies
                .iter()
                .map(|dependency| format!("{}: {}", dependency.runtime.as_str(), dependency.file)),
        );
    }

    let skill_values: Vec<Value> = skills
        .iter()
        .map(|(skill, description)| skill_json(skill, description.clone()))
        .collect();
    let mut contents = json!({
        "skills": skill_values,
        "rules": rules,
        "context": context,
        "knowledge": knowledge.iter().map(|path| json!({"path": path, "served": served})).collect::<Vec<_>>(),
        "hooks": hooks,
        "env_scripts": scripts,
        "setup": setup,
        "mcp_servers": mcp,
        "bin": bin,
        "requirements": requirements,
        "generated": !has_manifest,
        "digest": digest,
        "issues": issues,
    });
    if !has_manifest {
        contents["manifest"] = Value::String(generated_manifest(name, &skills));
    }
    Ok(contents)
}
