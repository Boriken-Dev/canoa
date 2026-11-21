//! `canoa office-launcher` command and login registration tests.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a command line names what it uses from std"
)]

use super::args::{ConfigArgs, Format, LauncherArgs};
use super::commands::{load, print};
use super::{Failed, Front, home_of};

/// What `office-launcher` is asked to do about signing in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoginAction {
    Install,
    Uninstall,
    Status,
}

/// The add-in's address from canoa.cfg, which the launcher holds.
fn office_web(args: &ConfigArgs, front: &dyn Front) -> Result<crate::config::WebConfig, Failed> {
    let config = load(args, front)?;
    config
        .web
        .clone()
        .filter(|web| web.office && web.office_local)
        .ok_or_else(|| {
            Failed::usage(
                "canoa.cfg has no [host.web] with office = true and office_local = true: there is no add-in address to hold",
            )
        })
}

/// `canoa office-launcher`.
pub(super) fn office_launcher(args: &LauncherArgs, front: &dyn Front) -> Result<(), Failed> {
    let action = if args.install {
        Some(LoginAction::Install)
    } else if args.uninstall {
        Some(LoginAction::Uninstall)
    } else if args.status {
        Some(LoginAction::Status)
    } else {
        None
    };
    if let Some(action) = action {
        let place = crate::launcher::login::Place::current().map_err(Failed::run)?;
        return office_login(action, &args.config, front, &place);
    }
    if args.detach {
        return office_detach(&args.config, front);
    }
    let args = &args.config;
    let web = office_web(args, front)?;
    let home = home_of(front)?;
    let program = front
        .program()
        .map_err(|error| Failed::run(format!("this program cannot find itself: {error}")))?;
    let launch = crate::launcher::launch_of(&home, &program, args.config.as_deref());
    crate::launcher::run(
        &web,
        &home.dir().join("office"),
        &|| crate::launcher::host_runs(&home),
        &|| launch.start(&home),
        &|line| front.say(line),
        &|| false,
        &crate::launcher::Pace::default(),
    )
    .map_err(Failed::run)
}

/// `canoa office-launcher --detach`: the launcher, started again with no console, and this process ends.
fn office_detach(args: &ConfigArgs, front: &dyn Front) -> Result<(), Failed> {
    // The launcher checks this itself, but its answer would be in a log nobody is reading when it is started at sign-in.
    office_web(args, front)?;
    let home = home_of(front)?;
    let command = login_command(args, front, false)?;
    let log = home.dir().join("office").join("launcher.log");
    let id = crate::launcher::start_hidden(&command, &log).map_err(Failed::run)?;
    print(
        front,
        &if args.format == Format::Json {
            serde_json::json!({"pid": id, "log": log.display().to_string()}).to_string()
        } else {
            format!(
                "the launcher runs with no window (process {id}); it writes to {}",
                log.display()
            )
        },
    )
}

/// What the sign-in entry runs: this program, its leading arguments and `office-launcher`, with the `--config` it was given
/// and, with `detach`, the flag that starts it with no console.
fn login_command(
    args: &ConfigArgs,
    front: &dyn Front,
    detach: bool,
) -> Result<Vec<String>, Failed> {
    let program = front
        .program()
        .map_err(|error| Failed::run(format!("this program cannot find itself: {error}")))?;
    let leading: Vec<String> = program
        .args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    Ok(crate::launcher::login::command_of(
        &program.path,
        &leading,
        args.config.as_deref(),
        detach,
    ))
}

/// `canoa office-launcher --install|--uninstall|--status`, against `place`.
fn office_login(
    action: LoginAction,
    args: &ConfigArgs,
    front: &dyn Front,
    place: &crate::launcher::login::Place,
) -> Result<(), Failed> {
    match action {
        LoginAction::Uninstall => {
            let line = if place.remove().map_err(Failed::run)? {
                format!("removed {}", place.location())
            } else {
                format!("nothing registered at {}", place.location())
            };
            print(front, &line)
        }
        LoginAction::Install => {
            // The check the launcher makes when it starts: an entry that fails at every sign-in helps nobody.
            office_web(args, front)?;
            let command = login_command(args, front, place.needs_detach())?;
            place.install(&command).map_err(Failed::run)?;
            print(
                front,
                &format!(
                    "registered {}: {} (it starts at the next sign-in)",
                    place.location(),
                    place.text_of(&command)
                ),
            )
        }
        LoginAction::Status => {
            let address = office_web(args, front).ok().map(|web| web.listen);
            let home = home_of(front)?;
            let status = crate::launcher::login::status(
                place,
                &login_command(args, front, place.needs_detach())?,
                address,
                crate::launcher::host_runs(&home),
            )
            .map_err(Failed::run)?;
            print(front, &status.describe(args.format == Format::Json))
        }
    }
}

#[cfg(test)]
mod office_login_tests {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use clap::Parser as _;

    use super::super::Program;
    use super::super::args::{Cli, ConfigArgs, Format};
    use super::*;
    use crate::launcher::login::Place;

