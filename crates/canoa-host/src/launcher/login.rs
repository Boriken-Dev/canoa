//! Registering `canoa office-launcher` to run when the person signs in, and removing it again.
//!
//! One entry, in the place the operating system reads for the signed-in user and nowhere else: a value under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` on Windows, a property list in `~/Library/LaunchAgents` on macOS, a
//! desktop entry in `~/.config/autostart` elsewhere. Nothing needs elevation, and removing deletes that one entry by its name.

use crate::spawn::Command;
use canoa::brand::CONFIG_FILE;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The value's name under the Run key.
fn value() -> String {
    format!("{}OfficeLauncher", canoa::brand::NAME.replace(' ', ""))
}
/// The file name stem of the macOS and Linux entries, and the macOS job's label.
fn label() -> String {
    format!("{}.office-launcher", canoa::brand::ID)
}
/// The key Windows reads at sign-in, below `HKCU`.
#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Where one entry lives.
#[derive(Clone, Debug)]
#[allow(
    dead_code,
    reason = "each system builds its own place; the others' are built by tests"
)]
pub(crate) enum Place {
    /// A value under this key of `HKCU` (Windows).
    RunKey(String),
    /// A property list in this folder (macOS).
    LaunchAgents(PathBuf),
    /// A desktop entry in this folder (the other Unix systems).
    Autostart(PathBuf),
}

impl Place {
    /// The place this system reads for the signed-in user.
    ///
    /// # Errors
    ///
    /// When the user's folder cannot be found.
    pub(crate) fn current() -> Result<Self, String> {
        #[cfg(windows)]
        {
            Ok(Self::RunKey(RUN_KEY.to_owned()))
        }
        #[cfg(not(windows))]
        {
            let home = std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from);
            if cfg!(target_os = "macos") {
                let home = home.ok_or("no home folder to register in")?;
                Ok(Self::LaunchAgents(
                    home.join("Library").join("LaunchAgents"),
                ))
            } else {
                let config = std::env::var_os("XDG_CONFIG_HOME")
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
                    .or_else(|| home.map(|home| home.join(".config")))
                    .ok_or("no home folder to register in")?;
                Ok(Self::Autostart(config.join("autostart")))
            }
        }
    }

    /// The file this entry is, or the key and value it is.
    pub(crate) fn location(&self) -> String {
        match self {
            Self::RunKey(key) => format!(r"HKCU\{key}\{}", value()),
            Self::LaunchAgents(dir) => dir.join(format!("{}.plist", label())).display().to_string(),
            Self::Autostart(dir) => dir
                .join(format!("{}.desktop", label()))
                .display()
                .to_string(),
        }
    }

    /// Whether the entry's program gets a console window of its own at sign-in unless it ends in a moment: the Run value of
    /// Windows starts `canoa.exe`, a console program, whose window would stay as long as the launcher runs. The entry names
    /// `--detach` there, so the window is gone as soon as the launcher has started itself without one.
    pub(crate) const fn needs_detach(&self) -> bool {
        matches!(self, Self::RunKey(_))
    }

    /// What the entry holds for `command` (the program, then its arguments), as a person reads it: the Run value, the
    /// desktop entry's `Exec`, the property list's arguments joined by spaces.
    pub(crate) fn text_of(&self, command: &[String]) -> String {
        match self {
            Self::RunKey(_) => command
                .iter()
                .map(|part| quote_windows(part))
                .collect::<Vec<_>>()
                .join(" "),
            Self::LaunchAgents(_) => command.join(" "),
            Self::Autostart(_) => exec_line(command),
        }
    }

    /// Writes the entry, replacing the one that is there.
    ///
    /// # Errors
    ///
    /// Why the registry or the folder refused.
    pub(crate) fn install(&self, command: &[String]) -> Result<(), String> {
        match self {
            Self::RunKey(key) => reg(&[
                "add",
                &format!(r"HKCU\{key}"),
                "/v",
                &value(),
                "/t",
                "REG_SZ",
                "/d",
                &self.text_of(command),
                "/f",
            ])
            .map(drop),
            Self::LaunchAgents(dir) => {
                write_entry(dir, &format!("{}.plist", label()), &plist(command))
            }
            Self::Autostart(dir) => {
                write_entry(dir, &format!("{}.desktop", label()), &desktop(command))
            }
        }
    }

    /// Deletes the entry, which is not an error when it was not there. Returns whether it was.
    ///
    /// # Errors
    ///
    /// Why the registry or the file refused.
    pub(crate) fn remove(&self) -> Result<bool, String> {
        match self {
            Self::RunKey(key) => {
                if self.registered()?.is_none() {
                    return Ok(false);
                }
                reg(&["delete", &format!(r"HKCU\{key}"), "/v", &value(), "/f"]).map(|_| true)
            }
            Self::LaunchAgents(_) | Self::Autostart(_) => {
                let file = self.file().ok_or("no entry file")?;
                match std::fs::remove_file(&file) {
                    Ok(()) => Ok(true),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
                    Err(error) => Err(format!("{} could not be removed: {error}", file.display())),
                }
            }
        }
    }

    /// What the entry holds now, in the shape [`Place::text_of`] gives, or `None` when there is none.
    ///
    /// # Errors
    ///
    /// Why the registry or the file could not be read.
    pub(crate) fn registered(&self) -> Result<Option<String>, String> {
        match self {
            Self::RunKey(key) => {
                let Some(output) = reg_query(&format!(r"HKCU\{key}"))? else {
                    return Ok(None);
                };
                Ok(value_in(&output, &value()))
            }
            Self::LaunchAgents(_) | Self::Autostart(_) => {
                let file = self.file().ok_or("no entry file")?;
                let text = match std::fs::read_to_string(&file) {
                    Ok(text) => text,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                    Err(error) => {
                        return Err(format!("{} could not be read: {error}", file.display()));
                    }
                };
                Ok(Some(match self {
                    Self::LaunchAgents(_) => plist_arguments(&text).join(" "),
                    _ => text
                        .lines()
                        .find_map(|line| line.strip_prefix("Exec="))
                        .unwrap_or_default()
                        .to_owned(),
                }))
            }
        }
    }

    fn file(&self) -> Option<PathBuf> {
        match self {
            Self::RunKey(_) => None,
            Self::LaunchAgents(dir) => Some(dir.join(format!("{}.plist", label()))),
            Self::Autostart(dir) => Some(dir.join(format!("{}.desktop", label()))),
        }
    }
}

