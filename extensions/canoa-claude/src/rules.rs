//! The rules the person gave Claude Code that refuse or ask about files and commands: its `permissions.deny` and `permissions.ask`
//! lists, read from every settings layer Claude Code reads (the machine's managed settings, the person's own folder, and the
//! project's two files), in the host's terms.
//!
//! Claude Code applies a rule to the tool it names, so a rule on `Read` or `Bash` never matches the MCP tools through which a
//! session reaches an environment; the host applies what is read here to those.

use std::path::{Path, PathBuf};

use canoa::vendor::{PersonRule, RuleAccess, RuleDecision, RulesRequest};
use serde_json::Value;

/// Claude Code's tools that read files, by name.
const READ_TOOLS: [&str; 1] = ["Read"];
/// Its tools that write or edit files: a rule on one of them is read as a rule on the others, as Claude Code applies `Edit`.
const WRITE_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];
/// Its tools that run commands.
const RUN_TOOLS: [&str; 2] = ["Bash", "PowerShell"];

/// The managed settings file of the operating system this runs on.
fn managed_file() -> Option<PathBuf> {
    match std::env::consts::OS {
        "windows" => Some(PathBuf::from(
            r"C:\Program Files\ClaudeCode\managed-settings.json",
        )),
        "macos" => Some(PathBuf::from(
            "/Library/Application Support/ClaudeCode/managed-settings.json",
        )),
        "linux" => Some(PathBuf::from("/etc/claude-code/managed-settings.json")),
        _ => None,
    }
}

/// The settings files of a session, from the machine's down to the project's own.
fn layers(request: &RulesRequest, managed: Option<PathBuf>) -> Vec<PathBuf> {
    let mut files = Vec::new();
    files.extend(managed);
    if let Some(folder) = &request.folder {
        files.push(Path::new(folder).join("settings.json"));
    }
    let project = Path::new(&request.cwd).join(".claude");
    files.push(project.join("settings.json"));
    files.push(project.join("settings.local.json"));
    files
}

/// Every deny and ask rule of `request`'s layers. A file that is missing or is not a JSON object says nothing; a rule that is not
/// about a file or a command is left to Claude Code, which applies it itself.
pub(crate) fn person_rules(request: &RulesRequest) -> Vec<PersonRule> {
    rules_in(request, managed_file())
}

/// [`person_rules`] with the machine's managed settings at `managed`.
fn rules_in(request: &RulesRequest, managed: Option<PathBuf>) -> Vec<PersonRule> {
    let mut rules = Vec::new();
    for file in layers(request, managed) {
        let Some(settings) = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        else {
            continue;
        };
        rules.extend(rules_of(&settings));
    }
    rules
}

/// The deny and ask rules one settings document holds.
pub(crate) fn rules_of(settings: &Value) -> Vec<PersonRule> {
    let mut rules = Vec::new();
    for (key, decision) in [("deny", RuleDecision::Deny), ("ask", RuleDecision::Ask)] {
        let listed = settings
            .pointer(&format!("/permissions/{key}"))
            .and_then(Value::as_array);
        for each in listed.into_iter().flatten().filter_map(Value::as_str) {
            rules.extend(rule(decision, each));
        }
    }
    rules
}

/// `Tool` or `Tool(specifier)`.
fn split(rule: &str) -> (&str, Option<&str>) {
    let rule = rule.trim();
    match rule.split_once('(') {
        Some((tool, rest)) => (
            tool.trim(),
            Some(rest.strip_suffix(')').unwrap_or(rest).trim()),
        ),
        None => (rule, None),
    }
}

fn rule(decision: RuleDecision, text: &str) -> Vec<PersonRule> {
    let (tool, specifier) = split(text);
    if READ_TOOLS.contains(&tool) {
        return vec![PersonRule::new(
            decision,
            RuleAccess::Read,
            specifier.and_then(path_line),
        )];
    }
    if WRITE_TOOLS.contains(&tool) {
        return vec![PersonRule::new(
            decision,
            RuleAccess::Write,
            specifier.and_then(path_line),
        )];
    }
    if RUN_TOOLS.contains(&tool) {
        return commands(specifier)
            .into_iter()
            .map(|pattern| PersonRule::new(decision, RuleAccess::Run, pattern))
            .collect();
    }
    Vec::new()
}

