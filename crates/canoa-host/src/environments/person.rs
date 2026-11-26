//! The person's own rules for their agent (what its settings refuse or ask about: files and commands), applied to the calls the agent
//! makes to the tools of an environment.
//!
//! An agent applies a rule to the tool the rule names, so a rule on its file or shell tool never matches the MCP tools through
//! which a session reaches an environment, and where those run unasked (an allow rule for the server, a mode that asks about
//! nothing) the person's refusal would not reach the environment. The agent's hook sees the calls of every tool, so the host
//! judges the environment's calls by the same rules, before the agent's own mode and rules, and answers a refusal or a question
//! in the agent's terms. A rule the host cannot place is read as strictly as it can be: a path that cannot be judged against a
//! deny rule is refused.

use canoa::store::protect::Protected;
use canoa::vendor::{PersonRule, RuleAccess, RuleDecision, ToolHook};
use serde_json::Value;

use super::mcp::SERVER_PREFIX;
use super::places::{self, Rules};

/// What the person's rules make of a call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Judged {
    /// A deny rule names it, or a deny rule cannot be judged against it.
    Deny(String),
    /// An ask rule names it.
    Ask(String),
}

/// The server and the tool an agent's name for an MCP tool means, when the server is one of the host's environments'.
fn environment_call<'a>(hook: &ToolHook, name: &'a str) -> Option<(&'a str, &'a str)> {
    let rest = name.strip_prefix(hook.mcp_prefix.as_deref()?)?;
    let (server, tool) = rest.rsplit_once(hook.mcp_separator.as_deref()?)?;
    server.starts_with(SERVER_PREFIX).then_some((server, tool))
}

/// What a call of `tool` with `input` reads, writes or runs: each as a kind of access and the path or the command.
fn targets(tool: &str, input: &serde_json::Map<String, Value>) -> Vec<(RuleAccess, String)> {
    let text = |key: &str| input.get(key).and_then(Value::as_str).map(str::to_owned);
    match tool {
        "fs_read" | "fs_list" => text("path")
            .map(|path| (RuleAccess::Read, path))
            .into_iter()
            .collect(),
        "fs_write" | "edit" => text("path")
            .map(|path| (RuleAccess::Write, path))
            .into_iter()
            .collect(),
        "apply_patch" => text("patch")
            .and_then(|patch| {
                super::patch::parse(&patch).ok().map(|patch| {
                    patch
                        .paths()
                        .into_iter()
                        .map(|path| (RuleAccess::Write, path.to_owned()))
                        .collect()
                })
            })
            .unwrap_or_default(),
        "exec" => {
            let program = text("program").unwrap_or_default();
            let args: Vec<String> = input
                .get("args")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|arg| arg.as_str().map(str::to_owned))
                .collect();
            command_texts(&program, &args)
                .into_iter()
                .map(|command| (RuleAccess::Run, command))
                .collect()
        }
        "terminal_send" => text("text")
            .map(|text| (RuleAccess::Run, text))
            .into_iter()
            .collect(),
        "terminal_open" => text("shell")
            .map(|shell| (RuleAccess::Run, shell))
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

/// The shells whose `-c` argument is a script of commands.
const SHELLS: [&str; 7] = ["sh", "bash", "zsh", "dash", "pwsh", "powershell", "cmd"];
/// The arguments of a shell that hold its script.
const SCRIPT_FLAGS: [&str; 4] = ["-c", "-command", "/c", "-commandwithargs"];

/// The commands a program run with `args` is: its own line, and, for a shell given a script, the script's.
fn command_texts(program: &str, args: &[String]) -> Vec<String> {
    let mut line = program.to_owned();
    for arg in args {
        line.push(' ');
        line.push_str(arg);
    }
    let mut all = vec![line];
    let name = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    if SHELLS.contains(&name)
        && let Some(at) = args
            .iter()
            .position(|arg| SCRIPT_FLAGS.contains(&arg.to_lowercase().as_str()))
    {
        all.push(args[at + 1..].join(" "));
    }
    all
}

/// `command` and each command a shell line makes of it (`;`, `&&`, `||`, `|`, `&` and a new line end one), trimmed.
fn candidates(command: &str) -> Vec<String> {
    let mut all = vec![command.trim().to_owned()];
    let mut current = String::new();
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        if matches!(c, ';' | '|' | '&' | '\n' | '\r') {
            if matches!(c, '|' | '&') && chars.peek() == Some(&c) {
                chars.next();
            }
            all.push(current.trim().to_owned());
            current.clear();
        } else {
            current.push(c);
        }
    }
    all.push(current.trim().to_owned());
    all.retain(|each| !each.is_empty());
    all
}

/// Whether `text` is the whole of what `pattern` says, `*` standing for any text.
fn glob(pattern: &[char], text: &[char]) -> bool {
    match pattern.split_first() {
        None => text.is_empty(),
        Some(('*', rest)) => (0..=text.len()).any(|skip| glob(rest, &text[skip..])),
        Some((c, rest)) => text.first() == Some(c) && glob(rest, &text[1..]),
    }
}

