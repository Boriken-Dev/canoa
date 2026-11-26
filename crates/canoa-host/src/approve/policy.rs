//! The `policy` approver: the person's own rules, answered on the spot.
//!
//! A rule names what it matches (the call's kind, its tool's name, its command, the files it names) and what it says, `allow` or
//! `reject`, and the first rule a request matches answers it. A request no rule matches is abstained from, so it goes on to the next
//! approver and the window. Its allow is the person's rule and not the person: the host never applies it to a protected place.

use std::path::Path;

use canoa::approver::Ticket;
use canoa::approver::{Answer, Approver, ApproverInfo, Authority, Request};
use canoa::store::catalog::wildcard;
use canoa::store::protect::Protected;
use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::{FromValue, Provider, Schema, ToValue};

/// What a rule says.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
pub(crate) enum Verdict {
    /// Allow the call once.
    Allow,
    /// Refuse the call once.
    Reject,
}

/// One rule: every part it names must match.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
pub(crate) struct Rule {
    /// The agent's kind of call (`read`, `edit`, `execute`, ...), whatever its case.
    pub(crate) kind: Option<String>,
    /// The tool's name, with `*` for any run of characters.
    pub(crate) name: Option<String>,
    /// The command, with `*` for any run of characters.
    pub(crate) command: Option<String>,
    /// The files the call names, in the syntax of `[permissions] protected`: every file for an allow, any file for a reject.
    pub(crate) path: Option<String>,
    /// What it says.
    pub(crate) verdict: Verdict,
}

/// What an entry of the `policy` provider is set up with.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
pub(crate) struct PolicySettings {
    /// The rules, asked in order.
    #[map(default)]
    pub(crate) rules: Vec<Rule>,
}

struct Compiled {
    rule: Rule,
    path: Option<Protected>,
}

/// The built-in approver of the person's rules.
#[derive(Provider)]
#[provider(Approver, id = "policy", config = PolicySettings, crate = ::coqui_acp_api::guatiao)]
pub(crate) struct Policy {
    rules: Vec<Compiled>,
    home: Option<std::path::PathBuf>,
}

impl From<PolicySettings> for Policy {
    fn from(settings: PolicySettings) -> Self {
        Self {
            rules: settings
                .rules
                .into_iter()
                .map(|rule| Compiled {
                    path: rule
                        .path
                        .as_deref()
                        .map(|pattern| Protected::new([pattern])),
                    rule,
                })
                .collect(),
            home: std::env::home_dir(),
        }
    }
}

impl Compiled {
    fn matches(&self, request: &Request, home: Option<&Path>) -> bool {
        let rule = &self.rule;
        let tool = &request.tool;
        if rule
            .kind
            .as_deref()
            .is_some_and(|kind| !kind.eq_ignore_ascii_case(&tool.kind))
        {
            return false;
        }
        if rule
            .name
            .as_deref()
            .is_some_and(|name| !wildcard(name, &tool.name))
        {
            return false;
        }
        if let Some(pattern) = &rule.command
            && !tool
                .command
                .as_deref()
                .is_some_and(|command| wildcard(pattern, command))
        {
            return false;
        }
        if let Some(list) = &self.path {
            let fits = |path: &String| list.protects(Path::new(path), home);
            let ok = !tool.paths.is_empty()
                && match rule.verdict {
                    Verdict::Allow => tool.paths.iter().all(fits),
                    Verdict::Reject => tool.paths.iter().any(fits),
                };
            if !ok {
                return false;
            }
        }
        true
    }
}

impl Approver for Policy {
    fn describe(&self) -> Result<ApproverInfo, ProviderError> {
        let mut info = ApproverInfo::new("Rules", Authority::Policy, true);
        info.help =
            Some("Answers the requests your rules match, and leaves the others.".to_owned());
        info.wants_detail = self
            .rules
            .iter()
            .any(|each| each.rule.command.is_some() || each.rule.path.is_some());
        Ok(info)
    }

    fn ask(&self, request: Request) -> Result<Answer, ProviderError> {
        for each in &self.rules {
            // A rule never allows what the host says is protected: the request goes on to the person.
            if request.protected && each.rule.verdict == Verdict::Allow {
                continue;
            }
            if each.matches(&request, self.home.as_deref()) {
                return Ok(match each.rule.verdict {
                    Verdict::Allow => Answer::allow(),
                    Verdict::Reject => Answer::reject(),
                });
            }
        }
        Ok(Answer::abstain("no rule matches"))
    }

    fn wait(&self, _: Ticket) -> Result<Answer, ProviderError> {
        Ok(Answer::abstain("a rule answers at once"))
    }

    fn withdraw(&self, _: Ticket) -> Result<(), ProviderError> {
        Ok(())
    }
}

coqui_acp_api::guatiao::local_providers!(
    id = "canoa-approvers",
    version = env!("CARGO_PKG_VERSION"),
    providers = [Policy]
);

