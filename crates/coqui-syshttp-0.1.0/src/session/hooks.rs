//! URL hooks: a wrapper a curl front end runs, or code a [`Client`](super::Client) calls, for
//! requests whose URL matches a pattern.
//!
//! Hooks declared in the environment, each variable read under the prefixes of an [`HttpEnv`]:
//! `SYSHTTP_HOOKS_<KEY>` (plural) declares a hook, `SYSHTTP_HOOK_<NAME>` (singular) is a variable
//! one hook run exports to its wrapper.
//!
//! ```text
//! SYSHTTP_HOOKS_<KEY>=<regex>               searched in the URL the caller asked for: the
//!                                           origin URL, never a proxy's or gateway's rewrite
//! SYSHTTP_HOOKS_<KEY>_CURL=<command>        a curl front end runs THIS instead of itself, with
//!                                           its own arguments appended
//! ```
//!
//! The environment declares only the wrapper a front end runs: this crate has no command line, so
//! it ports the data ([`Hook`], [`matching`], [`curl_command`], [`wrapper_environment`]). The code
//! a client calls is given in code: [`ClientBuilder::hook`](super::ClientBuilder::hook).
//!
//! The wrapper is a shell-quoted command line, a program and its own arguments, that the front
//! end's whole argument list is appended to: `SYSHTTP_HOOKS_X_CURL='cmd fetch --'` runs
//! `cmd fetch -- "$@"`. Quote (single or double) what has spaces; a backslash is literal on every
//! platform, so a Windows path needs no doubling. The wrapper receives
//! ([`wrapper_environment`]) `SYSHTTP_HOOK_URL` (the requested URL), `SYSHTTP_HOOK_KEY` (which
//! hook matched), `SYSHTTP_HOOK_CURL` naming the front end so it can call curl back, and
//! `SYSHTTP_HOOK_SKIP` carrying its KEY, so that call does not run the same wrapper again.
//!
//! Several hooks may match one URL: they are taken in KEY order (sorted), and a front end runs the
//! first. A pattern that does not compile is a configuration error naming the variable, the first
//! time it would be consulted.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use std::path::Path;

use regex::Regex;

use super::env::HttpEnv;
use super::error::Error;

/// The prefix of the variables that declare a hook.
pub const HOOKS: &str = "SYSHTTP_HOOKS_";
/// The suffix of the variable that names a hook's wrapper.
pub const CURL_SUFFIX: &str = "_CURL";
/// The KEYs a front end must not run again (comma list).
pub const HOOK_SKIP: &str = "SYSHTTP_HOOK_SKIP";
/// What a wrapper is given: the URL requested.
pub const HOOK_URL: &str = "SYSHTTP_HOOK_URL";
/// What a wrapper is given: the KEY of the hook that matched.
pub const HOOK_KEY: &str = "SYSHTTP_HOOK_KEY";
/// What a wrapper is given: the front end to call back.
pub const HOOK_CURL: &str = "SYSHTTP_HOOK_CURL";

/// The target a hook has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HookKind {
    /// A wrapper a curl front end runs.
    Curl,
}

/// One declared hook.
#[derive(Debug, Clone)]
pub struct Hook {
    /// The KEY, upper-cased.
    pub key: String,
    /// What is searched in the URL.
    pub pattern: Regex,
    /// The wrapper command line.
    pub curl: Option<String>,
    /// The name of the variable declaring it (`SYSHTTP_HOOKS_X`), which every message names.
    pub variable: Option<String>,
}

impl Hook {
    /// Whether the pattern is found in `url`.
    #[must_use]
    pub fn matches(&self, url: &str) -> bool {
        self.pattern.is_match(url)
    }

    fn variable_name(&self) -> String {
        self.variable
            .clone()
            .unwrap_or_else(|| alloc::format!("{HOOKS}{}", self.key))
    }
}

/// Every configured hook, in KEY order. KEYs are upper-cased (variable names are compared as
/// Windows compares them), so `SYSHTTP_HOOKS_mykey` and `SYSHTTP_HOOKS_MYKEY` are one hook; the
/// first prefix that declares a variable of the hook wins for that variable. A `_CURL` variable
/// whose KEY has no pattern is ignored, and so is a blank target and a hook with no target.
///
/// # Errors
///
/// [`Error::Configuration`] naming the variable of a pattern that does not compile (never its
/// text).
pub fn from_env(env: &HttpEnv) -> Result<Vec<Hook>, Error> {
    let found: BTreeMap<String, (String, String)> = env
        .family(HOOKS)?
        .into_iter()
        .map(|(rest, value, variable)| (rest.to_ascii_uppercase(), (value, variable)))
        .collect();
    let mut hooks = Vec::new();
    for (key, (value, variable)) in &found {
        if key.ends_with(CURL_SUFFIX) || key.ends_with("_PY") {
            continue;
        }
        let pattern = Regex::new(value)
            .map_err(|_| Error::config(variable.clone(), "is not a valid regular expression"))?;
        let curl = found
            .get(&alloc::format!("{key}{CURL_SUFFIX}"))
            .map(|(target, _)| target.clone());
        if curl.is_some() {
            hooks.push(Hook {
                key: key.clone(),
                pattern,
                curl,
                variable: Some(variable.clone()),
            });
        }
    }
    Ok(hooks)
}

