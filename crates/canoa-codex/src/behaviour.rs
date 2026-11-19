//! What Canoa does for Codex beyond its description: the variables an account sets, where a login lives, the shell word and the
//! sessions its program kept in its own folder.
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
    Variable,
};
use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_history::Body;
use coqui_acp_history::conversation::is_uuid;
use serde_json::Value;

use crate::acp::accounts::{AUTH_FILE, CHATGPT, FILE_KEY, variables};
use crate::acp::history::{DEFAULT_MAX_BYTES, codex_home, export};

pub(crate) fn account_variables(grant: &coqui_acp_api::AccountGrant) -> Vec<Variable> {
    variables(grant)
        .into_iter()
        .map(|(name, value)| Variable::new(name, value))
        .collect()
}

pub(crate) fn login_file(kind: &str) -> Option<LoginFile> {
    (kind == CHATGPT).then(|| LoginFile::new(FILE_KEY, AUTH_FILE))
}

/// Codex has no setting for its shell: it looks for `pwsh` and `powershell` on the `PATH` on Windows and takes the account's login
/// shell elsewhere.
pub(crate) fn shell_advice() -> ShellAdvice {
    ShellAdvice::cannot(
        "Codex has no setting that names its shell: it looks for pwsh or powershell on the PATH on Windows and uses the account's login shell elsewhere",
    )
}

/// Where Codex looks for its sessions: `CODEX_HOME` or `.codex` in the home.
fn home_of(request: &SessionsRequest) -> Option<PathBuf> {
    request
        .home
        .as_ref()
        .map(PathBuf::from)
        .or_else(|| codex_home(&BTreeMap::new(), false))
}

/// Whether the character before `id` in `name` is the dash that joins it to the time.
fn rest_is_dash(name: &str, id: &str) -> bool {
    name.strip_suffix(".jsonl")
        .and_then(|stem| stem.strip_suffix(id))
        .is_some_and(|front| front.ends_with('-'))
}

/// The sessions under `sessions/<year>/<month>/<day>/rollout-<time>-<uuid>.jsonl`.
pub(crate) fn sessions(request: &SessionsRequest) -> SessionListing {
    let home = home_of(request);
    let limit = usize::try_from(request.limit).unwrap_or(usize::MAX);
    let mut found_files = Vec::new();
    if let Some(home) = &home {
        'walk: for year in folders(&home.join("sessions")) {
            for month in folders(&year) {
                for day in folders(&month) {
                    for file in files(&day) {
                        let name = name_of(&file);
                        // `rollout-<time>-<id>.jsonl`: the id is the last 36 characters before the extension.
                        let id = name
                            .strip_prefix("rollout-")
                            .and_then(|rest| rest.strip_suffix(".jsonl"))
                            .filter(|rest| rest.len() > 36)
                            .and_then(|rest| rest.get(rest.len() - 36..));
                        if let Some(id) = id
                            && rest_is_dash(&name, id)
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
                .get("payload")
                .and_then(|payload| payload.get("cwd"))
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
    let payload = value.get("payload")?;
    if value.get("type")?.as_str()? != "response_item"
        || payload.get("type")?.as_str()? != "message"
        || payload.get("role")?.as_str()? != "user"
    {
        return None;
    }
    let text = payload
        .get("content")?
        .as_array()?
        .iter()
        .find_map(|block| block.get("text").and_then(Value::as_str))?
        .to_owned();
    // Commands, caveats and injected instructions are the program's own, not the person's words.
    let trimmed = text.trim_start();
    (!trimmed.is_empty() && !trimmed.starts_with('<') && !trimmed.starts_with("# AGENTS.md"))
        .then_some(text)
}

pub(crate) fn session_history(request: &HistoryRequest) -> Result<HistoryAnswer, ProviderError> {
    let document = export(&request.text, &request.session)
        .map_err(|error| ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &error.to_string()))?;
    if !document
        .lines
        .iter()
        .any(|line| matches!(line.body, Body::Prompt { .. }))
    {
        return Err(ProviderError::new(
            Status::GUATIAO_ERR_BAD_VALUE,
            "it holds no conversation",
        ));
    }
    let bytes = document
        .to_vec()
        .map_err(|error| ProviderError::new(Status::GUATIAO_ERR_INTERNAL, &error.to_string()))?;
    let title = peek(&window_lines(&request.text)).0;
    Ok(HistoryAnswer::new(
        String::from_utf8_lossy(&bytes).into_owned(),
        title,
    ))
}
