//! A git worktree per session, made by the host inside an entry's working roots. A client names the directory a session
//! would work in and a branch; the host finds the repository there, puts the worktree beside it in `<name>.worktrees/`,
//! and answers where it is. Every path is resolved and checked against the roots resolved too, so a link never leads a
//! worktree, or a removal, out of them. Git runs from an argument list, never a shell, with hooks and fsmonitor off,
//! and a repository whose own configuration names a filter or an include is refused before git reads its files.

use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;
use std::ffi::{OsStr, OsString};
use std::io::{ErrorKind, Read as _};
use std::path::{Component, Path, PathBuf, Prefix};
use std::process::{Command, Stdio};
use std::time::Instant;

use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

use crate::config::plain_absolute;

/// Makes a worktree: `{"cwd", "branch"}` to `{"path", "branch", "repo"}`.
pub(crate) const NEW: &str = "_canoa/worktree/new";
/// What a worktree holds: `{"path"}` to `{"exists": false}` or
/// `{"exists": true, "branch", "detached", "changed", "commits", "listing"}`.
pub(crate) const STATUS: &str = "_canoa/worktree/status";
/// Removes one: `{"path", "discard"}` to `{"removed": true, "branch", "branchKept", "commits"}`.
pub(crate) const REMOVE: &str = "_canoa/worktree/remove";

/// Every branch the host makes or removes starts with this.
pub(crate) const BRANCH_PREFIX: &str = "canoa/";
/// The longest branch name taken, prefix included.
pub(crate) const MAX_BRANCH: usize = 70;
/// Worktrees one repository may hold in its `.worktrees` directory before the host makes no more.
pub(crate) const MAX_WORKTREES: usize = 32;
/// How long one git command may run before it is ended.
const GIT_LIMIT: Duration = Duration::from_secs(120);
/// The most of git's own words an error carries.
const MAX_GIT_TEXT: usize = 1024;
/// Repository settings that make git run a program (a filter) or read another file of settings (an include), and where
/// attributes naming a filter come from.
const UNSAFE_SETTINGS: &str = r"^(filter|include|includeif)\.|^core\.attributesfile$";

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

fn failed(message: &str) -> Refusal {
    Refusal {
        code: INTERNAL_ERROR,
        message: message.to_owned(),
    }
}

/// A worktree the host made, as it is now.
struct Found {
    /// The repository's main worktree.
    repo: PathBuf,
    /// The branch the host made it on, named by where it is.
    branch: String,
    /// The branch's commit, while it exists.
    tip: Option<String>,
    /// The branch checked out there now; `None` when its HEAD is detached.
    head: Option<String>,
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

