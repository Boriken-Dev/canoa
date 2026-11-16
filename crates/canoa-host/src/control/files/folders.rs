//! What a client asks about the folders of this computer beyond browsing them: whether a folder is there and in a git repository,
//! the names an `@` offers inside a session's folder, one name resolved inside it, and the scratch folder a chat works in.
//!
//! Each is behind the grant `fs/list` is behind (`folders`) and keeps to the same places: a key holding `files` anywhere this
//! account can reach, any other key the home and the working roots of its entries; the person's own app, the whole computer.

#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and processes are std's"
)]

use crate::spawn::{Command, Stdio};
use std::fs;
use std::io::Read as _;
use std::path::{MAIN_SEPARATOR_STR, Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::{Files, Outcome, confined, plain, text_of};
use crate::control::{Caller, Grant, NOT_FOUND, REFUSED, bad, text_param};

/// The most folders one `fs/info` answers for.
pub const MAX_INFO: usize = 64;
/// The entries `fs/mentions` lists unless the request names another count, and the most it lists whatever it asks.
const DEFAULT_ENTRIES: usize = 5000;
const MAX_ENTRIES: usize = 20_000;
/// The levels `fs/mentions` walks outside a repository, and the most it walks whatever it asks.
const DEFAULT_DEPTH: usize = 8;
const MAX_DEPTH: usize = 32;
/// What git may print, and how long it may take, before the folder is walked instead.
const GIT_OUTPUT: usize = 16 * 1024 * 1024;
const GIT_TIME: Duration = Duration::from_secs(10);
/// The most characters of an expanded value that is one path segment.
const SEGMENT: usize = 100;
/// `[sessions] workspace` when canoa.cfg sets none.
const DEFAULT_TEMPLATE: &str = "${documents}/Canoa/scratch/${session}";
/// A session id to show what a template makes before any session has one.
const EXAMPLE_SESSION: &str = "3f2a9c1e-7b4d-4e58-9a16-0d5c2b8e6f40";
/// What a template may name.
const VARIABLES: [&str; 10] = [
    "documents",
    "home",
    "session",
    "date",
    "time",
    "datetime",
    "handler",
    "source",
    "host",
    "fqdn",
];
const FOLDER_VARIABLES: [&str; 2] = ["documents", "home"];

impl Files {
    /// The folder or file `asked` names, as `caller` may see it: resolved, and for a key not holding `files` under a place it may
    /// browse, as `fs/list` keeps to them; a share is a place like any other, reached through the operating system.
    fn place(&self, caller: &Caller, asked: &str) -> Result<PathBuf, (i32, String)> {
        let roots = self.browse_roots(caller);
        if caller.may(Grant::Files) {
            if !Path::new(asked).is_absolute() {
                return Err(bad("a path must be absolute: it starts at a drive or at /"));
            }
            return fs::canonicalize(asked).map(plain).map_err(|_| {
                (
                    NOT_FOUND,
                    "there is no such place, or this account cannot see it".to_owned(),
                )
            });
        }
        confined(asked, &roots)
    }

    /// Whether `place` is inside the places `caller` browses, for a root a key may be told about.
    fn within(&self, caller: &Caller, place: &Path) -> bool {
        caller.may(Grant::Files)
            || self
                .browse_roots(caller)
                .iter()
                .any(|root| place.starts_with(root))
    }

    /// `fs/info`: for each of `paths` (or the one `path`), whether it is there, whether it is a folder, and the git repository it
    /// is in: `{path, exists, folder, repository, root}`. A place the key may not see is not there, and says so.
    pub(crate) fn info(&self, caller: &Caller, params: &Value) -> Outcome {
        let mut asked: Vec<&str> = Vec::new();
        match params.get("paths") {
            None | Some(Value::Null) => {}
            Some(Value::Array(list)) => {
                for each in list {
                    asked.push(
                        each.as_str()
                            .ok_or_else(|| bad("each of paths is a path as text"))?,
                    );
                }
            }
            Some(_) => return Err(bad("paths is a list of paths")),
        }
        if let Some(one) = text_param(params, "path")? {
            asked.push(one);
        }
        if asked.is_empty() {
            return Err(bad("path or paths is needed"));
        }
        if asked.len() > MAX_INFO {
            return Err(bad(&format!("at most {MAX_INFO} paths in one request")));
        }
        let found: Vec<Value> = asked
            .into_iter()
            .map(|path| match self.place(caller, path) {
                Ok(place) => {
                    let metadata = fs::metadata(&place);
                    let folder = metadata.as_ref().is_ok_and(fs::Metadata::is_dir);
                    let root = folder.then(|| repository_root(&place)).flatten();
                    json!({
                        "path": path,
                        "exists": metadata.is_ok(),
                        "folder": folder,
                        "repository": root.is_some(),
                        "root": root
                            .filter(|root| self.within(caller, root))
                            .map(|root| text_of(&root)),
                    })
                }
                Err((_, why)) => json!({
                    "path": path,
                    "exists": false,
                    "folder": false,
                    "repository": false,
                    "root": null,
                    "why": why,
                }),
            })
            .collect();
        Ok(json!({"folders": found}))
    }

    /// `fs/mentions`: the files and folders under `path` as paths relative to it with `/` (a folder ends in `/`), nearest first.
    /// In a git repository they are the files git lists by the repository's own rules, tracked or untracked and not ignored;
    /// elsewhere the folder is walked. Links are neither followed nor listed, names with control or direction characters are left
    /// out, and at most `max_entries` are read and `max_depth` levels walked.
    pub(crate) fn mentions(&self, caller: &Caller, params: &Value) -> Outcome {
        let asked =
            text_param(params, "path")?.ok_or_else(|| bad("a path is needed: the folder"))?;
        let entries = count(params, "max_entries", DEFAULT_ENTRIES, MAX_ENTRIES)?;
        let depth = count(params, "max_depth", DEFAULT_DEPTH, MAX_DEPTH)?;
        let folder = self.place(caller, asked)?;
        if !fs::metadata(&folder).is_ok_and(|meta| meta.is_dir()) {
            return Err((NOT_FOUND, "that is not a folder".to_owned()));
        }
        let listed = if repository_root(&folder).is_some() {
            git_listing(&folder, entries)
        } else {
            None
        };
        let listed = listed.unwrap_or_else(|| walk_listing(&folder, entries, depth));
        Ok(json!({"path": text_of(&folder), "entries": listed}))
    }

    /// `fs/resolve`: the file or folder `name` (relative, with `/`) names inside `path`, as `{path, folder, size}`. Anything that
    /// climbs, is absolute, has a drive, a backslash or a control character, is not there, or is or leads through a link out of
    /// the folder is not found.
    pub(crate) fn resolve(&self, caller: &Caller, params: &Value) -> Outcome {
        let asked =
            text_param(params, "path")?.ok_or_else(|| bad("a path is needed: the folder"))?;
        let name = text_param(params, "name")?
            .ok_or_else(|| bad("a name is needed: the path inside it"))?;
        let folder = self.place(caller, asked)?;
        let outside = || {
            (
                NOT_FOUND,
                "that is not a file or folder inside this folder".to_owned(),
            )
        };
        let parts = relative_parts(name).ok_or_else(outside)?;
        let mut candidate = folder.clone();
        for part in &parts {
            candidate.push(part);
        }
        let metadata = fs::metadata(&candidate).map_err(|_| outside())?;
        if !metadata.is_dir() && !metadata.is_file() {
            return Err(outside());
        }
        let real = fs::canonicalize(&candidate)
            .map(plain)
            .map_err(|_| outside())?;
        if !inside(&folder, &real) {
            return Err(outside());
        }
        Ok(json!({
            "path": text_of(&candidate),
            "folder": metadata.is_dir(),
            "size": if metadata.is_dir() { 0 } else { metadata.len() },
        }))
    }

    /// `fs/scratch`, `fs/scratch_remove` and `fs/scratch_preview`: the folder the person's `[sessions] workspace` template makes
    /// for a chat, made (or taken away again if it is empty), or only worked out for a template that is being edited.
    pub(crate) fn scratch(&self, params: &Value, how: Scratch) -> Outcome {
        let preview = how == Scratch::Preview;
        let template = if preview {
            text_param(params, "template")?
                .ok_or_else(|| bad("a template is needed"))?
                .to_owned()
        } else {
            self.workspace_template()
        };
        if let Some(problem) = workspace_problem(&template) {
            return Err(bad(&format!(
                "the workspace template is not usable: {problem}"
            )));
        }
        let session = match text_param(params, "session")? {
            Some(id) if session_id(id) => id.to_owned(),
            Some(_) => {
                return Err(bad(
                    "session is letters, digits and hyphens, at most 36 characters",
                ));
            }
            None if preview => EXAMPLE_SESSION.to_owned(),
            None => return Err(bad("a session id is needed")),
        };
        let values = Values {
            documents: self
                .documents
                .clone()
                .or_else(crate::site::system_documents)
                .map(|folder| text_of(&folder))
                .unwrap_or_default(),
            home: std::env::home_dir()
                .map(|folder| text_of(&folder))
                .unwrap_or_default(),
            session,
            when: when_of(text_param(params, "now")?)?,
            handler: text_param(params, "handler")?
                .unwrap_or_default()
                .to_owned(),
            source: text_param(params, "source")?.unwrap_or_default().to_owned(),
            host: crate::native::computer_name().unwrap_or_default(),
        };
        let path = expand_workspace(&template, &values, MAIN_SEPARATOR_STR);
        if preview {
            return Ok(json!({"path": path}));
        }
        let folder = PathBuf::from(&path);
        if !folder.is_absolute() {
            return Err((
                REFUSED,
                format!(
                    "the workspace template makes {path}, which is not a place on this computer: it needs ${{documents}} or a drive"
                ),
            ));
        }
        if how == Scratch::Remove {
            // Only an empty folder, and only the one this template makes for this session.
            let removed = fs::remove_dir(&folder).is_ok();
            return Ok(json!({"path": path, "removed": removed}));
        }
        let made = !folder.exists();
        crate::site::make_private(&folder).map_err(|error| {
            (
                REFUSED,
                format!("the workspace folder could not be made: {error}"),
            )
        })?;
        Ok(json!({"path": path, "made": made}))
    }

    /// `[sessions] workspace` from the person's canoa.cfg, or the default when it has none or cannot be read.
    fn workspace_template(&self) -> String {
        self.config_get(&json!({"section": "sessions"}))
            .ok()
            .and_then(|answer| answer["value"]["workspace"].as_str().map(str::to_owned))
            .unwrap_or_else(|| DEFAULT_TEMPLATE.to_owned())
    }
}

/// Which of the three scratch calls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Scratch {
    Make,
    Remove,
    Preview,
}

