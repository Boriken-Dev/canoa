//! The places a program store's install may not declare a change in, and the variables it may not be given: lists in gitignore
//! syntax, which the person edits in canoa.cfg's `[host.store]`.
//!
//! A line is a pattern; a leading `!` allows what an earlier line protected, and the last line that matches wins. A pattern
//! that matches a directory covers everything under it. `*` and `?` stand for part of a name, `**` for any number of names, `#`
//! starts a comment. Where a pattern starts says what it is relative to: `~/` the home directory, a drive (`C:/x`) or `/` the
//! file system, anything else any place (`.env` matches every file of that name, `a/b` every `b` under an `a`). Names are compared
//! whatever their case. The syntax is the one of coqui-acp-pi's `protected` setting.
//!
//! The host holds no list of its own in code beyond the two recommendations here, which a list the person writes replaces.

use std::path::{Component, Path};

/// What the host recommends keeping a store's declared changes out of, for a person who has not written a list of their own:
/// what a login or a tool reads and can be made to run, and the folders programs share (a program may declare a place inside
/// one, never the folder). Written out in a list to keep, change or remove any of it.
pub const RECOMMENDED_PLACES: &[&str] = &[
    "# Shared folders: a program's own place inside one is fine, the folder itself is not.",
    "~/.config",
    "!~/.config/*",
    "~/.local",
    "!~/.local/*/*",
    "~/AppData",
    "!~/AppData/*/*",
    "~/Library",
    "!~/Library/*/*",
    "~/Documents",
    "!~/Documents/*",
    "# What a login, a shell or the system reads, and keys.",
    "~/.ssh",
    "~/.gnupg",
    "~/.config/autostart",
    "~/.config/systemd",
    "~/.config/fish",
    "~/Library/LaunchAgents",
    "~/AppData/Roaming/Microsoft/Windows/Start Menu",
];

/// What the host recommends never passing on to a store or its install, for a person who has not written a list of their own:
/// variables that look like a credential, the ssh agent and the host's own settings. Matched against a variable's name.
pub const RECOMMENDED_VARIABLES: &[&str] = &[
    "*_API_KEY",
    "*TOKEN",
    "*_SECRET",
    "*PASSWORD",
    "*_KEY",
    "*_PAT",
    "*APIKEY",
    "*_CREDENTIALS",
    "*_CREDENTIAL",
    "SSH_AUTH_SOCK",
    "DOTAGENTS_*",
];

/// What a pattern is relative to.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Anchor {
    Home,
    Root,
    Anywhere,
}

#[derive(Clone, Debug)]
struct Rule {
    allow: bool,
    anchor: Anchor,
    names: Vec<String>,
}

/// A list of patterns, ready to match paths and names against.
#[derive(Clone, Debug, Default)]
pub struct Protected {
    rules: Vec<Rule>,
}

fn fold(text: &str) -> String {
    text.to_lowercase()
}

/// The names of a path, drive first on Windows, lower case.
fn names_of(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|part| match part {
            Component::Prefix(prefix) => {
                // `\\?\C:` (what links followed make of a drive) is the drive `C:`.
                let text = prefix.as_os_str().to_string_lossy().into_owned();
                let drive = text
                    .rfind(':')
                    .filter(|at| *at >= 1)
                    .map_or(text.as_str(), |at| &text[at - 1..=at]);
                Some(fold(drive))
            }
            Component::Normal(name) => Some(fold(&name.to_string_lossy())),
            _ => None,
        })
        .collect()
}

fn glob(pattern: &[char], name: &[char]) -> bool {
    match pattern.split_first() {
        None => name.is_empty(),
        Some(('*', rest)) => (0..=name.len()).any(|skip| glob(rest, &name[skip..])),
        Some(('?', rest)) => !name.is_empty() && glob(rest, &name[1..]),
        Some((c, rest)) => name.first() == Some(c) && glob(rest, &name[1..]),
    }
}

