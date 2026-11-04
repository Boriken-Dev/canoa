//! A git worktree per session, made by the host inside an entry's working roots. A client names the directory a session
//! would work in and a branch; the host finds the repository there, puts the worktree beside it in `<name>.worktrees/`,
//! and answers where it is. Every path is resolved and checked against the roots resolved too, so a link never leads a
//! worktree, or a removal, out of them. Git runs from an argument list, never a shell, with hooks and fsmonitor off.

use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use std::ffi::OsStr;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf, Prefix};
use std::process::{Command, Stdio};

use serde_json::{Map, Value, json};

use crate::config::plain_absolute;

/// Makes a worktree: `{"cwd", "branch"}` to `{"path", "branch", "repo"}`.
pub(crate) const NEW: &str = "_canoa/worktree/new";
/// What a worktree holds: `{"path"}` to `{"exists": false}` or `{"exists": true, "branch", "changed", "commits"}`.
pub(crate) const STATUS: &str = "_canoa/worktree/status";
/// Removes one: `{"path", "discard"}` to `{"removed": true, "branch", "branchKept", "commits"}`.
pub(crate) const REMOVE: &str = "_canoa/worktree/remove";

/// Every branch the host makes or removes starts with this.
pub(crate) const BRANCH_PREFIX: &str = "canoa/";
/// The longest branch name taken, prefix included.
pub(crate) const MAX_BRANCH: usize = 70;
/// The most of git's own words an error carries.
const MAX_GIT_TEXT: usize = 1024;

const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;

