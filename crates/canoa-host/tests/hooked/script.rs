//! A hook script for this platform made from what it does at each event: it logs the event and its payload, then does
//! the one thing its arm says. A PowerShell script on Windows and a shell script elsewhere, since those are the
//! interpreters every machine has.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    dead_code,
    reason = "test support names what it uses from std, and not every test uses all of it"
)]

use std::path::Path;

/// What a hook does for an event, after logging it.
pub enum Arm {
    /// Writes this text to standard output.
    Print(String),
    /// Exits with this status.
    Exit(i32),
    /// Waits this many seconds.
    Sleep(u64),
    /// Writes this many bytes.
    Big(usize),
    /// Leaves a process that outlives the hook and holds its standard output (Unix only).
    Daemon,
    /// Logs the value of this environment variable, as the line `envprobe=<value>`.
    LogEnv(String),
}

/// The default hook script's name on this platform.
pub const fn file_name() -> &'static str {
    if cfg!(windows) { "hook.ps1" } else { "hook.sh" }
}

/// The script: every event is logged to `log` as `<event> <payload>`, and `arms` say what else is done.
pub fn body(log: &Path, arms: &[(&str, Arm)]) -> String {
    let log = log.to_string_lossy();
    if cfg!(windows) {
        let mut out = format!(
            "$event = $args[0]\n$payload = [Console]::In.ReadToEnd()\n\
             Add-Content -LiteralPath '{}' -Value \"$event $payload\"\nswitch ($event) {{\n",
            log.replace('\'', "''")
        );
        for (event, arm) in arms {
            let action = match arm {
                Arm::Print(text) => format!("[Console]::Out.Write('{}')", text.replace('\'', "''")),
                Arm::Exit(status) => format!("exit {status}"),
                Arm::Sleep(seconds) => format!("Start-Sleep -Seconds {seconds}"),
                Arm::Big(bytes) => format!("[Console]::Out.Write(('x' * {bytes}))"),
                Arm::Daemon => "exit 0".to_owned(),
                Arm::LogEnv(name) => {
                    format!(
                        "Add-Content -LiteralPath '{}' -Value \"envprobe=$env:{name}\"",
                        log.replace('\'', "''")
                    )
                }
            };
            out.push_str(&format!("  '{event}' {{ {action} }}\n"));
        }
        // A switch needs at least one clause, and an event no arm names does nothing.
        out.push_str("  default { }\n}\n");
        out
    } else {
        let mut out = format!(
            "event=\"$1\"\npayload=$(cat)\nprintf '%s %s\\n' \"$event\" \"$payload\" >> '{}'\ncase \"$event\" in\n",
            log.replace('\'', "'\\''")
        );
        for (event, arm) in arms {
            let action = match arm {
                Arm::Print(text) => format!("printf '%s' '{}'", text.replace('\'', "'\\''")),
                Arm::Exit(status) => format!("exit {status}"),
                Arm::Sleep(seconds) => format!("sleep {seconds}"),
                Arm::Big(bytes) => format!("head -c {bytes} /dev/zero | tr '\\0' x"),
                Arm::Daemon => "setsid sleep 15 &".to_owned(),
                Arm::LogEnv(name) => format!(
                    "printf 'envprobe=%s\\n' \"${name}\" >> '{}'",
                    log.replace('\'', "'\\''")
                ),
            };
            out.push_str(&format!("  '{event}') {action} ;;\n"));
        }
        out.push_str("esac\n");
        out
    }
}

/// The events and payloads a script logged, in order.
pub fn logged(log: &Path) -> Vec<(String, serde_json::Value)> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (event, payload) = line.trim_start_matches('\u{feff}').split_once(' ')?;
            Some((event.to_owned(), serde_json::from_str(payload).ok()?))
        })
        .collect()
}
