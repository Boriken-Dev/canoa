//! `CanoaUI.exe`, the portable launcher. See the crate's README for what it does and refuses.

#![cfg_attr(windows, windows_subsystem = "windows")]

use canoa_launch::{APP_PROGRAM, Error, Payload, install, prune};
use std::path::PathBuf;
use std::process::{Command, ExitCode};

/// The app folder the build packed, empty when no payload was named at build time.
static PAYLOAD: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/payload.zip"));
/// Its SHA-256, fixed by the build script.
const PAYLOAD_SHA256: &str = include_str!(concat!(env!("OUT_DIR"), "/payload.sha256"));

/// `%LOCALAPPDATA%\Canoa`; elsewhere the user's data folder, so the same code runs in tests on any platform.
fn base_dir() -> Result<PathBuf, Error> {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let root = if cfg!(windows) {
        var("LOCALAPPDATA")
    } else {
        var("XDG_DATA_HOME").or_else(|| var("HOME").map(|h| h.join(".local").join("share")))
    };
    let root =
        root.ok_or_else(|| Error::Refused("no per-user data folder (LOCALAPPDATA) is set".into()))?;
    if !root.is_absolute() {
        return Err(Error::Refused(format!(
            "{} is not an absolute path",
            root.display()
        )));
    }
    Ok(root.join("Canoa"))
}

/// CANOA_CONFIG is a list of files (a leading `+` layers them over the installed, system and user ones), split at `;` on
/// Windows and `:` elsewhere. Each entry is made absolute, as the person gave it otherwise: the app runs in another folder.
fn absolute_list(list: &std::ffi::OsStr) -> Result<std::ffi::OsString, Error> {
    let Some(text) = list.to_str() else {
        return Ok(list.to_owned());
    };
    let separator = if cfg!(windows) { ';' } else { ':' };
    let (plus, rest) = text
        .strip_prefix('+')
        .map_or(("", text), |rest| ("+", rest));
    let mut out = Vec::new();
    for entry in rest.split(separator) {
        if entry.is_empty() || !PathBuf::from(entry).is_relative() {
            out.push(entry.to_owned());
            continue;
        }
        let absolute = std::path::absolute(entry)
            .map_err(|e| Error::Refused(format!("CANOA_CONFIG {entry}: {e}")))?;
        out.push(absolute.to_string_lossy().into_owned());
    }
    Ok(format!("{plus}{}", out.join(&separator.to_string())).into())
}

fn run() -> Result<i32, Error> {
    let base = base_dir()?;
    let installed = install(
        &base,
        &Payload {
            bytes: PAYLOAD,
            sha256: PAYLOAD_SHA256,
        },
    )?;
    let _ = prune(&base, &installed.manifest.version);
    let program = installed.program(APP_PROGRAM)?;
    // The app runs in its own folder, not wherever it was started from, which a download may have filled. It reads no
    // file by a path relative to its working folder except the one CANOA_CONFIG names, kept as the person gave it.
    let mut command = Command::new(&program);
    command
        .args(std::env::args_os().skip(1))
        .current_dir(&installed.dir);
    // The app runs from an unpacked copy, so "beside the program" for its installed canoa.cfg is this launcher's folder.
    if let Some(folder) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
    {
        command.env("CANOA_INSTALLED_DIR", folder);
    }
    if let Some(list) = std::env::var_os("CANOA_CONFIG") {
        command.env("CANOA_CONFIG", absolute_list(&list)?);
    }
    let status = command
        .status()
        .map_err(|e| Error::Refused(format!("start {}: {e}", program.display())))?;
    Ok(status.code().unwrap_or(1))
}

/// Says why the launch failed: on standard error, and in a message box when there is a desktop, because this program has
/// no console to show it in.
fn report(error: &Error) {
    eprintln!("canoa: {error}");
    #[cfg(windows)]
    if std::env::var_os("CANOA_LAUNCH_NO_DIALOG").is_none() {
        let _ =
            win_msgbox::error::<win_msgbox::Okay>(&format!("Canoa could not start.\n\n{error}"))
                .title("Canoa")
                .show();
    }
}

fn main() -> ExitCode {
    match run() {
        // The app's own status, whole: a crash code is wider than a byte.
        Ok(code) => std::process::exit(code),
        Err(error) => {
            report(&error);
            ExitCode::FAILURE
        }
    }
}