fn count(params: &Value, key: &str, default: usize, most: usize) -> Result<usize, (i32, String)> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(value) => value
            .as_u64()
            .and_then(|number| usize::try_from(number).ok())
            .filter(|number| *number > 0)
            .map(|number| number.min(most))
            .ok_or_else(|| bad(&format!("{key} is a whole number above zero"))),
    }
}

/// The folder at or above `folder` that holds `.git` (a folder or a file, as a worktree has), if any.
fn repository_root(folder: &Path) -> Option<PathBuf> {
    folder
        .ancestors()
        .find(|dir| fs::symlink_metadata(dir.join(".git")).is_ok())
        .map(Path::to_path_buf)
}

/// Whether `real` is below `root` once both are resolved, compared as the file system compares them.
fn inside(root: &Path, real: &Path) -> bool {
    let root = fs::canonicalize(root)
        .map(plain)
        .unwrap_or_else(|_| root.to_path_buf());
    if cfg!(windows) {
        let (root, real) = (text_of(&root).to_lowercase(), text_of(real).to_lowercase());
        let root = root.trim_end_matches('\\');
        real.strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('\\'))
    } else {
        real != root && real.starts_with(&root)
    }
}

/// Whether `name` is a name an `@` may offer: nothing that hides or reorders text.
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && !name.chars().any(|c| {
            c.is_control()
                || matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
        })
}

