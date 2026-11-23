//! Finishing the work in a folder, for the person: commit what they chose, push the branch, open a pull request. Git and `gh`
//! are the person's own, run in the folder with their own settings and, for them, the repository's own hooks; nothing here
//! refuses what git does not. A failure carries git's or `gh`'s own words. No prompt can appear: neither program has an input.

use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Value, json};

use super::{NotRun, Refusal, Worktrees, failed, refused, run_command, run_git_with};
use crate::spawn::Command;

/// What stays as written in a branch name put in a web address's path.
const IN_PATH: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b'/');
/// What stays as written in a query value.
const IN_QUERY: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

impl Worktrees {
    /// The repository folder `path` names, after the host's checks of it.
    fn folder(&self, path: &str) -> Result<PathBuf, Refusal> {
        let path = self
            .checked(path)?
            .ok_or_else(|| refused("that directory does not exist"))?;
        self.safe_repository(&path)?;
        Ok(path)
    }

    /// `git args` in `dir` with the repository's own hooks, as the person's git runs them.
    fn git_hooked(&self, dir: &Path, args: &[&OsStr]) -> Result<String, Refusal> {
        let ceilings: Vec<PathBuf> = self
            .resolved_roots()
            .into_iter()
            .filter_map(|(_, root)| root.parent().map(Path::to_path_buf))
            .collect();
        let ceilings = std::env::join_paths(ceilings).unwrap_or_default();
        match run_git_with(dir, args, &ceilings, self.git_limit(), self.person)? {
            (true, out) => Ok(out),
            (false, said) => Err(failed(&format!("git failed: {said}"))),
        }
    }

    /// The branch checked out in `dir`; a detached HEAD has none to push or propose.
    fn current_branch(&self, dir: &Path) -> Result<String, Refusal> {
        self.git(
            dir,
            &[
                "symbolic-ref".as_ref(),
                "--quiet".as_ref(),
                "--short".as_ref(),
                "HEAD".as_ref(),
            ],
        )
        .map(|branch| branch.trim_end_matches(['\n', '\r']).to_owned())
        .map_err(|_| refused("HEAD is detached: there is no branch to push or propose"))
    }