/// The KEYs `SYSHTTP_HOOK_SKIP` (a comma list) says not to run, upper-cased.
#[must_use]
pub fn skipped(env: &HttpEnv) -> BTreeSet<String> {
    env.get(&[HOOK_SKIP])
        .ok()
        .flatten()
        .unwrap_or_default()
        .split(',')
        .map(|key| key.trim().to_ascii_uppercase())
        .filter(|key| !key.is_empty())
        .collect()
}

/// The hooks for `url` (the caller's URL), in KEY order, minus the skipped ones; `kind` keeps only
/// hooks with that target.
///
/// # Errors
///
/// As [`from_env`].
pub fn matching(env: &HttpEnv, url: &str, kind: Option<HookKind>) -> Result<Vec<Hook>, Error> {
    let skip = skipped(env);
    Ok(from_env(env)?
        .into_iter()
        .filter(|hook| !skip.contains(&hook.key.to_ascii_uppercase()) && hook.matches(url))
        .filter(|hook| match kind {
            None => true,
            Some(HookKind::Curl) => hook.curl.is_some(),
        })
        .collect())
}

/// A shell-quoted command line into words: single and double quotes group words with spaces, a
/// backslash is literal (no escaping on any platform, so `C:\Tools\wrap.exe` stays what it is),
/// `#` starts no comment. An unclosed quote runs to the end.
#[must_use]
pub fn split_command(value: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    for character in value.chars() {
        match (quote, character) {
            (Some(open), c) if c == open => quote = None,
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => {
                quote = Some(character);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    words.push(core::mem::take(&mut word));
                    started = false;
                }
            }
            (None, c) => {
                word.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}

/// The words a `_CURL` target means, the front end's arguments to be appended: the shell-quoted
/// command line split; a bare path to an existing file (spaces and all) is that one program; a
/// first word that is a `.py` file runs under `python` (default `python`).
///
/// # Errors
///
/// [`Error::Configuration`] naming the variable for a target that is empty.
pub fn curl_command(hook: &Hook, python: Option<&Path>) -> Result<Vec<String>, Error> {
    let value = hook.curl.as_deref().unwrap_or("").trim();
    let variable = alloc::format!("{}{CURL_SUFFIX}", hook.variable_name());
    if value.is_empty() {
        return Err(Error::config(variable, "is empty"));
    }
    let mut words = if Path::new(value).is_file() {
        alloc::vec![value.to_string()]
    } else {
        split_command(value)
    };
    if words.is_empty() {
        return Err(Error::config(variable, "holds no command"));
    }
    if words[0].to_ascii_lowercase().ends_with(".py") && Path::new(&words[0]).is_file() {
        let interpreter = python.map_or_else(
            || "python".to_string(),
            |path| path.to_string_lossy().into_owned(),
        );
        words.insert(0, interpreter);
    }
    Ok(words)
}

/// The variables a `_CURL` wrapper run adds to its environment: `SYSHTTP_HOOK_URL` (`url`),
/// `SYSHTTP_HOOK_KEY`, `SYSHTTP_HOOK_CURL` (`entry`, the front end to call back) and
/// `SYSHTTP_HOOK_SKIP` with this KEY added, so the call back does not run the wrapper again; each
/// under the first prefix.
///
/// # Errors
///
/// [`Error::Configuration`] for a malformed `SYSHTTP_PREFIX`.
pub fn wrapper_environment(
    hook: &Hook,
    url: &str,
    entry: &str,
    env: &HttpEnv,
) -> Result<BTreeMap<String, String>, Error> {
    let mut skip = skipped(env);
    skip.insert(hook.key.to_ascii_uppercase());
    let skip = skip.into_iter().collect::<Vec<_>>().join(",");
    Ok(BTreeMap::from([
        (env.name(HOOK_URL)?, url.to_string()),
        (env.name(HOOK_KEY)?, hook.key.clone()),
        (env.name(HOOK_CURL)?, entry.to_string()),
        (env.name(HOOK_SKIP)?, skip),
    ]))
}