/// `name` split at `/` into the parts that name something inside a folder, or none when it names anything else.
fn relative_parts(name: &str) -> Option<Vec<&str>> {
    if name.is_empty() || !safe_name(name) || name.contains('\\') || name.starts_with('/') {
        return None;
    }
    let mut chars = name.chars();
    if let (Some(first), Some(':')) = (chars.next(), chars.next())
        && first.is_ascii_alphabetic()
    {
        return None;
    }
    let mut parts: Vec<&str> = name.split('/').collect();
    if parts.last().is_some_and(|last| last.is_empty()) {
        parts.pop();
    }
    if parts.is_empty()
        || parts
            .iter()
            .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return None;
    }
    Some(parts)
}

fn walk_listing(folder: &Path, max_entries: usize, max_depth: usize) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut queue: Vec<(PathBuf, String, usize)> = vec![(folder.to_path_buf(), String::new(), 0)];
    let mut next = 0;
    while next < queue.len() && found.len() < max_entries {
        let (dir, prefix, depth) = queue[next].clone();
        next += 1;
        let Ok(reader) = fs::read_dir(&dir) else {
            continue;
        };
        let mut here: Vec<(String, bool)> = Vec::new();
        for entry in reader.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if kind.is_symlink() || !safe_name(&name) {
                continue;
            }
            if kind.is_dir() {
                here.push((name, true));
            } else if kind.is_file() {
                here.push((name, false));
            }
        }
        here.sort();
        for (name, is_folder) in here {
            if found.len() >= max_entries {
                break;
            }
            found.push(if is_folder {
                format!("{prefix}{name}/")
            } else {
                format!("{prefix}{name}")
            });
            if is_folder && depth < max_depth {
                queue.push((dir.join(&name), format!("{prefix}{name}/"), depth + 1));
            }
        }
    }
    found
}