    /// Whether `resolved` is under a root's target.
    fn inside(&self, resolved: &Path) -> bool {
        self.resolved_roots()
            .iter()
            .any(|(_, root)| resolved.starts_with(root))
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
        if !self.inside(&resolved) {
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

    /// Runs `git -C dir args`, its output as text; see [`run_git`]. Git looks for a repository no higher than the roots.
    fn git(&self, dir: &Path, args: &[&OsStr]) -> Result<String, Refusal> {
        let ceilings: Vec<PathBuf> = self
            .resolved_roots()
            .into_iter()
            .filter_map(|(_, root)| root.parent().map(Path::to_path_buf))
            .collect();
        let ceilings = std::env::join_paths(ceilings).unwrap_or_default();
        match run_git(dir, args, &ceilings)? {
            (true, out) => Ok(out),
            (false, said) => Err(failed(&format!("git failed: {said}"))),
        }
    }

    /// Refuses a repository git would be led out of the roots by, or made to run a program by, before git reads more of it
    /// than its settings: a `.git` file or `commondir` naming a share, settings naming a filter or an include, a git
    /// directory outside the roots or apart from the work tree (a submodule's, `--separate-git-dir`), or alternates.
    /// Answers the repository's main worktree.
    fn safe_repository(&self, dir: &Path) -> Result<PathBuf, Refusal> {
        for ancestor in dir.ancestors().take_while(|ancestor| self.inside(ancestor)) {
            let dot_git = ancestor.join(".git");
            if std::fs::symlink_metadata(&dot_git).is_ok_and(|meta| meta.is_file()) {
                let gitdir = pointed(&dot_git, "gitdir:", ancestor)?;
                pointed(&gitdir.join("commondir"), "", &gitdir)?;
            }
        }
        let ceilings: Vec<PathBuf> = self
            .resolved_roots()
            .into_iter()
            .filter_map(|(_, root)| root.parent().map(Path::to_path_buf))
            .collect();
        let ceilings = std::env::join_paths(ceilings).unwrap_or_default();
        for scope in ["--local", "--worktree"] {
            let args: [&OsStr; 5] = [
                "config".as_ref(),
                scope.as_ref(),
                "--name-only".as_ref(),
                "--get-regexp".as_ref(),
                UNSAFE_SETTINGS.as_ref(),
            ];
            if let (true, _) = run_git(dir, &args, &ceilings)? {
                return Err(refused(
                    "the repository's own configuration names a filter, an include or an attributes file, which would \
                     run or read what this host does not check: none is made, read or removed there",
                ));
            }
        }
        let repo = self.main_worktree(dir)?;
        let common = self.git(
            dir,
            &[
                "rev-parse".as_ref(),
                "--path-format=absolute".as_ref(),
                "--git-common-dir".as_ref(),
            ],
        )?;
        let common = resolve(Path::new(common.trim_end_matches(['\n', '\r'])))
            .map_err(|_| refused("the repository's git directory cannot be read"))?;
        if resolve(&repo.join(".git")).ok().as_ref() != Some(&common) || !self.inside(&common) {
            return Err(refused(
                "the repository's git directory is apart from its work tree, as a submodule's is: none is made there",
            ));
        }
        if common
            .join("objects")
            .join("info")
            .join("alternates")
            .exists()
        {
            return Err(refused(
                "the repository borrows objects from another (alternates): none is made there",
            ));
        }
        Ok(repo)
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
        let repo = self.safe_repository(&cwd)?;
        let location = self.location(&repo, branch)?;
        match std::fs::symlink_metadata(&location) {
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            _ => return Err(refused("a worktree for that branch is already there")),
        }
        let holder = location.parent().unwrap_or(&location);
        let listed = self.list(&repo)?;
        if listed
            .iter()
            .filter(|path| resolve(path).is_ok_and(|path| path.starts_with(holder)))
            .count()
            >= MAX_WORKTREES
        {
            return Err(refused(&format!(
                "the repository holds {MAX_WORKTREES} worktrees beside it already, the most this host makes"
            )));
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
        self.git(&cwd, &args)?;
        // Made through a link that appeared meanwhile, it is taken back.
        if resolve(&location).ok().as_deref() != Some(location.as_path()) {
            args = vec![
                "worktree".as_ref(),
                "remove".as_ref(),
                "--force".as_ref(),
                "--".as_ref(),
                location.as_os_str(),
            ];
            let _ = self.git(&repo, &args);
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

    /// Every worktree of the repository at `dir`, the main one first, as git writes them.
    fn list(&self, dir: &Path) -> Result<Vec<PathBuf>, Refusal> {
        let listed = self.git(
            dir,
            &[
                "worktree".as_ref(),
                "list".as_ref(),
                "--porcelain".as_ref(),
                "-z".as_ref(),
            ],
        )?;
        if listed
            .split('\0')
            .take_while(|line| !line.is_empty())
            .any(|line| line == "bare")
        {
            return Err(refused("a bare repository has no worktree to start from"));
        }
        Ok(listed
            .split('\0')
            .filter_map(|line| line.strip_prefix("worktree "))
            .map(PathBuf::from)
            .collect())
    }

    /// The repository's main worktree, resolved, which must be under a root too.
    fn main_worktree(&self, dir: &Path) -> Result<PathBuf, Refusal> {
        let listed = self.list(dir)?;
        let first = listed
            .first()
            .ok_or_else(|| refused("that directory is not in a git repository"))?;
        let repo =
            resolve(first).map_err(|_| refused("the repository's directory cannot be read"))?;
        if !self.inside(&repo) {
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
        if !self.inside(parent) {
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

    /// A worktree this host would have made: its own top level, a linked worktree, where [`Worktrees::location`] puts the
    /// branch its directory is named for. Found by where it is, so one whose HEAD was detached or moved to another branch is
    /// still found.
    fn ours(&self, path: &Path) -> Result<Found, Refusal> {
        let not_ours = || refused("that directory is not a worktree Canoa made");
        let repo = self.safe_repository(path)?;
        let top = self
            .git(path, &["rev-parse".as_ref(), "--show-toplevel".as_ref()])
            .map_err(|_| not_ours())?;
        if resolve(Path::new(top.trim_end_matches(['\n', '\r'])))
            .ok()
            .as_deref()
            != Some(path)
        {
            return Err(not_ours());
        }
        let branch = format!(
            "{BRANCH_PREFIX}{}",
            path.file_name().unwrap_or_default().to_string_lossy()
        );
        if repo == path || !safe_branch(&branch) || self.location(&repo, &branch)? != path {
            return Err(not_ours());
        }
        let reference = format!("refs/heads/{branch}");
        let tip = self
            .git(
                &repo,
                &[
                    "rev-parse".as_ref(),
                    "--verify".as_ref(),
                    "--quiet".as_ref(),
                    reference.as_ref(),
                ],
            )
            .ok()
            .map(|tip| tip.trim().to_owned());
        let head = self
            .git(
                path,
                &[
                    "symbolic-ref".as_ref(),
                    "--quiet".as_ref(),
                    "--short".as_ref(),
                    "HEAD".as_ref(),
                ],
            )
            .ok()
            .map(|head| head.trim_end_matches(['\n', '\r']).to_owned());
        Ok(Found {
            repo,
            branch,
            tip,
            head,
        })
    }

    /// What `git status` lists there, files git ignores and other repositories inside included: what removing the worktree
    /// deletes. Answers how many, and a digest of the whole list.
    fn changed(&self, path: &Path) -> Result<(u64, String), Refusal> {
        let listed = self.git(
            path,
            &[
                "status".as_ref(),
                "--porcelain=v1".as_ref(),
                "-z".as_ref(),
                "--untracked-files=all".as_ref(),
                "--ignored=matching".as_ref(),
                "--ignore-submodules=all".as_ref(),
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
        let digest: String = Sha256::digest(listed.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok((count, digest))
    }

    /// Commits `starts` reach that no branch but `besides` (none when `None`), no tag and no remote does.
    fn commits_only_on(
        &self,
        dir: &Path,
        besides: Option<&str>,
        starts: &[&str],
    ) -> Result<u64, Refusal> {
        let exclude = besides.map(|branch| format!("--exclude={branch}"));
        let mut args: Vec<&OsStr> = vec!["rev-list".as_ref(), "--count".as_ref()];
        args.extend(starts.iter().map(OsStr::new));
        args.push("--not".as_ref());
        if let Some(exclude) = &exclude {
            args.push(exclude.as_ref());
        }
        args.extend(["--branches", "--remotes", "--tags", "--"].map(OsStr::new));
        self.git(dir, &args)?
            .trim()
            .parse()
            .map_err(|_| failed("git did not count the branch's commits"))
    }

    fn status(&self, path: &str) -> Result<Value, Refusal> {
        let Some(path) = self.checked(path)? else {
            return Ok(json!({"exists": false}));
        };
        let found = self.ours(&path)?;
        let (changed, listing) = self.changed(&path)?;
        let reference = format!("refs/heads/{}", found.branch);
        let mut starts = vec!["HEAD"];
        if found.tip.is_some() {
            starts.push(&reference);
        }
        Ok(json!({
            "exists": true,
            "branch": found.head,
            "detached": found.head.is_none(),
            "changed": changed,
            "commits": self.commits_only_on(&path, Some(&found.branch), &starts)?,
            "listing": listing,
        }))
    }

    fn remove(&self, path: &str, discard: bool) -> Result<Value, Refusal> {
        self.remove_after(path, discard, || {})
    }

    /// [`Worktrees::remove`], running `between` once the branch's commits are counted, as another process might.
    fn remove_after(
        &self,
        path: &str,
        discard: bool,
        between: impl FnOnce(),
    ) -> Result<Value, Refusal> {
        let path = self
            .checked(path)?
            .ok_or_else(|| refused("that worktree is gone"))?;
        let found = self.ours(&path)?;
        // A commit only a detached HEAD reaches is lost with the worktree, whatever the client says.
        if found.head.is_none() && self.commits_only_on(&path, None, &["HEAD"])? > 0 {
            return Err(refused(
                "its HEAD is detached at commits no branch holds, which removing it would lose: put them on a branch first",
            ));
        }
        // Git itself removes what it ignores without --force, so the host refuses that unless told to discard.
        let (changed, _) = self.changed(&path)?;
        if changed > 0 && !discard {
            return Err(refused(&format!(
                "{changed} files or directories there are in no commit, which removing it deletes: only with discard"
            )));
        }
        let reference = format!("refs/heads/{}", found.branch);
        let commits = match &found.tip {
            Some(_) => self.commits_only_on(&found.repo, Some(&found.branch), &[&reference])?,
            None => 0,
        };
        between();
        let mut args = vec!["worktree".as_ref(), "remove".as_ref()];
        if discard {
            args.push("--force".as_ref());
        }
        args.extend(["--".as_ref(), path.as_os_str()]);
        self.git(&found.repo, &args)?;
        // A branch holding commits no other branch, tag or remote has is kept, and so is one that moved since it was
        // counted: `update-ref` deletes it only at the commit counted.
        let deleted = match (&found.tip, commits) {
            (Some(tip), 0) => self
                .git(
                    &found.repo,
                    &[
                        "update-ref".as_ref(),
                        "-d".as_ref(),
                        reference.as_ref(),
                        tip.as_ref(),
                    ],
                )
                .is_ok(),
            _ => false,
        };
        Ok(json!({
            "removed": true,
            "branch": found.branch,
            "branchKept": found.tip.is_some() && !deleted,
            "commits": commits,
        }))
    }
}

/// What the file `file` points at (`gitdir: X` for a `.git` file, the whole text for `commondir`), made absolute from
/// `base`; refused when it names a share or a device. A missing file points at `base` itself.
fn pointed(file: &Path, prefix: &str, base: &Path) -> Result<PathBuf, Refusal> {
    let mut text = String::new();
    match std::fs::File::open(file) {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(base.to_path_buf()),
        Err(_) => return Err(refused("the repository's .git file cannot be read")),
        Ok(opened) => {
            opened
                .take(4096)
                .read_to_string(&mut text)
                .map_err(|_| refused("the repository's .git file cannot be read"))?;
        }
    }
    let target = text
        .lines()
        .next()
        .unwrap_or("")
        .strip_prefix(prefix)
        .unwrap_or("")
        .trim();
    let named = Path::new(target);
    if target.starts_with("//") || target.starts_with(r"\\") || !local(named) {
        return Err(refused(
            "the repository's git directory is named on a share or device, which this host does not reach",
        ));
    }
    Ok(base.join(named))
}

/// Runs `git -C dir args`: `(true, output)`, or `(false, git's own words)` for a failure. Hooks and fsmonitor are off,
/// `ceilings` is `GIT_CEILING_DIRECTORIES`, and no `GIT_` variable of the host's reaches it, so a repository's settings
/// cannot point it at another directory or run a hook. A git that runs past [`GIT_LIMIT`] is ended.
fn run_git(dir: &Path, args: &[&OsStr], ceilings: &OsString) -> Result<(bool, String), Refusal> {
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
    let mut child = command
        .arg("-C")
        .arg(dir)
        .args(["-c", no_hooks, "-c", "core.fsmonitor=false"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CEILING_DIRECTORIES", ceilings)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| failed("git could not be started"))?;
    let readers = [
        child
            .stdout
            .take()
            .map(|out| Box::new(out) as Box<dyn std::io::Read + Send>),
        child
            .stderr
            .take()
            .map(|err| Box::new(err) as Box<dyn std::io::Read + Send>),
    ]
    .map(|stream| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut stream) = stream {
                let _ = stream.read_to_end(&mut bytes);
            }
            bytes
        })
    });
    let until = Instant::now() + GIT_LIMIT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(failed("git ran too long and was ended"));
            }
        }
    };
    let [out, err] = readers.map(|reader| reader.join().unwrap_or_default());
    if status.success() {
        return Ok((true, String::from_utf8_lossy(&out).into_owned()));
    }
    let said: String = String::from_utf8_lossy(&err)
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_GIT_TEXT)
        .collect();
    Ok((false, said.trim().to_owned()))
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