/// A path specifier as a line of the list of protected places, or `None` for every path.
///
/// Claude Code reads `//path` as an absolute path, `~/path` in the person's home, `/path` below the project's folder, and
/// `path` or `./path` below the working folder. A session's files are in an environment whose folders are not this computer's,
/// so a path below the project or the working folder is a name anywhere, which refuses at least what the rule does.
pub(crate) fn path_line(specifier: &str) -> Option<String> {
    let mut line = specifier.trim().replace('\\', "/");
    if let Some(rest) = line.strip_prefix("//") {
        line = format!("/{rest}");
    } else if line.starts_with("~/") {
        // Kept: the person's home.
    } else if let Some(rest) = line.strip_prefix("./") {
        line = rest.to_owned();
    } else if let Some(rest) = line.strip_prefix('/') {
        line = rest.to_owned();
    }
    for tail in ["/**", "/*"] {
        if let Some(rest) = line.strip_suffix(tail) {
            line = rest.to_owned();
        }
    }
    match line.as_str() {
        "" | "*" | "**" | "." | "/" => None,
        _ => Some(line),
    }
}

/// A command specifier as the whole commands it names: `rm:*` is `rm` and `rm *`, a specifier that is a command is that command,
/// and `*` stands for any text. No specifier, or `*`, is every command.
pub(crate) fn commands(specifier: Option<&str>) -> Vec<Option<String>> {
    let Some(specifier) = specifier.map(str::trim) else {
        return vec![None];
    };
    if specifier.is_empty() || specifier == "*" {
        return vec![None];
    }
    match specifier.strip_suffix(":*") {
        Some(prefix) => vec![
            Some(prefix.trim().to_owned()),
            Some(format!("{} *", prefix.trim())),
        ],
        None => vec![Some(specifier.to_owned())],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn of(settings: &Value) -> Vec<(RuleDecision, RuleAccess, Option<String>)> {
        rules_of(settings)
            .into_iter()
            .map(|rule| (rule.decision, rule.access, rule.pattern))
            .collect()
    }

    #[test]
    fn read_edit_write_and_command_rules_become_the_hosts_rules_and_the_rest_is_left_to_claude_code()
     {
        let found = of(&json!({"permissions": {
            "deny": ["Read(./.env)", "Edit(~/.ssh/**)", "Write(//etc/**)", "Bash(rm:*)", "WebFetch(domain:example.com)", "mcp__x"],
            "ask": ["Read", "Bash(git push *)", "PowerShell(Remove-Item)"],
            "allow": ["Bash"],
        }}));
        let denied = RuleDecision::Deny;
        let asked = RuleDecision::Ask;
        assert_eq!(
            found,
            vec![
                (denied, RuleAccess::Read, Some(".env".to_owned())),
                (denied, RuleAccess::Write, Some("~/.ssh".to_owned())),
                (denied, RuleAccess::Write, Some("/etc".to_owned())),
                (denied, RuleAccess::Run, Some("rm".to_owned())),
                (denied, RuleAccess::Run, Some("rm *".to_owned())),
                (asked, RuleAccess::Read, None),
                (asked, RuleAccess::Run, Some("git push *".to_owned())),
                (asked, RuleAccess::Run, Some("Remove-Item".to_owned())),
            ]
        );
    }

    #[test]
    fn the_tool_alone_and_a_star_name_every_file_or_command() {
        assert_eq!(path_line("**"), None);
        assert_eq!(path_line("*"), None);
        assert_eq!(path_line("/src/**").as_deref(), Some("src"));
        assert_eq!(path_line("**/secrets").as_deref(), Some("**/secrets"));
        assert_eq!(path_line(r"~\keys").as_deref(), Some("~/keys"));
        assert_eq!(commands(None), vec![None]);
        assert_eq!(commands(Some("*")), vec![None]);
        assert_eq!(
            commands(Some("npm run build")),
            vec![Some("npm run build".to_owned())]
        );
    }

    #[test]
    fn every_layer_adds_its_rules_and_one_that_cannot_be_read_adds_none() {
        let dir = std::env::temp_dir().join(format!("canoa-claude-rules-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (home, project) = (dir.join("home"), dir.join("project"));
        std::fs::create_dir_all(project.join(".claude")).expect("folders");
        std::fs::create_dir_all(&home).expect("home");
        std::fs::write(
            home.join("settings.json"),
            r#"{"permissions":{"deny":["Read(./.env)"]}}"#,
        )
        .expect("user settings");
        std::fs::write(
            project.join(".claude").join("settings.json"),
            r#"{"permissions":{"ask":["Bash(git push:*)"]}}"#,
        )
        .expect("project settings");
        std::fs::write(
            project.join(".claude").join("settings.local.json"),
            "not json",
        )
        .expect("local settings");
        let found = rules_in(
            &RulesRequest::new(
                Some(home.to_string_lossy().into_owned()),
                project.to_string_lossy().into_owned(),
            ),
            Some(dir.join("managed-settings.json")),
        );
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(found.len(), 3, "{found:?}");
        assert!(found.iter().any(|rule| rule.decision == RuleDecision::Deny
            && rule.access == RuleAccess::Read
            && rule.pattern.as_deref() == Some(".env")));
        assert!(found.iter().any(|rule| rule.decision == RuleDecision::Ask
            && rule.pattern.as_deref() == Some("git push *")));
    }
}