/// What `git ls-files` lists in `folder`, as files and their folders, or none when git is not there, fails or takes too long. git
/// is found along `PATH` and started with nothing from the environment or the repository's configuration that could run a
/// program, nothing asked of the person, and no optional lock.
fn git_listing(folder: &Path, max_entries: usize) -> Option<Vec<String>> {
    let git = super::super::overlays::find_git().ok()?;
    let mut command = Command::new(git);
    for (name, _) in std::env::vars_os() {
        if name
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_")
        {
            command.env_remove(name);
        }
    }
    let mut child = command
        .arg("-C")
        .arg(folder)
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.quotepath=false",
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            ".",
        ])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .stdin(Stdio::Null)
        .stdout(Stdio::Piped)
        .stderr(Stdio::Null)
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 64 * 1024];
        loop {
            match out.read(&mut chunk) {
                Ok(0) | Err(_) => return Some(bytes),
                Ok(read) => {
                    if bytes.len() + read > GIT_OUTPUT {
                        return None;
                    }
                    bytes.extend_from_slice(&chunk[..read]);
                }
            }
        }
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < GIT_TIME && !reader.is_finished() => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Err(_) => break None,
        }
    };
    let bytes = reader.join().ok()??;
    if !status.is_some_and(|status| status.success()) {
        return None;
    }
    let mut files: Vec<String> = Vec::new();
    for path in String::from_utf8_lossy(&bytes).split('\u{0}') {
        if path.is_empty()
            || !safe_name(path)
            || path.starts_with('/')
            || path.contains('\\')
            || path.split('/').any(|part| part == ".." || part == ".")
        {
            continue;
        }
        // A tracked file that was deleted, a link and a submodule are not offered.
        let local = path.split('/').fold(folder.to_path_buf(), |mut at, part| {
            at.push(part);
            at
        });
        if fs::symlink_metadata(&local).is_ok_and(|meta| meta.is_file()) {
            files.push(path.to_owned());
        }
    }
    let mut entries: std::collections::BTreeSet<String> = files.iter().cloned().collect();
    for path in &files {
        let mut at = 0;
        while let Some(slash) = path[at..].find('/') {
            at += slash + 1;
            entries.insert(path[..at].to_owned());
        }
    }
    let depth = |entry: &str| entry.trim_end_matches('/').matches('/').count();
    let mut ordered: Vec<String> = entries.into_iter().collect();
    ordered.sort_by(|a, b| depth(a).cmp(&depth(b)).then_with(|| a.cmp(b)));
    ordered.truncate(max_entries);
    Some(ordered)
}

/// A session id as the app makes them: letters, digits and hyphens, no longer than a UUID.
fn session_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 36
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// Why `template` cannot be a workspace, in a person's words, or none when it can: every `${name}` is a known variable and
/// `${session}` is in it, so two sessions never share a folder by accident.
pub(crate) fn workspace_problem(template: &str) -> Option<String> {
    if template.trim().is_empty() {
        return Some("it is empty".to_owned());
    }
    let mut rest = template;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            return Some("a ${ is never closed".to_owned());
        };
        let name = &after[..end];
        if !VARIABLES.contains(&name) {
            let all: Vec<String> = VARIABLES
                .iter()
                .map(|each| format!("${{{each}}}"))
                .collect();
            return Some(format!(
                "unknown variable ${{{name}}} (use {})",
                all.join(", ")
            ));
        }
        rest = &after[end + 1..];
    }
    if !template.contains("${session}") {
        return Some("it has no ${session}, so two sessions could share a workspace".to_owned());
    }
    None
}

/// What a template's variables stand for.
struct Values {
    documents: String,
    home: String,
    session: String,
    /// `(date, time)` as `2026-11-17` and `103005`.
    when: (String, String),
    handler: String,
    source: String,
    host: String,
}

