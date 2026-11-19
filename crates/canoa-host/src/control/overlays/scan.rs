//! `skill/scan`: which skills a git source holds, found the way `npx skills` finds them and listed from their `SKILL.md` front
//! matter alone. The source is fetched one commit deep into a scratch repository and read from git's objects; no skill's other
//! file is read, nothing is installed and the scratch repository is removed.

use std::ffi::OsString;

use coqui_aos::{
    Case, EXCLUDED_COMPONENTS, Format, Settings, read_front_matter, valid_relative_path,
};
use serde_json::{Value, json};

use super::{Outcome, Overlays, refused, unique, valid_skill_name};

/// The most entries a source's tree may list before a scan stops: a source this large names the folder to read.
const MAX_ENTRIES: usize = 262_144;
/// The most skills one scan lists.
const MAX_SKILLS: usize = 1024;
/// The most bytes of a `SKILL.md` a scan reads for its front matter.
const MAX_SKILL_MD: u64 = 256 * 1024;
/// The most characters of a description a scan carries.
const MAX_DESCRIPTION: usize = 1024;
/// How deep below the folder scanned a `SKILL.md` is looked for when none is in the usual places.
const MAX_DEPTH: usize = 5;

/// The folders whose subfolders are skills, which `npx skills` reads first: its own `skills/`, the three folders under it, the
/// `.agents/skills` and `.agent/skills` that agents share, and the folder each loaded agent says it reads skills from.
fn skill_folders() -> Vec<String> {
    let mut folders: Vec<String> = [
        "skills",
        "skills/.curated",
        "skills/.experimental",
        "skills/.system",
        ".agents/skills",
        ".agent/skills",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for folder in crate::vendors::all()
        .into_iter()
        .filter_map(|vendor| vendor.skills_folder)
    {
        if !folders.iter().any(|held| held == folder) {
            folders.push(folder.to_owned());
        }
    }
    folders
}

/// One `SKILL.md` the tree holds: its folder below the folder scanned (empty for that folder itself), its blob and its size.
struct Found {
    folder: String,
    blob: String,
    size: u64,
}

impl Overlays {
    /// `skill/scan` `{"repo", "ref"?, "path"?, "own"?}` -> `{"commit", "skills": [{name, description, path, problem?}], "skipped",
    /// "truncated"}`: the skills the source holds at one commit. `path` is the folder of the source to read, the whole of it when
    /// empty; each skill's `path` is from the source's root, as `skill/fetch` takes it. A skill whose name is no skill name or
    /// is not its folder's carries a `problem` and is not to be installed. `skipped` counts what was left out: a `SKILL.md`
    /// that is no skill (no front matter naming it and saying what it does), one marked internal, and a second of a name.
    pub(crate) fn skill_scan(&self, params: &Value, local: &dyn Fn(&str) -> bool) -> Outcome {
        let named = Self::skill_source(params, local)?;
        self.with_scratch(|cache| {
            let commit = self.fetch_commit(cache, &named)?;
            let found = self.skill_files_of(cache, &commit, &named.path)?;
            let (skills, skipped, truncated) = self.read_skills(cache, &named.path, found)?;
            Ok(json!({
                "commit": commit,
                "skills": skills,
                "skipped": skipped,
                "truncated": truncated,
            }))
        })
    }

    /// Every `SKILL.md` below `path` at `commit`, from one `ls-tree`, chosen as `npx skills` chooses: the folder's own, else
    /// those of the usual folders, else any within [`MAX_DEPTH`] folders.
    fn skill_files_of(
        &self,
        cache: &std::path::Path,
        commit: &str,
        path: &str,
    ) -> Result<Vec<Found>, (i32, String)> {
        let mut args: Vec<OsString> = vec![
            format!("--git-dir={}", cache.display()).into(),
            "ls-tree".into(),
            "-r".into(),
            "-l".into(),
            "-z".into(),
            "--full-tree".into(),
            commit.into(),
            "--".into(),
        ];
        if !path.is_empty() {
            args.push(path.into());
        }
        let out = self.run(&args, None, None, 64 * 1024 * 1024)?;
        let prefix = if path.is_empty() {
            String::new()
        } else {
            format!("{path}/")
        };
        let mut found = Vec::new();
        for (count, record) in out
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty())
            .enumerate()
        {
            if count >= MAX_ENTRIES {
                return Err(refused(format!(
                    "The source holds more than {MAX_ENTRIES} entries: name a folder of it"
                )));
            }
            let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
                continue;
            };
            let Ok(full) = std::str::from_utf8(&record[tab + 1..]) else {
                continue;
            };
            let Some(relative) = full.strip_prefix(&prefix) else {
                continue;
            };
            let (folder, name) = relative.rsplit_once('/').unwrap_or(("", relative));
            if name != "SKILL.md" {
                continue;
            }
            let head = String::from_utf8_lossy(&record[..tab]).into_owned();
            let fields: Vec<&str> = head.split_whitespace().collect();
            let [mode, kind, blob, size] = fields[..] else {
                continue;
            };
            // A link is no file; it is refused when the skill is fetched, if it is ever asked for.
            if kind != "blob" || mode == "120000" {
                continue;
            }
            if folder
                .split('/')
                .any(|part| EXCLUDED_COMPONENTS.contains(&part))
            {
                continue;
            }
            found.push(Found {
                folder: folder.to_owned(),
                blob: blob.to_owned(),
                size: size.parse().unwrap_or(0),
            });
        }
        found.sort_by(|a, b| a.folder.cmp(&b.folder));
        Ok(choose(found))
    }

    /// The listing of each `SKILL.md` in `found`: read from its blob, its front matter alone.
    fn read_skills(
        &self,
        cache: &std::path::Path,
        path: &str,
        found: Vec<Found>,
    ) -> Result<(Vec<Value>, usize, bool), (i32, String)> {
        let truncated = found.len() > MAX_SKILLS;
        let readable: Vec<&Found> = found
            .iter()
            .filter(|each| each.size <= MAX_SKILL_MD)
            .take(MAX_SKILLS)
            .collect();
        let mut skipped = found.len().min(MAX_SKILLS) - readable.len();
        let ids = unique(readable.iter().map(|each| &each.blob));
        let blobs = self.blobs(cache, &ids)?;
        let mut listed: Vec<(bool, String, Value)> = Vec::new();
        for each in readable {
            let Some(bytes) = blobs.get(&each.blob) else {
                return Err(refused("git did not give a file it was asked for"));
            };
            let Some(Said {
                name,
                description,
                internal,
            }) = front_matter_of(bytes)
            else {
                skipped += 1;
                continue;
            };
            if internal {
                skipped += 1;
                continue;
            }
            let full = match (path.is_empty(), each.folder.is_empty()) {
                (_, true) => path.to_owned(),
                (true, false) => each.folder.clone(),
                (false, false) => format!("{path}/{}", each.folder),
            };
            let mut entry = json!({"name": name, "description": description, "path": full});
            // The folder is what a skill's name has to be, as the Agent Skills specification says; a skill at the root has none.
            let folder = full.rsplit('/').next().unwrap_or_default();
            let problem = if !(valid_skill_name(&name) && valid_relative_path(&name)) {
                Some("its name is not one a skill may have".to_owned())
            } else if !folder.is_empty() && folder != name {
                Some(format!(
                    "its name is {name}, and the name has to be its folder's, {folder}"
                ))
            } else {
                None
            };
            let sound = problem.is_none();
            if let Some(problem) = problem {
                entry["problem"] = Value::String(problem);
            }
            listed.push((sound, name, entry));
        }
        // Two skills of a name: the one a person can install keeps it, and else the first folder.
        listed.sort_by_key(|(sound, ..)| !sound);
        let mut kept: Vec<(String, Value)> = Vec::new();
        for (_, name, entry) in listed {
            if kept.iter().any(|(each, _)| *each == name) {
                skipped += 1;
            } else {
                kept.push((name, entry));
            }
        }
        kept.sort_by_key(|(name, _)| name.to_lowercase());
        Ok((
            kept.into_iter().map(|(_, entry)| entry).collect(),
            skipped,
            truncated,
        ))
    }
}

