//! The `panel_tab` kind: a kind of tab the application's bottom panel can open.
//!
//! Which kinds of tab the panel offers is not Canoa's to say: each is a provider of this kind, the ones Canoa carries (a terminal,
//! the host's log) and the ones an extension adds alike. A provider describes itself (its id, name, icon and what the tab shows) and,
//! for a log, answers the latest lines the application polls for. A tab can also stand for a page the extension serves on this
//! computer's loopback, which the application opens in the person's own browser.
//!
//! The models are plain data, the same in every language.

use alloc::string::String;

use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::{FromValue, Schema, Status, ToValue};

/// The name of the `panel_tab` kind.
pub const KIND: &str = "panel_tab";

/// The versions of the `panel_tab` kind this host offers.
pub const KIND_VERSIONS: [&str; 1] = ["1"];

/// The most lines a [`PanelTab::tail`] answers.
pub const MAX_TAIL_LINES: u32 = 5000;

/// An error with status `GUATIAO_ERR_INTERNAL`.
#[must_use]
pub fn internal_error(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_INTERNAL, why.as_ref())
}

/// What a tab shows.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
pub enum Shows {
    /// A shell on the host: the one named by [`TabInfo::shell`], or the host's own choice.
    Terminal,
    /// Lines of text the provider keeps, which [`PanelTab::tail`] answers and the application shows as they come.
    Log,
    /// A page the extension serves on this computer's loopback, at [`TabInfo::url`]: the tab names it and opens it in the person's
    /// browser, since the application has no web view of its own.
    Page,
}

/// What a kind of tab is, from [`PanelTab::describe`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct TabInfo {
    /// The id the kind is found by: a word of lower-case letters, digits and `_`, 1 to 32.
    pub id: String,
    /// Its name for people, as the new-tab menu and a tab say it.
    pub title: String,
    /// A line saying what it is.
    pub help: Option<String>,
    /// The name of one of the application's icons; a name it does not know draws the default.
    pub icon: Option<String>,
    /// What a tab of this kind shows.
    pub shows: Shows,
    /// For [`Shows::Terminal`]: the id of the shell the host lists that a new tab runs; none for the host's own choice.
    pub shell: Option<String>,
    /// For [`Shows::Page`]: the address of the page, `http` or `https` on this computer's loopback (`127.0.0.1`, `[::1]` or
    /// `localhost`, with or without a port), which the extension's own server answers.
    pub url: Option<String>,
}

impl TabInfo {
    /// A kind of tab showing `shows`.
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>, shows: Shows) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            help: None,
            icon: None,
            shows,
            shell: None,
            url: None,
        }
    }

    /// A tab that opens the page at `url`.
    #[must_use]
    pub fn page(id: impl Into<String>, title: impl Into<String>, url: impl Into<String>) -> Self {
        let mut info = Self::new(id, title, Shows::Page);
        info.url = Some(url.into());
        info
    }

    /// Why this provider cannot be offered, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        let word = !self.id.is_empty()
            && self.id.len() <= 32
            && self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
        if !word {
            return Some(alloc::format!(
                "the tab id {:?} is not 1 to 32 lower-case letters, digits and _",
                self.id
            ));
        }
        if self.title.trim().is_empty() {
            return Some(alloc::format!("the tab {:?} has no title", self.id));
        }
        if self.shell.is_some() && self.shows != Shows::Terminal {
            return Some(alloc::format!(
                "the tab {:?} names a shell but shows no terminal",
                self.id
            ));
        }
        match (&self.url, self.shows) {
            (Some(_), shows) if shows != Shows::Page => Some(alloc::format!(
                "the tab {:?} names an address but shows no page",
                self.id
            )),
            (None, Shows::Page) => Some(alloc::format!(
                "the tab {:?} shows a page and names none",
                self.id
            )),
            (Some(url), _) => loopback_problem(url)
                .map(|why| alloc::format!("the tab {:?} names {url:?}, which {why}", self.id)),
            (None, _) => None,
        }
    }
}