/// A registry holding the host's own approvers.
pub(super) fn registry() -> Result<coqui_acp_api::guatiao::library::Registry, String> {
    let mut registry =
        coqui_acp_api::guatiao::library::Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    let loading = registry
        .register_local("canoa-approvers", library)
        .map_err(|error| format!("canoa-approvers: {error}"))?;
    if loading.loaded().is_none() {
        return Err("canoa-approvers was not accepted".to_owned());
    }
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use canoa::approver::{Request, SessionRef, Step, Tool};

    use super::*;

    fn policy(rules: Vec<Rule>) -> Policy {
        Policy::from(PolicySettings { rules })
    }

    fn rule(verdict: Verdict) -> Rule {
        Rule {
            kind: None,
            name: None,
            command: None,
            path: None,
            verdict,
        }
    }

    fn request(kind: &str, name: &str, command: Option<&str>, paths: &[&str]) -> Request {
        let mut tool = Tool::new(kind, name, "title");
        tool.command = command.map(str::to_owned);
        tool.paths = paths.iter().map(|path| (*path).to_owned()).collect();
        Request::new("r", SessionRef::new("s", "a"), tool)
    }

    fn step(policy: &Policy, request: Request) -> (Step, Option<canoa::approver::Choice>) {
        let answer = policy.ask(request).expect("answers");
        (answer.step, answer.choice)
    }

    #[test]
    fn the_first_rule_a_request_matches_answers_and_no_match_abstains() {
        let read = Rule {
            kind: Some("READ".into()),
            ..rule(Verdict::Allow)
        };
        let status = Rule {
            command: Some("git status*".into()),
            ..rule(Verdict::Allow)
        };
        let all = Rule {
            name: Some("Bash".into()),
            ..rule(Verdict::Reject)
        };
        let set = policy(vec![read, status, all]);
        let allow = (Step::Answered, Some(canoa::approver::Choice::Allow));
        assert_eq!(step(&set, request("read", "Read", None, &[])), allow);
        assert_eq!(
            step(&set, request("execute", "Bash", Some("git status -s"), &[])),
            allow
        );
        assert_eq!(
            step(&set, request("execute", "Bash", Some("rm -rf x"), &[])),
            (Step::Answered, Some(canoa::approver::Choice::Reject))
        );
        assert_eq!(
            step(&set, request("edit", "Edit", None, &[])),
            (Step::Abstain, None)
        );
    }

    #[test]
    fn a_rule_that_reads_what_the_request_was_not_given_does_not_match() {
        let set = policy(vec![
            Rule {
                command: Some("*".into()),
                ..rule(Verdict::Allow)
            },
            Rule {
                path: Some("*.rs".into()),
                ..rule(Verdict::Allow)
            },
        ]);
        assert_eq!(
            step(&set, request("execute", "Bash", None, &[])).0,
            Step::Abstain
        );
    }

    #[test]
    fn an_allow_by_path_needs_every_file_to_match_and_a_reject_needs_one() {
        let allow = policy(vec![Rule {
            path: Some("*.rs".into()),
            ..rule(Verdict::Allow)
        }]);
        assert_eq!(
            step(
                &allow,
                request("edit", "Edit", None, &["/a/b.rs", "/a/c.rs"])
            )
            .0,
            Step::Answered
        );
        assert_eq!(
            step(
                &allow,
                request("edit", "Edit", None, &["/a/b.rs", "/a/c.txt"])
            )
            .0,
            Step::Abstain
        );
        let reject = policy(vec![Rule {
            path: Some("*.pem".into()),
            ..rule(Verdict::Reject)
        }]);
        assert_eq!(
            step(
                &reject,
                request("read", "Read", None, &["/a/b.rs", "/k/x.pem"])
            )
            .1,
            Some(canoa::approver::Choice::Reject)
        );
    }

    #[test]
    fn a_policy_does_not_allow_a_request_the_host_calls_protected_and_still_rejects_it() {
        let set = policy(vec![
            Rule {
                kind: Some("edit".into()),
                ..rule(Verdict::Allow)
            },
            Rule {
                name: Some("Edit".into()),
                ..rule(Verdict::Reject)
            },
        ]);
        let mut protected = request("edit", "Edit", None, &[]);
        protected.protected = true;
        // The allow is skipped; the reject after it answers.
        assert_eq!(
            step(&set, protected.clone()).1,
            Some(canoa::approver::Choice::Reject)
        );
        let only_allow = policy(vec![Rule {
            kind: Some("edit".into()),
            ..rule(Verdict::Allow)
        }]);
        assert_eq!(step(&only_allow, protected), (Step::Abstain, None));
    }

    #[test]
    fn a_policy_says_when_its_rules_read_the_command_or_the_files() {
        assert!(
            !policy(vec![rule(Verdict::Allow)])
                .describe()
                .expect("d")
                .wants_detail
        );
        let with_command = policy(vec![Rule {
            command: Some("ls".into()),
            ..rule(Verdict::Allow)
        }]);
        assert!(with_command.describe().expect("d").wants_detail);
    }
}