/// Whether one of `rules` (the commands they name) names `command` or one of the commands it holds.
fn names_command(rules: &[&PersonRule], command: &str) -> bool {
    let candidates = candidates(command);
    rules.iter().any(|rule| match &rule.pattern {
        None => true,
        Some(pattern) => {
            let pattern: Vec<char> = pattern.chars().collect();
            candidates
                .iter()
                .any(|each| glob(&pattern, &each.chars().collect::<Vec<_>>()))
        }
    })
}

/// Whether `rules` (the paths they name) name `path` as an environment with `paths` places it; why it cannot be said, when it
/// cannot.
fn names_path(rules: &[&PersonRule], paths: &Rules, path: &str) -> Result<bool, String> {
    if rules.is_empty() {
        return Ok(false);
    }
    if rules.iter().any(|rule| rule.pattern.is_none()) {
        return Ok(true);
    }
    let list = Protected::new(rules.iter().filter_map(|rule| rule.pattern.as_deref()));
    places::protected(&list, paths, path)
}

/// What the person's rules make of the agent's call `name` with `payload`'s input, when it is a call to a tool of an environment
/// (`servers` give each environment server's path rules); `None` when it is not, or no rule names it. The rules are read by `load`
/// only when the call is one.
pub(crate) fn judge(
    hook: &ToolHook,
    servers: &[(String, Rules)],
    load: &dyn Fn() -> Vec<PersonRule>,
    (name, input): (&str, &serde_json::Map<String, Value>),
) -> Option<Judged> {
    let (server, tool) = environment_call(hook, name)?;
    let targets = targets(tool, input);
    if targets.is_empty() {
        return None;
    }
    let rules = load();
    let paths = servers
        .iter()
        .find(|(each, _)| each == server)
        .map(|(_, rules)| rules.clone())
        .unwrap_or_default();
    for decision in [RuleDecision::Deny, RuleDecision::Ask] {
        for (access, target) in &targets {
            let of_kind: Vec<&PersonRule> = rules
                .iter()
                .filter(|rule| rule.decision == decision && rule.access == *access)
                .collect();
            let named = match access {
                RuleAccess::Run => Ok(names_command(&of_kind, target)),
                _ => names_path(&of_kind, &paths, target),
            };
            let what = match access {
                RuleAccess::Read => "reads",
                RuleAccess::Write => "changes",
                RuleAccess::Run => "runs",
                _ => "does",
            };
            match (decision, named) {
                (RuleDecision::Deny, Ok(true)) => {
                    return Some(Judged::Deny(format!(
                        "The person's own rules for this agent refuse a call that {what} {target}: it is refused in the environment too."
                    )));
                }
                (RuleDecision::Deny, Err(why)) => {
                    return Some(Judged::Deny(format!(
                        "The person's own rules for this agent refuse some places, and {why}"
                    )));
                }
                (_, Ok(true) | Err(_)) => {
                    return Some(Judged::Ask(format!(
                        "The person's own rules for this agent ask about a call that {what} {target}."
                    )));
                }
                (_, Ok(false)) => {}
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hook() -> ToolHook {
        let mut hook = ToolHook::new(
            Vec::new(),
            ("/tool_name", "/tool_input"),
            (String::new(), "/r"),
            (String::new(), "/i"),
        );
        hook.mcp_prefix = Some("mcp__".to_owned());
        hook.mcp_separator = Some("__".to_owned());
        hook
    }

    fn rule(decision: RuleDecision, access: RuleAccess, pattern: Option<&str>) -> PersonRule {
        PersonRule::new(decision, access, pattern.map(str::to_owned))
    }

    fn judged(rules: &[PersonRule], name: &str, input: &Value) -> Option<Judged> {
        let servers = vec![("env_box".to_owned(), Rules::default())];
        judge(
            &hook(),
            &servers,
            &|| rules.to_vec(),
            (name, input.as_object().expect("an object")),
        )
    }

    #[test]
    fn a_deny_rule_for_reading_a_path_refuses_the_environments_read_of_it_and_a_write_is_not_a_read() {
        let rules = [rule(RuleDecision::Deny, RuleAccess::Read, Some(".env"))];
        let read = json!({"path": "/work/app/.env"});
        assert!(matches!(
            judged(&rules, "mcp__env_box__fs_read", &read),
            Some(Judged::Deny(_))
        ));
        assert!(matches!(
            judged(&rules, "mcp__env_box__fs_list", &read),
            Some(Judged::Deny(_))
        ));
        assert_eq!(judged(&rules, "mcp__env_box__fs_write", &read), None);
        assert_eq!(
            judged(&rules, "mcp__env_box__fs_read", &json!({"path": "/work/app/main.rs"})),
            None
        );
    }

    #[test]
    fn a_rule_names_a_tool_only_when_it_is_an_environments_and_it_acts() {
        let rules = [rule(RuleDecision::Deny, RuleAccess::Read, None)];
        let read = json!({"path": "/x"});
        // Not an environment's server, not an MCP name, and a tool with nothing to read.
        assert_eq!(judged(&rules, "mcp__other__fs_read", &read), None);
        assert_eq!(judged(&rules, "Read", &read), None);
        assert_eq!(judged(&rules, "mcp__env_box__terminal_read", &json!({"id": "1"})), None);
        assert!(judged(&rules, "mcp__env_box__fs_read", &read).is_some());
    }

    #[test]
    fn a_deny_beats_an_ask_and_an_ask_alone_asks() {
        let both = [
            rule(RuleDecision::Ask, RuleAccess::Write, Some("src")),
            rule(RuleDecision::Deny, RuleAccess::Write, Some("src/secret.rs")),
        ];
        let secret = json!({"path": "/w/src/secret.rs", "content": "x"});
        assert!(matches!(
            judged(&both, "mcp__env_box__fs_write", &secret),
            Some(Judged::Deny(_))
        ));
        let plain = json!({"path": "/w/src/lib.rs", "content": "x"});
        assert!(matches!(
            judged(&both, "mcp__env_box__fs_write", &plain),
            Some(Judged::Ask(_))
        ));
        let edit = json!({"path": "/w/src/lib.rs", "old_string": "a", "new_string": "b"});
        assert!(matches!(
            judged(&both, "mcp__env_box__edit", &edit),
            Some(Judged::Ask(_))
        ));
    }

    #[test]
    fn a_patch_is_judged_by_every_file_it_writes() {
        let rules = [rule(RuleDecision::Deny, RuleAccess::Write, Some(".env"))];
        let patch = "*** Begin Patch\n*** Add File: /w/ok.txt\n+a\n*** Add File: /w/.env\n+b\n*** End Patch";
        assert!(matches!(
            judged(&rules, "mcp__env_box__apply_patch", &json!({"patch": patch})),
            Some(Judged::Deny(_))
        ));
        let fine = "*** Begin Patch\n*** Add File: /w/ok.txt\n+a\n*** End Patch";
        assert_eq!(
            judged(&rules, "mcp__env_box__apply_patch", &json!({"patch": fine})),
            None
        );
    }

    #[test]
    fn a_command_rule_names_the_program_a_shell_script_and_each_command_of_a_line() {
        let rules = [
            rule(RuleDecision::Deny, RuleAccess::Run, Some("rm")),
            rule(RuleDecision::Deny, RuleAccess::Run, Some("rm *")),
            rule(RuleDecision::Ask, RuleAccess::Run, Some("git push *")),
        ];
        let denied = |input: Value| {
            matches!(
                judged(&rules, "mcp__env_box__exec", &input),
                Some(Judged::Deny(_))
            )
        };
        assert!(denied(json!({"program": "rm", "args": ["-rf", "x"]})));
        assert!(denied(json!({"program": "bash", "args": ["-c", "ls && rm -rf x"]})));
        assert!(denied(json!({"program": "/bin/sh", "args": ["-c", "echo a; rm b"]})));
        assert!(!denied(json!({"program": "rmdir", "args": ["x"]})));
        assert!(!denied(json!({"program": "ls", "args": ["rm"]})));
        assert!(matches!(
            judged(&rules, "mcp__env_box__exec", &json!({"program": "git", "args": ["push", "origin"]})),
            Some(Judged::Ask(_))
        ));
        assert!(matches!(
            judged(&rules, "mcp__env_box__terminal_send", &json!({"id": "1", "text": "rm x\n"})),
            Some(Judged::Deny(_))
        ));
    }

    #[test]
    fn a_path_a_deny_rule_cannot_place_is_refused_and_an_ask_rule_asks() {
        let anchored = |decision| [rule(decision, RuleAccess::Read, Some("/etc"))];
        let relative = json!({"path": "etc/passwd"});
        assert!(matches!(
            judged(&anchored(RuleDecision::Deny), "mcp__env_box__fs_read", &relative),
            Some(Judged::Deny(why)) if why.contains("give the full path")
        ));
        assert!(matches!(
            judged(&anchored(RuleDecision::Ask), "mcp__env_box__fs_read", &relative),
            Some(Judged::Ask(_))
        ));
        assert!(matches!(
            judged(&anchored(RuleDecision::Deny), "mcp__env_box__fs_read", &json!({"path": "/etc/passwd"})),
            Some(Judged::Deny(_))
        ));
    }

    #[test]
    fn the_rules_are_read_only_for_a_call_to_an_environment() {
        let loaded = std::cell::Cell::new(0);
        let load = || {
            loaded.set(loaded.get() + 1);
            Vec::new()
        };
        let input = json!({"command": "ls"});
        let object = input.as_object().expect("an object");
        let servers: Vec<(String, Rules)> = Vec::new();
        assert_eq!(judge(&hook(), &servers, &load, ("Bash", object)), None);
        assert_eq!(loaded.get(), 0);
        let path = json!({"path": "/x"});
        let object = path.as_object().expect("an object");
        assert_eq!(judge(&hook(), &servers, &load, ("mcp__env_a__fs_read", object)), None);
        assert_eq!(loaded.get(), 1);
    }
}
