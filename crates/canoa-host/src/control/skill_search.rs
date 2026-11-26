//! `skill/search`: the search of a skills directory (skills.sh), for the Skills page, once the person has turned it on.
//!
//! The answer is a list of names to look at, nothing to install: a result names the repository that holds a skill, and installing
//! one goes through `skill/scan` and `skill/fetch` on that repository like any other git source. The directory is a third party
//! and its search address is not documented, so it is read only while `[store] skills_search` names it (`true` for skills.sh, or
//! the address of another that answers the same), with no redirect followed and the answer read as plain data.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the clock is std's"
)]

use canoa::brand::CONFIG_FILE;
use std::time::Duration;

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Value, json};
use url::Url;

use super::{Control, REFUSED, bad, text_param};

type Outcome = Result<Value, (i32, String)>;

/// The directory `skills_search = true` names.
pub(crate) const DEFAULT_BASE: &str = "https://skills.sh";
/// How long the directory has to answer, and again to send its body.
const SEARCH_TIME: Duration = Duration::from_secs(20);
/// The most bytes of an answer that are read; fifty results are a few kilobytes.
const MAX_SEARCH_BYTES: u64 = 1024 * 1024;
/// The fewest and most characters of a query: the directory refuses one shorter than two.
const QUERY_CHARS: core::ops::RangeInclusive<usize> = 2..=128;
/// How many results one search asks for, unless the request says, and the most it may.
const DEFAULT_LIMIT: u64 = 20;
const MAX_LIMIT: u64 = 50;
/// The most characters of any text in a result.
const MAX_TEXT: usize = 256;

/// The address `[store] skills_search` names, or why the search is off.
pub(crate) fn search_base(store: &Value) -> Result<String, (i32, String)> {
    match store.get("skills_search") {
        Some(Value::Bool(true)) => Ok(DEFAULT_BASE.to_owned()),
        Some(Value::String(address)) => directory_base(address),
        _ => Err((
            REFUSED,
            format!(
                "Skill search is off: turn it on in Store settings, or write skills_search = true under [store] in {CONFIG_FILE}"
            ),
        )),
    }
}

/// `address` as the directory's base: an http or https address, no user information, query or fragment. Plain http is the person's
/// choice wherever it goes, and the answer to a search says so.
fn directory_base(address: &str) -> Result<String, (i32, String)> {
    let refused = |why: &str| {
        (
            REFUSED,
            format!("[store] skills_search is not a directory's address: {why}"),
        )
    };
    if address.len() > 2048
        || address
            .chars()
            .any(|c| c.is_control() || c == ' ' || c == '\\')
    {
        return Err(refused("it holds a space, control character or backslash"));
    }
    let url = Url::parse(address).map_err(|_| refused("it is not an address"))?;
    if url.host().is_none() {
        return Err(refused("it has no host"));
    }
    if !matches!(url.scheme(), "http" | "https") {
        return Err(refused("it is an http or https address"));
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(refused("it holds user information, a query or a fragment"));
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

/// A line of text from the directory: not empty, not long, no control character.
fn line(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).filter(|text| {
        !text.is_empty() && text.chars().count() <= MAX_TEXT && !text.chars().any(char::is_control)
    })
}

/// Whether `text` is a GitHub `owner/repo`: the only place a result may name, since a third party's answer must not point the
/// app at a folder, an address or a git remote of its choosing.
fn repository(text: &str) -> bool {
    let plain = |part: &str| {
        (1..=100).contains(&part.len())
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            && part.starts_with(|c: char| c.is_ascii_alphanumeric())
    };
    text.split_once('/')
        .is_some_and(|(owner, name)| plain(owner) && plain(name))
}

/// The results of an answer, each with only the members the page reads. One that lacks its repository or its skill, or whose
/// repository is not an `owner/repo` or whose skill is not a plain name, is left out.
fn clean_results(answer: &Value, limit: usize) -> Option<Vec<Value>> {
    let listed = answer.get("skills")?.as_array()?;
    Some(
        listed
            .iter()
            .filter_map(|each| {
                let source = line(each.get("source")).filter(|source| repository(source))?;
                let skill = line(each.get("skillId"))?;
                let name = line(each.get("name")).unwrap_or(skill);
                if !skill
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                    || skill.starts_with(['.', '-'])
                {
                    return None;
                }
                Some(json!({
                    "id": line(each.get("id")).map_or_else(|| format!("{source}/{skill}"), str::to_owned),
                    "source": source,
                    "skillId": skill,
                    "name": name,
                    "installs": each.get("installs").and_then(Value::as_u64).unwrap_or(0),
                }))
            })
            .take(limit)
            .collect(),
    )
}

impl Control {
    /// `skill/search` `{"query", "limit"?}` -> `{"from", "skills": [{"id", "source", "skillId", "name", "installs"}], "warning"?}`: skills
    /// whose names match `query` (two characters at least), as the directory `[store] skills_search` names lists them, most
    /// installed first. Refused while that key is absent or false. `source` is the repository the skill lives in, as
    /// `owner/repo`. `warning` is there when the directory is plain http beyond this computer.
    pub(super) fn skill_search(&self, params: &Value) -> Outcome {
        let query = text_param(params, "query")?
            .ok_or_else(|| bad("query is needed"))?
            .trim();
        if !QUERY_CHARS.contains(&query.chars().count()) || query.chars().any(char::is_control) {
            return Err(bad(
                "query is 2 to 128 characters of text, with no control character",
            ));
        }
        let limit = match params.get("limit") {
            None | Some(Value::Null) => DEFAULT_LIMIT,
            Some(value) => value
                .as_u64()
                .filter(|limit| (1..=MAX_LIMIT).contains(limit))
                .ok_or_else(|| bad(&format!("limit is a whole number from 1 to {MAX_LIMIT}")))?,
        };
        let store = self
            .files
            .config_get(&json!({"section": "store"}))
            .ok()
            .and_then(|answer| answer.get("value").cloned())
            .unwrap_or(Value::Null);
        let base = search_base(&store)?;
        let url = format!(
            "{base}/api/search?q={}&limit={limit}",
            utf8_percent_encode(query, NON_ALPHANUMERIC)
        );
        let body = self
            .registry
            .read(&url, "skills search", SEARCH_TIME, MAX_SEARCH_BYTES)?;
        let skills = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|answer| clean_results(&answer, usize::try_from(limit).unwrap_or(1)))
            .ok_or_else(|| {
                (
                    REFUSED,
                    "The skills search gave an answer Canoa cannot read".to_owned(),
                )
            })?;
        let mut answer = json!({"from": base, "skills": skills});
        if let Some(note) = canoa::address::plain_http_note("[store] skills_search", &base) {
            answer["warning"] = Value::String(note);
        }
        Ok(answer)
    }
}

#[cfg(test)]
#[path = "skill_search_tests.rs"]
mod tests;