    /// `{"path", "message", "paths"?}` to `{"commit", "summary", "branch"}`: stages `paths` (relative to the folder, taken
    /// literally; every change under the folder when none) and commits them with `message`. With `paths`, only those files go
    /// in the commit, whatever else is staged.
    pub(super) fn commit(
        &self,
        path: &str,
        message: &str,
        paths: Option<&Value>,
    ) -> Result<Value, Refusal> {
        let dir = self.folder(path)?;
        if message.contains('\0') {
            return Err(refused("a message holds no NUL character"));
        }
        let paths = match paths {
            None | Some(Value::Null) => None,
            Some(Value::Array(listed)) => Some(
                listed
                    .iter()
                    .map(|file| match file.as_str() {
                        Some(file) if !file.is_empty() && !file.contains('\0') => {
                            Ok(file.to_owned())
                        }
                        _ => Err(refused("paths are non-empty text")),
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            Some(_) => return Err(refused("paths must be a list")),
        };
        if paths.as_ref().is_some_and(Vec::is_empty) {
            return Err(refused("no file is chosen to commit"));
        }
        let chosen: Vec<OsString> = paths.clone().map_or_else(
            || vec![OsString::from(".")],
            |paths| paths.into_iter().map(OsString::from).collect(),
        );
        let mut stage: Vec<&OsStr> = vec![
            "--literal-pathspecs".as_ref(),
            "add".as_ref(),
            "-A".as_ref(),
            "--".as_ref(),
        ];
        stage.extend(chosen.iter().map(OsString::as_os_str));
        self.git(&dir, &stage)?;
        let mut commit: Vec<&OsStr> = vec![
            "--literal-pathspecs".as_ref(),
            "commit".as_ref(),
            "-m".as_ref(),
            OsStr::new(message),
        ];
        if paths.is_some() {
            commit.push("--only".as_ref());
            commit.push("--".as_ref());
            commit.extend(chosen.iter().map(OsString::as_os_str));
        }
        self.git_hooked(&dir, &commit)?;
        let made = self.git(
            &dir,
            &["log".as_ref(), "-1".as_ref(), "--format=%H%n%s".as_ref()],
        )?;
        let mut lines = made.lines();
        let commit = lines.next().unwrap_or_default();
        let summary = lines.next().unwrap_or_default();
        Ok(json!({
            "commit": commit,
            "summary": summary,
            "branch": self.current_branch(&dir).ok(),
        }))
    }

    /// `{"path", "remote"?, "setUpstream"?}` to `{"pushed": true, "remote", "branch"}`: `git push`, which takes the branch's
    /// upstream as git does and says so when there is none. With `setUpstream` the branch goes to `remote` (`origin` when none
    /// is named) and is made to track it.
    pub(super) fn push(
        &self,
        path: &str,
        remote: Option<&str>,
        set_upstream: bool,
    ) -> Result<Value, Refusal> {
        let dir = self.folder(path)?;
        if remote.is_some_and(|remote| {
            remote.is_empty() || remote.starts_with('-') || remote.contains('\0')
        }) {
            return Err(refused("a remote is a name or an address, not an option"));
        }
        let branch = self.current_branch(&dir)?;
        let mut args: Vec<&OsStr> = vec!["push".as_ref()];
        let named = if set_upstream {
            let remote = remote.unwrap_or("origin");
            args.push("--set-upstream".as_ref());
            args.push(OsStr::new(remote));
            args.push(OsStr::new(&branch));
            Some(remote)
        } else {
            if let Some(remote) = remote {
                args.push(OsStr::new(remote));
            }
            remote
        };
        self.git_hooked(&dir, &args)?;
        Ok(json!({"pushed": true, "remote": named, "branch": branch}))
    }

    /// `{"path", "title", "body"?, "base"?, "draft"?}` to `{"via", "url", "branch"}`: `gh pr create` in the folder when `gh` is
    /// installed (`via` is `gh`, `url` the pull request), else the forge's web page that opens one for the branch (`via` is
    /// `web`, the person's browser opens `url`). `gh`'s own refusal, signed out or no pushed branch, is the answer when it is
    /// there.
    pub(super) fn pull_request(
        &self,
        path: &str,
        title: &str,
        body: &str,
        base: Option<&str>,
        draft: bool,
    ) -> Result<Value, Refusal> {
        let dir = self.folder(path)?;
        if base.is_some_and(|base| base.is_empty() || base.starts_with('-') || base.contains('\0'))
            || title.contains('\0')
            || body.contains('\0')
        {
            return Err(refused(
                "a title, body and base hold no NUL, and a base is not an option",
            ));
        }
        let branch = self.current_branch(&dir)?;
        let searched = self
            .tools_path
            .clone()
            .or_else(|| std::env::var_os("PATH"))
            .unwrap_or_default();
        let Some(gh) = find_on_path(&searched) else {
            return self.web_pull_request(&dir, &branch, base);
        };
        let mut command = Command::new(gh);
        command
            .current_dir(&dir)
            .args([
                "pr", "create", "--title", title, "--body", body, "--head", &branch,
            ])
            .env("GH_PROMPT_DISABLED", "1")
            .env("NO_COLOR", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1");
        if let Some(base) = base {
            command.args(["--base", base]);
        }
        if draft {
            command.arg("--draft");
        }
        if let Some(path) = &self.tools_path {
            command.env("PATH", path);
        }
        match run_command(&mut command, self.git_limit()) {
            Ok((true, said)) => {
                let url = said
                    .lines()
                    .rev()
                    .map(str::trim)
                    .find(|line| line.starts_with("http"))
                    .unwrap_or_default();
                Ok(json!({"via": "gh", "url": url, "branch": branch}))
            }
            Ok((false, said)) => Err(failed(&format!("gh failed: {said}"))),
            Err(NotRun::Ended) => Err(failed("gh ran too long and was ended")),
            Err(NotRun::Started) => Err(failed("gh could not be started")),
        }
    }

    /// The forge's page that starts a pull request for `branch`, from the address of the remote the branch tracks (`origin`
    /// when it tracks none).
    fn web_pull_request(
        &self,
        dir: &Path,
        branch: &str,
        base: Option<&str>,
    ) -> Result<Value, Refusal> {
        let key = format!("branch.{branch}.remote");
        let remote = self
            .git(
                dir,
                &["config".as_ref(), "--get".as_ref(), OsStr::new(&key)],
            )
            .map(|remote| remote.trim().to_owned())
            .ok()
            .filter(|remote| !remote.is_empty() && remote != ".")
            .unwrap_or_else(|| "origin".to_owned());
        let address = self
            .git(
                dir,
                &[
                    "remote".as_ref(),
                    "get-url".as_ref(),
                    "--".as_ref(),
                    OsStr::new(&remote),
                ],
            )
            .map(|address| address.trim().to_owned())?;
        let url = web_address(&address, branch, base).ok_or_else(|| {
            refused(&format!(
                "gh is not installed, and the remote {remote} ({address}) has no web address to open a pull request on"
            ))
        })?;
        Ok(json!({"via": "web", "url": url, "branch": branch}))
    }
}

/// The `gh` program in the first folder of `path` that holds one. Looked for here, and nowhere else, so that "no `gh`" means
/// none on the `PATH` the person's programs run with, whatever the system would search besides.
fn find_on_path(path: &OsStr) -> Option<PathBuf> {
    let name = if cfg!(windows) { "gh.exe" } else { "gh" };
    std::env::split_paths(path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// The page that starts a pull request for `branch` on the forge a remote's `address` names: GitHub's `compare` page, which
/// Gitea and Forgejo serve alike, GitLab's new merge request or Bitbucket's new pull request by the host's name. `None` for a
/// remote that is not on a host, a local path.
pub(super) fn web_address(address: &str, branch: &str, base: Option<&str>) -> Option<String> {
    let (scheme, host, path) = split_remote(address)?;
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if path.split('/').filter(|part| !part.is_empty()).count() < 2 {
        return None;
    }
    let site = format!("{scheme}://{host}/{path}");
    let lower = host.to_ascii_lowercase();
    let in_path = utf8_percent_encode(branch, IN_PATH);
    let in_query = utf8_percent_encode(branch, IN_QUERY);
    Some(if lower.contains("gitlab") {
        let target = base.map_or_else(String::new, |base| {
            format!(
                "&merge_request%5Btarget_branch%5D={}",
                utf8_percent_encode(base, IN_QUERY)
            )
        });
        format!("{site}/-/merge_requests/new?merge_request%5Bsource_branch%5D={in_query}{target}")
    } else if lower.contains("bitbucket") {
        let target = base.map_or_else(String::new, |base| {
            format!("&dest={}", utf8_percent_encode(base, IN_QUERY))
        });
        format!("{site}/pull-requests/new?source={in_query}{target}")
    } else {
        match base {
            Some(base) => format!(
                "{site}/compare/{}...{in_path}?expand=1",
                utf8_percent_encode(base, IN_PATH)
            ),
            None => format!("{site}/compare/{in_path}?expand=1"),
        }
    })
}

/// A remote's address as web scheme, host and path: `https://` and `http://` as written (the port kept), `ssh://`, `git://`
/// and `user@host:path` as `https` with no port.
fn split_remote(address: &str) -> Option<(&'static str, String, String)> {
    let address = address.trim();
    if let Some((scheme, rest)) = address.split_once("://") {
        let web = match scheme.to_ascii_lowercase().as_str() {
            "https" => "https",
            "http" => "http",
            "ssh" | "git" | "git+ssh" | "ssh+git" => "ssh",
            _ => return None,
        };
        let (authority, path) = rest.split_once('/')?;
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        if host.is_empty() {
            return None;
        }
        return Some(if web == "ssh" {
            let bare = host.rsplit_once(':').map_or(host, |(name, _)| name);
            ("https", bare.to_owned(), path.to_owned())
        } else {
            (web, host.to_owned(), path.to_owned())
        });
    }
    let (authority, path) = address.split_once(':')?;
    if authority.is_empty()
        || authority.contains('/')
        || authority.contains('\\')
        || path.is_empty()
    {
        return None;
    }
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    // A drive letter is a path, not a host.
    if host.len() == 1 {
        return None;
    }
    Some(("https", host.to_owned(), path.to_owned()))
}