/// `canoa/` and then lower-case letters and digits in runs joined by single hyphens: nothing git or a shell reads as an
/// option, a range or a path step, and no control character.
pub(crate) fn safe_branch(branch: &str) -> bool {
    let Some(slug) = branch.strip_prefix(BRANCH_PREFIX) else {
        return false;
    };
    branch.len() <= MAX_BRANCH
        && !slug.is_empty()
        && slug.split('-').all(|run| {
            !run.is_empty()
                && run
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

/// The worktree rule for one entry's clients.
#[derive(Clone, Debug, Default)]
pub(crate) struct Worktrees {
    roots: Vec<PathBuf>,
    on: bool,
}

/// Why a request was not done, as a JSON-RPC error.
#[derive(Debug, PartialEq, Eq)]
struct Refusal {
    code: i64,
    message: String,
}

fn refused(message: &str) -> Refusal {
    Refusal {
        code: INVALID_PARAMS,
        message: message.to_owned(),
    }
}

impl Worktrees {
    pub(crate) const fn new(roots: Vec<PathBuf>, on: bool) -> Self {
        Self { roots, on }
    }

    /// Whether `message` is a worktree request, which never reaches the agent.
    pub(crate) fn is_request(message: &[u8]) -> bool {
        serde_json::from_slice::<Value>(message).is_ok_and(|value| {
            matches!(
                value.get("method").and_then(Value::as_str),
                Some(NEW | STATUS | REMOVE)
            )
        })
    }

    /// The answer to a worktree request, or `None` for one sent as a notification, which is dropped.
    pub(crate) fn answer(&self, message: &[u8]) -> Option<Vec<u8>> {
        let value = serde_json::from_slice::<Value>(message).ok()?;
        let id = value.get("id")?.clone();
        let method = value.get("method").and_then(Value::as_str).unwrap_or("");
        let outcome = match value.get("params") {
            Some(Value::Object(params)) => self.run(method, params),
            _ => Err(refused("params must be an object")),
        };
        let reply = match outcome {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(refusal) => {
                tracing::info!("a worktree request was refused: {}", refusal.message);
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": refusal.code, "message": refusal.message}})
            }
        };
        serde_json::to_vec(&reply).ok()
    }

    fn run(&self, method: &str, params: &Map<String, Value>) -> Result<Value, Refusal> {
        if !self.on {
            return Err(refused(
                "this host makes no worktrees for this agent: its entry sets worktrees = false",
            ));
        }
        if self.roots.is_empty() {
            return Err(refused(
                "this host makes worktrees only inside working roots, and this agent's entry names none",
            ));
        }
        let text = |key: &str| {
            params
                .get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| refused(&format!("{key} is missing or not text")))
        };
        match method {
            NEW => self.make(text("cwd")?, text("branch")?),
            STATUS => self.status(text("path")?),
            REMOVE => {
                let discard = match params.get("discard") {
                    None => false,
                    Some(Value::Bool(discard)) => *discard,
                    Some(_) => return Err(refused("discard must be true or false")),
                };
                self.remove(text("path")?, discard)
            }
            _ => Err(refused("not a worktree request")),
        }
    }

    /// Each root as written, with its target once links are resolved; a root that does not exist now is left out.
    fn resolved_roots(&self) -> Vec<(&Path, PathBuf)> {
        self.roots
            .iter()
            .filter_map(|root| Some((root.as_path(), resolve(root).ok()?)))
            .collect()
    }

    /// `path` as a client wrote it, checked as the scope checks a `cwd` and then resolved: its target must still be under a
    /// root's target. `Ok(None)` when nothing is there.
    fn checked(&self, text: &str) -> Result<Option<PathBuf>, Refusal> {
        let path = Path::new(text);
        // Looking at a share would reach that machine with this user's credentials, before any check.
        if !plain_absolute(path)
            || !local(path)
            || !self.roots.iter().any(|root| path.starts_with(root))
        {
            return Err(refused(
                "this host makes worktrees only under its working roots",
            ));
        }
        let resolved = match resolve(path) {
            Ok(resolved) => resolved,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(refused("that directory cannot be read")),
        };
        if !self
            .resolved_roots()
            .iter()
            .any(|(_, root)| resolved.starts_with(root))
        {
            return Err(refused(
                "a link leads that directory out of the working roots",
            ));
        }
        Ok(Some(resolved))
    }

    /// `resolved` written under the root it is in, as the client's `cwd` will be checked against.
    fn written(&self, resolved: &Path) -> Option<PathBuf> {
        self.resolved_roots()
            .into_iter()
            .find_map(|(written, root)| {
                resolved.strip_prefix(&root).ok().map(|rest| {
                    if rest.as_os_str().is_empty() {
                        written.to_path_buf()
                    } else {
                        written.join(rest)
                    }
                })
            })
    }

    fn make(&self, cwd: &str, branch: &str) -> Result<Value, Refusal> {
        if !safe_branch(branch) {
            return Err(refused(
                "a branch is canoa/ then lower-case letters and digits joined by single hyphens, 70 characters at most",
            ));
        }
        let cwd = self
            .checked(cwd)?
            .ok_or_else(|| refused("that directory does not exist"))?;
        let repo = self.main_worktree(&cwd)?;
        let location = self.location(&repo, branch)?;
        match std::fs::symlink_metadata(&location) {
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            _ => return Err(refused("a worktree for that branch is already there")),
        }
        let mut args = vec![
            "worktree".as_ref(),
            "add".as_ref(),
            "-b".as_ref(),
            OsStr::new(branch),
            "--".as_ref(),
            location.as_os_str(),
            "HEAD".as_ref(),
        ];
        git(&cwd, &args)?;
        // Made through a link that appeared meanwhile, it is taken back.
        if resolve(&location).ok().as_deref() != Some(location.as_path()) {
            args = vec![
                "worktree".as_ref(),
                "remove".as_ref(),
                "--force".as_ref(),
                "--".as_ref(),
                location.as_os_str(),
            ];
            let _ = git(&repo, &args);
            return Err(refused(
                "a link leads that worktree out of the working roots",
            ));
        }
        let shown = |path: &Path| {
            self.written(path)
                .map(|path| path.to_string_lossy().into_owned())
        };
        Ok(json!({"path": shown(&location), "branch": branch, "repo": shown(&repo)}))
    }

    /// The repository's main worktree, resolved, which must be under a root too.
    fn main_worktree(&self, dir: &Path) -> Result<PathBuf, Refusal> {
        let listed = git(
            dir,
            &[
                "worktree".as_ref(),
                "list".as_ref(),
                "--porcelain".as_ref(),
                "-z".as_ref(),
            ],
        )?;
        let first = listed
            .split('\0')
            .next()
            .and_then(|line| line.strip_prefix("worktree "))
            .ok_or_else(|| refused("that directory is not in a git repository"))?;
        if listed
            .split('\0')
            .take_while(|line| !line.is_empty())
            .any(|line| line == "bare")
        {
            return Err(refused("a bare repository has no worktree to start from"));
        }
        let repo = resolve(Path::new(first))
            .map_err(|_| refused("the repository's directory cannot be read"))?;
        if !self
            .resolved_roots()
            .iter()
            .any(|(_, root)| repo.starts_with(root))
        {
            return Err(refused("that repository is outside the working roots"));
        }
        Ok(repo)
    }

    /// `<repository>.worktrees/<branch without canoa/>`, beside the repository, inside a root. A `.worktrees` directory
    /// already there must be a plain directory.
    fn location(&self, repo: &Path, branch: &str) -> Result<PathBuf, Refusal> {
        let (Some(parent), Some(name)) = (repo.parent(), repo.file_name()) else {
            return Err(refused(
                "a repository at the top of a drive has no directory beside it",
            ));
        };
        if !self
            .resolved_roots()
            .iter()
            .any(|(_, root)| parent.starts_with(root))
        {
            return Err(refused(
                "the worktree would be beside the repository, outside the working roots: name the directory holding the \
                 repository as a working root",
            ));
        }
        let mut holder_name = name.to_owned();
        holder_name.push(".worktrees");
        let holder = parent.join(holder_name);
        match std::fs::symlink_metadata(&holder) {
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Ok(meta)
                if meta.is_dir()
                    && !meta.file_type().is_symlink()
                    && resolve(&holder).ok().as_ref() == Some(&holder) => {}
            _ => {
                return Err(refused(
                    "the repository's .worktrees directory is a link or not a directory",
                ));
            }
        }
        let slug = branch.strip_prefix(BRANCH_PREFIX).unwrap_or(branch);
        Ok(holder.join(slug))
    }

    /// A worktree this host would have made: a linked worktree, on a branch it takes, where [`Worktrees::location`]
    /// puts that branch. Answers its repository and branch.
    fn ours(&self, path: &Path) -> Result<(PathBuf, String), Refusal> {
        let not_ours = || refused("that directory is not a worktree Canoa made");
        let top = git(path, &["rev-parse".as_ref(), "--show-toplevel".as_ref()])
            .map_err(|_| not_ours())?;
        if resolve(Path::new(top.trim_end_matches(['\n', '\r'])))
            .ok()
            .as_deref()
            != Some(path)
        {
            return Err(not_ours());
        }
        let repo = self.main_worktree(path)?;
        let branch = git(
            path,
            &[
                "symbolic-ref".as_ref(),
                "--quiet".as_ref(),
                "--short".as_ref(),
                "HEAD".as_ref(),
            ],
        )
        .map_err(|_| not_ours())?;
        let branch = branch.trim_end_matches(['\n', '\r']).to_owned();
        if repo == path || !safe_branch(&branch) || self.location(&repo, &branch)? != path {
            return Err(not_ours());
        }
        Ok((repo, branch))
    }

    fn status(&self, path: &str) -> Result<Value, Refusal> {
        let Some(path) = self.checked(path)? else {
            return Ok(json!({"exists": false}));
        };
        let (repo, branch) = self.ours(&path)?;
        Ok(json!({
            "exists": true,
            "branch": branch,
            "changed": changed(&path)?,
            "commits": commits_only_on(&repo, &branch)?,
        }))
    }

    fn remove(&self, path: &str, discard: bool) -> Result<Value, Refusal> {
        let path = self
            .checked(path)?
            .ok_or_else(|| refused("that worktree is gone"))?;
        let (repo, branch) = self.ours(&path)?;
        let commits = commits_only_on(&repo, &branch)?;
        // Without discard git refuses a worktree holding changes or untracked files, however they came after a status.
        let mut args = vec!["worktree".as_ref(), "remove".as_ref()];
        if discard {
            args.push("--force".as_ref());
        }
        args.extend(["--".as_ref(), path.as_os_str()]);
        git(&repo, &args)?;
        // A branch holding commits no other branch, tag or remote has is kept: removing it would lose them.
        if commits == 0 {
            git(
                &repo,
                &[
                    "branch".as_ref(),
                    "-D".as_ref(),
                    "--".as_ref(),
                    OsStr::new(&branch),
                ],
            )?;
        }
        Ok(
            json!({"removed": true, "branch": branch, "branchKept": commits > 0, "commits": commits}),
        )
    }
}

/// How many files `git status` lists, untracked ones included.
fn changed(path: &Path) -> Result<u64, Refusal> {
    let listed = git(
        path,
        &[
            "status".as_ref(),
            "--porcelain=v1".as_ref(),
            "-z".as_ref(),
            "--untracked-files=all".as_ref(),
        ],
    )?;
    let mut fields = listed.split('\0').filter(|field| !field.is_empty());
    let mut count = 0;
    while let Some(field) = fields.next() {
        count += 1;
        // A rename or copy is followed by the name it had.
        if field.starts_with(['R', 'C']) {
            fields.next();
        }
    }
    Ok(count)
}

/// Commits on `branch` that no other branch, tag or remote reaches.
fn commits_only_on(repo: &Path, branch: &str) -> Result<u64, Refusal> {
    let reference = format!("refs/heads/{branch}");
    let exclude = format!("--exclude={branch}");
    let counted = git(
        repo,
        &[
            "rev-list".as_ref(),
            "--count".as_ref(),
            reference.as_ref(),
            "--not".as_ref(),
            exclude.as_ref(),
            "--branches".as_ref(),
            "--remotes".as_ref(),
            "--tags".as_ref(),
            "--".as_ref(),
        ],
    )?;
    counted.trim().parse().map_err(|_| Refusal {
        code: INTERNAL_ERROR,
        message: "git did not count the branch's commits".to_owned(),
    })
}

/// Runs `git -C dir args`, its output as text. Hooks and fsmonitor are off and no `GIT_` variable of the host's reaches
/// it, so a repository's settings cannot point it at another directory or run a program.
fn git(dir: &Path, args: &[&OsStr]) -> Result<String, Refusal> {
    let no_hooks = if cfg!(windows) {
        "core.hooksPath=NUL"
    } else {
        "core.hooksPath=/dev/null"
    };
    let mut command = Command::new("git");
    for (name, _) in std::env::vars_os() {
        if name
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_")
        {
            command.env_remove(name);
        }
    }
    let output = command
        .arg("-C")
        .arg(dir)
        .args(["-c", no_hooks, "-c", "core.fsmonitor=false"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|_| Refusal {
            code: INTERNAL_ERROR,
            message: "git could not be started".to_owned(),
        })?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let said: String = String::from_utf8_lossy(&output.stderr)
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_GIT_TEXT)
        .collect();
    Err(Refusal {
        code: INTERNAL_ERROR,
        message: format!("git failed: {}", said.trim()),
    })
}

/// `path` with every link resolved. On Windows the result is written as a drive path, not `\\?\`, as git and the roots
/// are; a share is refused.
fn resolve(path: &Path) -> std::io::Result<PathBuf> {
    let resolved = std::fs::canonicalize(path)?;
    if !local(&resolved) {
        return Err(std::io::Error::new(
            ErrorKind::InvalidInput,
            "not a local directory",
        ));
    }
    Ok(
        match resolved
            .to_str()
            .and_then(|text| text.strip_prefix(r"\\?\"))
        {
            Some(drive) if cfg!(windows) => PathBuf::from(drive),
            _ => resolved,
        },
    )
}

/// Whether `path` names this machine's file system: on Windows a drive, not a share or a device namespace.
fn local(path: &Path) -> bool {
    match path.components().next() {
        Some(Component::Prefix(prefix)) => {
            matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests;