    /// A front with a home and a program of its own, keeping what is printed.
    struct Said {
        home: PathBuf,
        lines: Mutex<Vec<String>>,
    }

    impl Front for Said {
        fn print(&self, line: &str) -> std::io::Result<()> {
            self.lines
                .lock()
                .expect("not poisoned")
                .push(line.to_owned());
            Ok(())
        }

        fn say(&self, _line: &str) {}

        fn when_input_closes(
            &self,
            _status: u8,
            _then: Box<dyn FnOnce() + Send>,
        ) -> std::io::Result<()> {
            Ok(())
        }

        fn program(&self) -> std::io::Result<Program> {
            Ok(Program::new(
                PathBuf::from("/opt/canoa app/canoa"),
                Vec::new(),
            ))
        }

        fn home(&self) -> Option<PathBuf> {
            Some(self.home.clone())
        }

        fn holder(&self) -> &str {
            "a test"
        }

        fn ask(&self, _prompt: &str, _secret: bool) -> std::io::Result<Option<String>> {
            Ok(None)
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("canoa-login-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a folder");
        dir
    }

    fn config_with(dir: &Path, address: &str, web: bool) -> ConfigArgs {
        let file = dir.join("canoa.cfg");
        let text = if web {
            format!("[host.web]\nlisten = \"{address}\"\noffice = true\noffice_local = true\n")
        } else {
            "[host]\n".to_owned()
        };
        std::fs::write(&file, text).expect("a config");
        ConfigArgs {
            config: Some(file.display().to_string()),
            format: Format::Text,
        }
    }

    #[test]
    fn install_registers_one_entry_naming_the_config_status_says_both_facts_and_uninstall_removes_it()
     {
        let dir = scratch("flow");
        let held = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
        let address = held.local_addr().expect("its address").to_string();
        let args = config_with(&dir, &address, true);
        let front = Said {
            home: dir.join("home"),
            lines: Mutex::new(Vec::new()),
        };
        let place = Place::Autostart(dir.join("autostart"));
        let said = |front: &Said| {
            front
                .lines
                .lock()
                .expect("not poisoned")
                .pop()
                .unwrap_or_default()
        };

        office_login(LoginAction::Install, &args, &front, &place).expect("installed");
        office_login(LoginAction::Install, &args, &front, &place).expect("installed again");
        let entries: Vec<_> = std::fs::read_dir(dir.join("autostart"))
            .expect("a folder")
            .collect();
        assert_eq!(entries.len(), 1, "one entry however often it is installed");
        let written = std::fs::read_to_string(place.location()).expect("the entry");
        assert!(written.contains("office-launcher"), "{written}");
        assert!(
            written.contains("--config") && written.contains("canoa.cfg"),
            "{written}"
        );

        office_login(LoginAction::Status, &args, &front, &place).expect("status");
        let status = said(&front);
        assert!(status.contains("registered: yes"), "{status}");
        assert!(
            status.contains(&format!("address: {address} answers")),
            "{status}"
        );
        assert!(status.contains("a launcher answers: yes"), "{status}");

        office_login(LoginAction::Uninstall, &args, &front, &place).expect("removed");
        assert!(!Path::new(&place.location()).exists());
        office_login(LoginAction::Status, &args, &front, &place).expect("status");
        assert!(said(&front).contains("registered: no"));
        office_login(LoginAction::Uninstall, &args, &front, &place).expect("again");
        assert!(said(&front).contains("nothing registered"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_refuses_a_config_with_no_address_to_hold_and_writes_nothing() {
        let dir = scratch("refuse");
        let args = config_with(&dir, "", false);
        let front = Said {
            home: dir.join("home"),
            lines: Mutex::new(Vec::new()),
        };
        let place = Place::Autostart(dir.join("autostart"));
        let failed =
            office_login(LoginAction::Install, &args, &front, &place).expect_err("refused");
        assert_eq!(failed.status, 2);
        assert!(!dir.join("autostart").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_install_flags_cannot_be_given_together() {
        for both in [
            ["--install", "--uninstall"],
            ["--install", "--status"],
            ["--uninstall", "--status"],
            ["--detach", "--install"],
            ["--detach", "--uninstall"],
            ["--detach", "--status"],
        ] {
            assert!(
                Cli::try_parse_from(["canoa", "office-launcher", both[0], both[1]]).is_err(),
                "{both:?}"
            );
        }
        assert!(Cli::try_parse_from(["canoa", "office-launcher", "--install"]).is_ok());
        assert!(Cli::try_parse_from(["canoa", "office-launcher", "--detach"]).is_ok());
    }

    #[test]
    fn after_help_lists_every_environment_variable_read_at_startup() {
        use clap::CommandFactory as _;
        let help = super::super::Cli::command().render_help().to_string();
        for name in [
            "CANOA_CONFIG",
            "CANOA_INSTALLED_DIR",
            "CANOA_LOG",
            "COQUI_AGENTIC_PEER_HOME",
            "NO_COLOR",
        ] {
            assert!(
                help.contains(name),
                "missing {name} in --help:
{help}"
            );
        }
    }
}
