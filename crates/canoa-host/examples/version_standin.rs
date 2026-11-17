//! A program the store tests make every version of: a copy of this file with a trailer appended. The trailer is one line,
//! `COQUI-STANDIN:` and a JSON object `{"line": "...", "exit": 0, "auto_update_var": "NAME"}`, which says what it prints for
//! any argument, how it exits, and which variable turns its updater off. Without that variable set it writes `auto-updated`
//! beside itself, so a test can see that a launch kept the updater off. It also writes `ran` beside itself when given
//! `setup`, to stand for an installer's extra step.
//!
//! Given `-o<folder>` it stands for a self-extracting archive: the trailer's `extract` is a list of `{"path", "line", "exit"}`,
//! and each is written under the folder as a copy of this program with a trailer of its own (the path's folders are made).
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    missing_docs,
    reason = "a test program"
)]

use std::fs;
use std::process::ExitCode;

const MARK: &[u8] = b"COQUI-STANDIN:";

fn trailer() -> Option<serde_json::Value> {
    let bytes = fs::read(std::env::current_exe().ok()?).ok()?;
    let at = bytes.windows(MARK.len()).rposition(|w| w == MARK)?;
    let rest = &bytes[at + MARK.len()..];
    let end = rest.iter().position(|b| *b == b'\n').unwrap_or(rest.len());
    serde_json::from_slice(&rest[..end]).ok()
}

/// Writes each program the trailer's `extract` lists under `folder`.
fn extract(
    exe: &std::path::Path,
    config: &serde_json::Value,
    folder: &std::path::Path,
) -> ExitCode {
    let Ok(bytes) = fs::read(exe) else {
        return ExitCode::from(4);
    };
    let at = bytes
        .windows(MARK.len())
        .rposition(|w| w == MARK)
        .map_or(bytes.len(), |at| at.saturating_sub(1));
    for each in config["extract"].as_array().into_iter().flatten() {
        let Some(path) = each["path"].as_str() else {
            return ExitCode::from(5);
        };
        let target = folder.join(path);
        if let Some(parent) = target.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let mut program = bytes[..at].to_vec();
        let child = serde_json::json!({"line": each["line"], "exit": each["exit"], "auto_update_var": null});
        program.extend_from_slice(b"\nCOQUI-STANDIN:");
        program.extend_from_slice(child.to_string().as_bytes());
        program.push(b'\n');
        if fs::write(&target, program).is_err() {
            return ExitCode::from(6);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = fs::set_permissions(&target, fs::Permissions::from_mode(0o755));
        }
    }
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let Some(config) = trailer() else {
        eprintln!("no trailer");
        return ExitCode::from(3);
    };
    let exe = std::env::current_exe().expect("own path");
    let beside = |name: &str| exe.with_file_name(name);
    if let Some(var) = config["auto_update_var"].as_str()
        && std::env::var_os(var).is_none()
    {
        let _ = fs::write(beside("auto-updated"), b"1");
    }
    if let Some(folder) = std::env::args().find_map(|arg| arg.strip_prefix("-o").map(str::to_owned))
    {
        return extract(&exe, &config, std::path::Path::new(&folder));
    }
    if std::env::args().nth(1).as_deref() == Some("setup") {
        let _ = fs::write(beside("ran"), b"1");
        return ExitCode::SUCCESS;
    }
    println!("{}", config["line"].as_str().unwrap_or(""));
    let code = config["exit"].as_u64().unwrap_or(0);
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