fn write_entry(dir: &Path, name: &str, text: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(dir.join(name), text))
        .map_err(|error| format!("{} could not be written: {error}", dir.join(name).display()))
}

/// Windows' own `reg`, found through `SystemRoot` as the rest of the host finds it.
fn reg(args: &[&str]) -> Result<String, String> {
    let output = run_reg(args)?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "reg {} failed: {}",
            args.first().copied().unwrap_or_default(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// The listing of a key, or `None` when the key is not there.
fn reg_query(key: &str) -> Result<Option<String>, String> {
    let output = run_reg(&["query", key])?;
    if output.status.success() {
        Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
    } else if output.status.code() == Some(1) {
        // `reg query` answers 1 for a key that is not there, in every language; its message is the only other word it has.
        Ok(None)
    } else {
        Err(format!(
            "reg query failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn run_reg(args: &[&str]) -> Result<std::process::Output, String> {
    if !cfg!(windows) {
        return Err("the registry is Windows' alone".to_owned());
    }
    let root = std::env::var("SystemRoot").map_err(|_| "no SystemRoot to find reg in")?;
    Command::new(format!(r"{root}\System32\reg.exe"))
        .args(args)
        .output()
        .map_err(|error| format!("reg could not be run: {error}"))
}

/// The data of the string value `name` in a `reg query` listing.
fn value_in(listing: &str, name: &str) -> Option<String> {
    listing.lines().find_map(|line| {
        let rest = line.trim_start().strip_prefix(name)?;
        let rest = rest.trim_start().strip_prefix("REG_SZ")?;
        Some(
            rest.strip_prefix("    ")
                .unwrap_or(rest.trim_start())
                .to_owned(),
        )
    })
}

/// One argument as `CommandLineToArgvW` reads it back.
fn quote_windows(argument: &str) -> String {
    if !argument.is_empty() && !argument.contains([' ', '\t', '"']) {
        return argument.to_owned();
    }
    let mut out = String::from('"');
    let mut slashes = 0_usize;
    for character in argument.chars() {
        match character {
            '\\' => slashes += 1,
            '"' => {
                out.extend(core::iter::repeat_n('\\', slashes * 2 + 1));
                out.push('"');
                slashes = 0;
            }
            other => {
                out.extend(core::iter::repeat_n('\\', slashes));
                out.push(other);
                slashes = 0;
            }
        }
    }
    out.extend(core::iter::repeat_n('\\', slashes * 2));
    out.push('"');
    out
}

/// The `Exec` value of a desktop entry: each argument quoted when it needs it, `%` doubled, and the whole line escaped as
/// a desktop file's string, whose backslash is its own escape.
fn exec_line(command: &[String]) -> String {
    let quoted: Vec<String> = command
        .iter()
        .map(|part| {
            let part = part.replace('%', "%%");
            let needs = part.is_empty()
                || part.contains([
                    ' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?',
                    '#', '(', ')', '`',
                ]);
            if needs {
                let mut out = String::from('"');
                for character in part.chars() {
                    if matches!(character, '"' | '`' | '$' | '\\') {
                        out.push('\\');
                    }
                    out.push(character);
                }
                out.push('"');
                out
            } else {
                part
            }
        })
        .collect();
    quoted.join(" ").replace('\\', "\\\\")
}

fn desktop(command: &[String]) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Canoa Office launcher\nComment=Offers to start Canoa when an Office task pane opens and Canoa is not running\nExec={}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        exec_line(command)
    )
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn plist(command: &[String]) -> String {
    let label = label();
    let arguments: String = command
        .iter()
        .map(|part| format!("    <string>{}</string>\n", xml_escape(part)))
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>Label</key>\n  <string>{label}</string>\n  <key>ProgramArguments</key>\n  <array>\n{arguments}  </array>\n  <key>RunAtLoad</key>\n  <true/>\n</dict>\n</plist>\n"
    )
}

/// The strings of a property list's `ProgramArguments`.
fn plist_arguments(text: &str) -> Vec<String> {
    let Some((_, after)) = text.split_once("<key>ProgramArguments</key>") else {
        return Vec::new();
    };
    let Some((array, _)) = after.split_once("</array>") else {
        return Vec::new();
    };
    array
        .split("<string>")
        .skip(1)
        .filter_map(|piece| piece.split_once("</string>"))
        .map(|(value, _)| {
            value
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&amp;", "&")
        })
        .collect()
}

/// `--config` with each file made absolute: the entry runs from a folder nobody chose, so a relative name would mean
/// another file there. A leading `+` and the list's separator are kept.
pub(crate) fn config_value(named: &str) -> String {
    let (plus, list) = named
        .strip_prefix('+')
        .map_or(("", named), |rest| ("+", rest));
    let separator = if cfg!(windows) { ';' } else { ':' };
    let made: Vec<String> = list
        .split(separator)
        .map(|file| {
            std::path::absolute(file).map_or_else(
                |_| file.to_owned(),
                |absolute| absolute.display().to_string(),
            )
        })
        .collect();
    format!("{plus}{}", made.join(&separator.to_string()))
}

/// What the entry runs: `program`, its leading arguments, `office-launcher`, for a named config `--config` and it, and with
/// `detach` the `--detach` that makes the launcher start itself again with no console and end.
pub(crate) fn command_of(
    program: &Path,
    leading: &[String],
    config: Option<&str>,
    detach: bool,
) -> Vec<String> {
    let mut command = vec![program.display().to_string()];
    command.extend(leading.iter().cloned());
    command.push("office-launcher".to_owned());
    if let Some(named) = config.filter(|value| !value.is_empty()) {
        command.push("--config".to_owned());
        command.push(config_value(named));
    }
    if detach {
        command.push("--detach".to_owned());
    }
    command
}

/// What `status` knows.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Status {
    /// Where the entry is, registered or not.
    pub(crate) location: String,
    /// What the entry holds, when there is one.
    pub(crate) holds: Option<String>,
    /// Whether it holds what `install` would write now.
    pub(crate) current: bool,
    /// The add-in's address, when canoa.cfg names one.
    pub(crate) address: Option<SocketAddr>,
    /// Whether something answers at it.
    pub(crate) answers: bool,
    /// Whether a host holds the home.
    pub(crate) host_runs: bool,
}

impl Status {
    /// Whether the launcher is what answers: something does, and no host holds the home, which is the only time the launcher
    /// holds the address.
    pub(crate) const fn launcher_answers(&self) -> bool {
        self.answers && !self.host_runs
    }

    /// The lines `status` prints, or the one JSON object.
    pub(crate) fn describe(&self, json: bool) -> String {
        let yes = |value: bool| if value { "yes" } else { "no" };
        if json {
            return serde_json::json!({
                "registered": self.holds.is_some(),
                "location": self.location,
                "holds": self.holds,
                "current": self.current,
                "address": self.address.map(|address| address.to_string()),
                "answers": self.answers,
                "host_runs": self.host_runs,
                "launcher_answers": self.launcher_answers(),
            })
            .to_string();
        }
        let mut lines = vec![format!(
            "registered: {} ({})",
            yes(self.holds.is_some()),
            self.location
        )];
        if let Some(holds) = &self.holds {
            lines.push(format!("runs: {holds}"));
            if !self.current {
                lines.push(
                    "this is not what --install would write now: run it again to replace it"
                        .to_owned(),
                );
            }
        }
        lines.push(match self.address {
            Some(address) => format!(
                "address: {address} {}",
                if self.answers {
                    "answers"
                } else {
                    "does not answer"
                }
            ),
            None => format!(
                "address: none ({CONFIG_FILE} has no [host.web] with office and office_local)"
            ),
        });
        lines.push(format!("a host holds the home: {}", yes(self.host_runs)));
        lines.push(format!(
            "a launcher answers: {}",
            yes(self.launcher_answers())
        ));
        lines.join("\n")
    }
}

/// Reads the entry and looks at the address.
///
/// # Errors
///
/// Why the entry could not be read.
pub(crate) fn status(
    place: &Place,
    command: &[String],
    address: Option<SocketAddr>,
    host_runs: bool,
) -> Result<Status, String> {
    let holds = place.registered()?;
    let current = holds.as_deref() == Some(place.text_of(command).as_str());
    Ok(Status {
        location: place.location(),
        holds,
        current,
        address,
        answers: address.is_some_and(answers),
        host_runs,
    })
}

/// Whether something accepts a connection at `address` (the loopback one, for an address that listens on every interface).
fn answers(address: SocketAddr) -> bool {
    let target = if address.ip().is_unspecified() {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), address.port())
    } else {
        address
    };
    TcpStream::connect_timeout(&target, Duration::from_secs(2)).is_ok()
}

#[cfg(test)]
#[path = "login_tests.rs"]
mod tests;