impl Values {
    fn get(&self, name: &str) -> Option<String> {
        Some(match name {
            "documents" => self.documents.clone(),
            "home" => self.home.clone(),
            "session" => self.session.clone(),
            "date" => self.when.0.clone(),
            "time" => self.when.1.clone(),
            "datetime" => format!("{}T{}", self.when.0, self.when.1),
            "handler" => self.handler.clone(),
            "source" => self.source.clone(),
            "host" => self.host.clone(),
            "fqdn" => String::new(),
            _ => return None,
        })
    }
}

/// `now` (`2026-11-17T10:30:05`, the person's local time as the app reads it) as a date and a time, else this clock's UTC.
fn when_of(now: Option<&str>) -> Result<(String, String), (i32, String)> {
    if let Some(text) = now {
        let digits = |range: core::ops::Range<usize>| {
            text.get(range)
                .filter(|part| part.bytes().all(|byte| byte.is_ascii_digit()))
        };
        if text.len() >= 19
            && let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
                digits(0..4),
                digits(5..7),
                digits(8..10),
                digits(11..13),
                digits(14..16),
                digits(17..19),
            )
            && text.as_bytes().get(4) == Some(&b'-')
            && text.as_bytes().get(7) == Some(&b'-')
            && matches!(text.as_bytes().get(10), Some(b'T' | b' '))
        {
            return Ok((
                format!("{year}-{month}-{day}"),
                format!("{hour}{minute}{second}"),
            ));
        }
        return Err(bad("now is a local time as 2026-11-17T10:30:05"));
    }
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    let (year, month, day) = civil(i64::try_from(days).unwrap_or(0));
    Ok((
        format!("{year:04}-{month:02}-{day:02}"),
        format!("{:02}{:02}{:02}", rest / 3600, rest % 3600 / 60, rest % 60),
    ))
}

/// The date `days` after 1970-01-01.
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 {
        shifted + 3
    } else {
        shifted - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `value` as one path segment: separators, `..`, control and reserved characters become `_`, and a trailing dot or space
/// (which Windows drops) is trimmed. Empty stays empty.
fn safe_segment(value: &str) -> String {
    let replaced: String = value
        .chars()
        .map(|c| {
            if matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect::<String>()
        .replace("..", "_");
    let trimmed = |text: &str| text.trim_end_matches(['.', ' ']).to_owned();
    let mut text = trimmed(replaced.trim());
    if text.chars().count() > SEGMENT {
        text = trimmed(&text.chars().take(SEGMENT).collect::<String>());
    }
    if text.is_empty() {
        return text;
    }
    if let Some(rest) = text.strip_prefix('.') {
        text = format!("_{rest}");
    }
    let lower = text.to_ascii_lowercase();
    let stem = lower.split('.').next().unwrap_or_default();
    let reserved = matches!(stem, "con" | "prn" | "aux" | "nul")
        || ["com", "lpt"].iter().any(|prefix| {
            stem.strip_prefix(prefix)
                .is_some_and(|digit| matches!(digit.as_bytes(), [b'1'..=b'9']))
        });
    if reserved { format!("_{text}") } else { text }
}

/// `template` with its variables put in, separated by `separator`. Only the template places separators: a value other than a
/// folder is one segment, and a part of the template left empty by its variables is dropped. `template` must pass
/// [`workspace_problem`].
fn expand_workspace(template: &str, values: &Values, separator: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for (index, part) in template.split(['\\', '/']).enumerate() {
        let mut had_variable = false;
        let mut text = String::new();
        let mut rest = part;
        while let Some(start) = rest.find("${") {
            text.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let Some(end) = after.find('}') else {
                break;
            };
            had_variable = true;
            let name = &after[..end];
            let value = values.get(name).unwrap_or_default();
            text.push_str(&if FOLDER_VARIABLES.contains(&name) {
                value
            } else {
                safe_segment(&value)
            });
            rest = &after[end + 1..];
        }
        text.push_str(rest);
        // A leading empty part is the root of an absolute template.
        if text.is_empty() && (had_variable || index > 0) {
            continue;
        }
        out.push(text);
    }
    let joined = out.join(separator);
    // A folder value may end in a separator (a drive's root); a network path keeps its two leading ones.
    let lead = joined
        .chars()
        .take(2)
        .take_while(|c| separator.starts_with(*c))
        .count()
        * separator.len();
    let mut collapsed = joined[..lead].to_owned();
    let mut last_was_separator = false;
    for c in joined[lead..].chars() {
        let is_separator = separator.starts_with(c);
        if !(is_separator && last_was_separator) {
            collapsed.push(c);
        }
        last_was_separator = is_separator;
    }
    collapsed
}
