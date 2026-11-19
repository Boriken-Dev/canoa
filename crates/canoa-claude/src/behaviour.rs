//! What Canoa does for Claude Code beyond its description: the variables an account sets, where a login lives, the shell word and
//! the sessions its program kept in its own folder.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "folders and files are std's"
)]

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use std::path::PathBuf;

use canoa::vendor::sessions::{
    files, folders, found, json_lines, listing, name_of, tidy, window_lines,
};
use canoa::vendor::{
    HistoryAnswer, HistoryRequest, LoginFile, SessionListing, SessionsRequest, ShellAdvice,
    ShellRequest, Variable,
};
use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_history::Body;
use coqui_acp_history::conversation::is_uuid;
use serde_json::Value;

use crate::acp::accounts::{CONSOLE, CREDENTIALS_FILE, FILE_KEY, LOGIN, variables};
use crate::acp::history::{DEFAULT_MAX_BYTES, export};

pub(crate) fn account_variables(grant: &coqui_acp_api::AccountGrant) -> Vec<Variable> {
    variables(grant)
        .into_iter()
        .map(|(name, value)| Variable::new(name, value))
        .collect()
}

pub(crate) fn login_file(kind: &str) -> Option<LoginFile> {
    matches!(kind, LOGIN | CONSOLE).then(|| LoginFile::new(FILE_KEY, CREDENTIALS_FILE))
}

/// Claude Code's `CLAUDE_CODE_GIT_BASH_PATH` on Windows (its documentation: "path to the Git Bash executable (`bash.exe`)", read only
/// when the file is named `bash.exe`, `sh.exe`, `bash` or `sh`) and `CLAUDE_CODE_SHELL` elsewhere ("a path to a `bash` or `zsh`
/// binary").
pub(crate) fn shell_advice(shell: &ShellRequest) -> ShellAdvice {
    let bash_like = matches!(shell.family.as_str(), "bash" | "zsh");
    let name = shell.name.as_str();
    if bash_like && shell.windows {
        if shell.wsl {
            ShellAdvice::cannot(
                "Claude Code on Windows runs Git Bash, and WSL's bash.exe starts a Linux shell: choose Git Bash or MSYS2's bash for it",
            )
        } else if name == "bash" || name == "sh" {
            ShellAdvice::sets("CLAUDE_CODE_GIT_BASH_PATH", &shell.path)
        } else {
            ShellAdvice::cannot("Claude Code on Windows takes a bash.exe or sh.exe")
        }
    } else if bash_like && (name == "bash" || name == "zsh") {
        ShellAdvice::sets("CLAUDE_CODE_SHELL", &shell.path)
    } else if shell.family == "powershell" {
        ShellAdvice::cannot(
            "Claude Code takes no path for PowerShell: its PowerShell tool runs the first pwsh on the PATH, which a copy the Store installs is on",
        )
    } else {
        ShellAdvice::cannot(
            "Claude Code runs its commands with bash (or zsh), and takes no other shell",
        )
    }
}

/// Where Claude Code looks for its sessions: `CLAUDE_CONFIG_DIR` or `.claude` in the home.
fn home_of(request: &SessionsRequest) -> Option<PathBuf> {
    request
        .home
        .as_ref()
        .map(PathBuf::from)
        .or_else(|| crate::acp::config_dir(&BTreeMap::new(), false))
}

/// The sessions under `projects/<folder>/<uuid>.jsonl`.
pub(crate) fn sessions(request: &SessionsRequest) -> SessionListing {
    let home = home_of(request);
    let limit = usize::try_from(request.limit).unwrap_or(usize::MAX);
    let mut found_files = Vec::new();
    if let Some(home) = &home {
        'walk: for project in folders(&home.join("projects")) {
            for file in files(&project) {
                let name = name_of(&file);
                if let Some(id) = name.strip_suffix(".jsonl")
                    && is_uuid(id)
                    && let Some(each) = found(&file, id)
                {
                    found_files.push(each);
                }
                if found_files.len() >= limit {
                    break 'walk;
                }
            }
        }
    }
    SessionListing::new(
        home.map(|home| home.to_string_lossy().into_owned()),
        DEFAULT_MAX_BYTES,
        listing(found_files, request, |path| peek(&json_lines(path))),
    )
}

/// The first thing the person said and the folder it was said in, from the JSON lines at the start of a session file.
fn peek(lines: &[String]) -> (Option<String>, Option<String>) {
    let (mut title, mut cwd) = (None, None);
    for line in lines {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if cwd.is_none() {
            cwd = value
                .get("cwd")
                .and_then(Value::as_str)
                .map(tidy)
                .filter(|cwd| !cwd.is_empty());
        }
        if title.is_none() {
            title = prompt_text(&value)
                .map(|said| tidy(&said))
                .filter(|said| !said.is_empty());
        }
        if title.is_some() && cwd.is_some() {
            break;
        }
    }
    (title, cwd)
}

/// The text of a line that is the person speaking, not something the program put in their place.
fn prompt_text(value: &Value) -> Option<String> {
    if value.get("type")?.as_str()? != "user"
        || value.get("isMeta").and_then(Value::as_bool) == Some(true)
        || value.get("isSidechain").and_then(Value::as_bool) == Some(true)
    {
        return None;
    }
    let content = value.get("message")?.get("content")?;
    let text = match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .find_map(|block| block.get("text").and_then(Value::as_str))?
            .to_owned(),
        _ => return None,
    };
    // Commands, caveats and injected instructions are the program's own, not the person's words.
    let trimmed = text.trim_start();
    (!trimmed.is_empty() && !trimmed.starts_with('<') && !trimmed.starts_with("# AGENTS.md"))
        .then_some(text)
}

pub(crate) fn session_history(request: &HistoryRequest) -> Result<HistoryAnswer, ProviderError> {
    let document = export(&request.text, &request.session).map_err(|error| {
        ProviderError::new(
            coqui_acp_api::guatiao::Status::GUATIAO_ERR_BAD_VALUE,
            &error.to_string(),
        )
    })?;
    if !document
        .lines
        .iter()
        .any(|line| matches!(line.body, Body::Prompt { .. }))
    {
        return Err(ProviderError::new(
            coqui_acp_api::guatiao::Status::GUATIAO_ERR_BAD_VALUE,
            "it holds no conversation",
        ));
    }
    let bytes = document.to_vec().map_err(|error| {
        ProviderError::new(
            coqui_acp_api::guatiao::Status::GUATIAO_ERR_INTERNAL,
            &error.to_string(),
        )
    })?;
    let title = peek(&window_lines(&request.text)).0;
    Ok(HistoryAnswer::new(
        String::from_utf8_lossy(&bytes).into_owned(),
        title,
    ))
}