/// The `SKILL.md`s `npx skills` lists of `found`, in folder order: the folder's own alone when it has one, else those directly
/// inside the usual folders, else any within [`MAX_DEPTH`] folders.
fn choose(found: Vec<Found>) -> Vec<Found> {
    if found.iter().any(|each| each.folder.is_empty()) {
        return found
            .into_iter()
            .filter(|each| each.folder.is_empty())
            .collect();
    }
    // A skill directly inside `skills/` has the folder `skills/<name>`, whose parent is a usual folder.
    let folders = skill_folders();
    let usual = |each: &Found| {
        each.folder
            .rsplit_once('/')
            .is_some_and(|(parent, _)| folders.iter().any(|held| held == parent))
    };
    if found.iter().any(usual) {
        return found.into_iter().filter(usual).collect();
    }
    found
        .into_iter()
        .filter(|each| each.folder.split('/').count() <= MAX_DEPTH)
        .collect()
}

/// What a `SKILL.md`'s front matter says of the skill.
struct Said {
    name: String,
    description: String,
    /// `metadata.internal` is true: the skill is its source's own, not offered.
    internal: bool,
}

/// What `bytes` say of a skill, or none when its front matter is not YAML naming it and saying what it does.
fn front_matter_of(bytes: &[u8]) -> Option<Said> {
    let read = read_front_matter(bytes, &Settings::default());
    if read.error.is_some() || read.case != Case::Mapping || read.format != Some(Format::Yaml) {
        return None;
    }
    let data = read.data?;
    let name = data.get("name")?.as_str()?.to_owned();
    let description: String = data
        .get("description")?
        .as_str()?
        .chars()
        .filter(|c| !c.is_control() || c.is_whitespace())
        .take(MAX_DESCRIPTION)
        .collect();
    if name.is_empty() || name.len() > 256 || description.trim().is_empty() {
        return None;
    }
    let internal = data
        .get("metadata")
        .and_then(Value::as_object)
        .and_then(|metadata| metadata.get("internal"))
        .is_some_and(|internal| internal == &Value::Bool(true));
    Some(Said {
        name,
        description,
        internal,
    })
}