/// Whether `pattern` (names, `**` for any number) is the whole of `names`.
fn fits(pattern: &[String], names: &[String]) -> bool {
    match pattern.split_first() {
        None => names.is_empty(),
        Some((first, rest)) if first == "**" => {
            (0..=names.len()).any(|skip| fits(rest, &names[skip..]))
        }
        Some((first, rest)) => {
            let (pattern_chars, name_chars): (Vec<char>, Vec<char>) = (
                first.chars().collect(),
                names
                    .first()
                    .map(|n| n.chars().collect())
                    .unwrap_or_default(),
            );
            !names.is_empty() && glob(&pattern_chars, &name_chars) && fits(rest, &names[1..])
        }
    }
}

impl Protected {
    /// The list `lines` write; a blank line and a `#` line are nothing.
    #[must_use]
    pub fn new<S: AsRef<str>>(lines: impl IntoIterator<Item = S>) -> Self {
        let mut rules = Vec::new();
        for line in lines {
            let mut text = line.as_ref().trim().replace('\\', "/");
            if text.is_empty() || text.starts_with('#') {
                continue;
            }
            let allow = text.starts_with('!');
            if allow {
                text.remove(0);
            }
            let (anchor, rest) = if let Some(rest) = text.strip_prefix("~/") {
                (Anchor::Home, rest.to_owned())
            } else if text.starts_with('/') {
                (Anchor::Root, text.trim_start_matches('/').to_owned())
            } else if text.as_bytes().get(1) == Some(&b':')
                && text.as_bytes()[0].is_ascii_alphabetic()
            {
                (Anchor::Root, text.clone())
            } else {
                (Anchor::Anywhere, text.clone())
            };
            let names: Vec<String> = rest
                .split('/')
                .filter(|name| !name.is_empty() && *name != ".")
                .map(fold)
                .collect();
            if !names.is_empty() {
                rules.push(Rule {
                    allow,
                    anchor,
                    names,
                });
            }
        }
        Self { rules }
    }

    /// Whether the last line that matches `names` (a path's names, lower case) protects it. `home` is the names of the directory
    /// `~/` means.
    fn protects_names(&self, names: &[String], home: Option<&[String]>) -> bool {
        let mut protected = false;
        for rule in &self.rules {
            let hit = (1..=names.len()).any(|end| match rule.anchor {
                Anchor::Anywhere => (0..end).any(|start| fits(&rule.names, &names[start..end])),
                Anchor::Root => fits(&rule.names, &names[..end]),
                Anchor::Home => home.is_some_and(|home| {
                    end > home.len()
                        && names[..home.len()] == *home
                        && fits(&rule.names, &names[home.len()..end])
                }),
            });
            if hit {
                protected = !rule.allow;
            }
        }
        protected
    }

    /// Whether the last line that matches `path` (already worked out) protects it. `home` is the directory `~/` means.
    #[must_use]
    pub fn protects(&self, path: &Path, home: Option<&Path>) -> bool {
        let home = home.map(names_of);
        self.protects_names(&names_of(path), home.as_deref())
    }

    /// Whether `path` is a proper ancestor of a place the list names outright (no wildcard, relative to the home directory or the
    /// file system): claiming it would claim that place too.
    #[must_use]
    pub fn holds(&self, path: &Path, home: Option<&Path>) -> bool {
        let names = names_of(path);
        let home = home.map(names_of);
        self.rules
            .iter()
            .filter(|rule| !rule.allow && !rule.names.iter().any(|name| name.contains(['*', '?'])))
            .any(|rule| {
                let place: Vec<String> = match (&rule.anchor, &home) {
                    (Anchor::Home, Some(home)) => home.iter().chain(&rule.names).cloned().collect(),
                    (Anchor::Root, _) => rule.names.clone(),
                    _ => return false,
                };
                names.len() < place.len() && place[..names.len()] == names[..]
            })
    }