/// Why `url` is no page this computer's loopback serves, or `None` when it is one: `http` or `https`, the host `127.0.0.1`, `[::1]` or
/// `localhost` and a port that is a number when there is one, with no user information and only printable ASCII. A page is the
/// extension's own and opens in the person's browser, so what it names is a server on this computer and not another site.
#[must_use]
pub fn loopback_problem(url: &str) -> Option<&'static str> {
    if !url.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Some("is not printable ASCII");
    }
    let Some(rest) = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
    else {
        return Some("is not an http or https address");
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return Some("carries user information");
    }
    let (host, port) = match authority.strip_prefix('[') {
        Some(inner) => match inner.split_once(']') {
            Some((host, after)) => (alloc::format!("[{host}]"), after.strip_prefix(':')),
            None => return Some("has an unclosed address"),
        },
        None => match authority.split_once(':') {
            Some((host, port)) => (String::from(host), Some(port)),
            None => (String::from(authority), None),
        },
    };
    if !(host == "127.0.0.1" || host == "[::1]" || host.eq_ignore_ascii_case("localhost")) {
        return Some("is not on this computer's loopback (127.0.0.1, [::1] or localhost)");
    }
    if port.is_some_and(|port| port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit())) {
        return Some("has a port that is not a number");
    }
    None
}

/// How many of the latest lines a log is asked for.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct TailRequest {
    /// At most [`MAX_TAIL_LINES`].
    pub lines: u32,
}

impl TailRequest {
    /// The latest `lines` lines.
    #[must_use]
    pub const fn new(lines: u32) -> Self {
        Self { lines }
    }
}

/// A kind of tab.
///
/// `describe` answers the same on every call with nothing run. `tail` is asked only of a [`Shows::Log`] kind.
#[coqui_acp_api::guatiao::kind(name = "panel_tab", crate = ::coqui_acp_api::guatiao)]
pub trait PanelTab: Send + Sync {
    /// What this kind of tab is.
    fn describe(&self) -> Result<TabInfo, ProviderError>;

    /// The latest lines of the log, oldest first, each ended by a newline; empty for a kind that shows no log.
    fn tail(&self, request: TailRequest) -> Result<String, ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_with_a_word_for_an_id_and_a_title_is_usable_and_a_shell_needs_a_terminal() {
        assert_eq!(TabInfo::new("logs", "Logs", Shows::Log).problem(), None);
        let mut info = TabInfo::new("dev", "Dev shell", Shows::Terminal);
        info.shell = Some("git-bash".into());
        assert_eq!(info.problem(), None);
        info.shows = Shows::Log;
        assert!(info.problem().is_some_and(|why| why.contains("shell")));
    }

    #[test]
    fn an_id_that_is_no_word_or_a_title_that_is_empty_is_refused() {
        for id in ["", "Logs", "a b", &"a".repeat(33)] {
            assert!(
                TabInfo::new(id, "T", Shows::Log).problem().is_some(),
                "{id:?}"
            );
        }
        assert!(TabInfo::new("a", " ", Shows::Log).problem().is_some());
    }

    #[test]
    fn a_page_names_an_address_on_this_computers_loopback_and_nothing_else_names_one() {
        for url in [
            "http://127.0.0.1:8123/",
            "https://localhost/app?x=1#top",
            "http://[::1]:9/a",
            "http://LOCALHOST:80",
        ] {
            assert_eq!(TabInfo::page("docs", "Docs", url).problem(), None, "{url}");
        }
        for url in [
            "http://example.com/",
            "http://127.0.0.1.example.com/",
            "http://localhost.evil.test/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "http://user@127.0.0.1/",
            "http://127.0.0.1:port/",
            "http://127.0.0.1:/",
            "http://[::1/",
            "http://127.0.0.1/\u{e9}",
            "127.0.0.1:80",
        ] {
            assert!(
                TabInfo::page("docs", "Docs", url).problem().is_some(),
                "{url}"
            );
        }
        assert!(
            TabInfo::new("docs", "Docs", Shows::Page)
                .problem()
                .is_some_and(|why| why.contains("names none"))
        );
        let mut log = TabInfo::new("logs", "Logs", Shows::Log);
        log.url = Some("http://127.0.0.1/".into());
        assert!(
            log.problem()
                .is_some_and(|why| why.contains("shows no page"))
        );
    }
}