    /// Whether the last line that matches the variable `name` protects it; case does not matter.
    #[must_use]
    pub fn protects_name(&self, name: &str) -> bool {
        self.protects_names(&[fold(name)], None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn home() -> PathBuf {
        PathBuf::from(if cfg!(windows) { "C:\\h" } else { "/h" })
    }

    fn at(rest: &str) -> PathBuf {
        let mut path = home();
        for name in rest.split('/') {
            path.push(name);
        }
        path
    }

    fn list(lines: &[&str]) -> Protected {
        Protected::new(lines.iter().copied())
    }

    #[test]
    fn a_pattern_covers_what_is_under_it_and_a_later_line_wins() {
        let rules = list(&["~/.ssh", "!~/.ssh/known_hosts", "~/.ssh/known_hosts2"]);
        assert!(rules.protects(&at(".ssh/id_rsa"), Some(&home())));
        assert!(rules.protects(&at(".ssh"), Some(&home())));
        assert!(!rules.protects(&at(".ssh/known_hosts"), Some(&home())));
        assert!(rules.protects(&at(".ssh/known_hosts2"), Some(&home())));
        assert!(!rules.protects(&at(".sshx/id"), Some(&home())));
        assert!(
            !rules.protects(&at("other/.ssh"), Some(&home())),
            "anchored at the home directory"
        );
    }

    #[test]
    fn a_pattern_without_an_anchor_matches_anywhere_and_wildcards_work() {
        let rules = list(&["# secrets", "", ".env", "keys/*.pem", "**/vault/**/token"]);
        assert!(rules.protects(&at("a/b/.env"), Some(&home())));
        assert!(rules.protects(&at("a/keys/x.pem"), Some(&home())));
        assert!(!rules.protects(&at("a/keys/x.txt"), Some(&home())));
        assert!(rules.protects(&at("p/vault/q/r/token"), Some(&home())));
        assert!(!rules.protects(&at("p/vault/q/r/tokens"), Some(&home())));
    }

    #[test]
    fn what_is_not_in_the_list_is_not_protected() {
        let rules = list(&[]);
        assert!(!rules.protects(&at(".ssh/id_rsa"), Some(&home())));
        let rules = list(&["~/x"]);
        assert!(
            !rules.protects(&at("x"), None),
            "no home directory, no `~/` entry"
        );
    }

    #[test]
    fn the_recommended_list_is_a_list_a_person_can_cut() {
        let all = Protected::new(RECOMMENDED_PLACES.iter().copied());
        assert!(all.protects(&at(".ssh/id_rsa"), Some(&home())));
        assert!(all.protects(&at(".config"), Some(&home())));
        assert!(
            !all.protects(&at(".config/echo"), Some(&home())),
            "a program's own place inside a shared folder"
        );
        assert!(all.protects(&at(".config/autostart/x.desktop"), Some(&home())));
        assert!(all.protects(&at(".local/bin"), Some(&home())));
        assert!(!all.protects(&at(".local/bin/claude"), Some(&home())));
        let mine = list(&["~/work/secrets"]);
        assert!(!mine.protects(&at(".ssh/id_rsa"), Some(&home())));
        assert!(mine.protects(&at("work/secrets/a"), Some(&home())));
    }

    #[test]
    fn a_path_that_holds_a_named_place_is_held() {
        let rules = list(&[
            "~/AppData/Roaming/Microsoft/Windows/Start Menu",
            "~/x/*",
            "!~/y",
        ]);
        assert!(rules.holds(&at("AppData"), Some(&home())));
        assert!(rules.holds(&at("AppData/Roaming/Microsoft"), Some(&home())));
        assert!(!rules.holds(
            &at("AppData/Roaming/Microsoft/Windows/Start Menu"),
            Some(&home())
        ));
        assert!(
            !rules.holds(&at("x"), Some(&home())),
            "a wildcard names no place"
        );
        assert!(!rules.holds(&at("y"), Some(&home())), "an allow names none");
        assert!(!rules.holds(&at("AppData"), None), "no home directory");
    }

    #[test]
    fn variable_names_are_matched_whatever_their_case() {
        let rules = Protected::new(RECOMMENDED_VARIABLES.iter().copied());
        for name in [
            "GITHUB_TOKEN",
            "github_token",
            "AWS_SECRET",
            "OPENAI_API_KEY",
            "SSH_AUTH_SOCK",
            "DOTAGENTS_X",
            "MY_PASSWORD",
        ] {
            assert!(rules.protects_name(name), "{name}");
        }
        for name in ["PATH", "HOME", "TOKENIZER_DIR", "KEYBOARD"] {
            assert!(!rules.protects_name(name), "{name}");
        }
        let cut = list(&["*TOKEN", "!GITHUB_TOKEN"]);
        assert!(cut.protects_name("NPM_TOKEN"));
        assert!(!cut.protects_name("GITHUB_TOKEN"));
    }
}
