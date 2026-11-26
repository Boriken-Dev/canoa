//! The `command` route: a program of the person's that listens as a proxy, started when the route opens and stopped when it ends.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "sockets and threads are std's"
)]

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use canoa::network_route::{
    Checked, LeaseStatus, NetworkRoute, OpenRequest, Reach, RouteInfo, RouteLease, Sharing,
};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Provider, Schema, ToValue};
use guatiao_intake::Form;

use super::shared::Lease;

/// How long the program has to start listening when the settings say nothing.
const LISTEN_SECONDS: u64 = 10;

/// What the `command` route is set with.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, Default)]
#[map(deny_unknown_fields)]
#[form(section(id = "program", label = "Program"))]
pub struct CommandSettings {
    /// The program and its arguments, one per line; `{listen}` stands for the `host:port` it must listen on, which is on loopback.
    #[map(default)]
    #[schema(section = "program", title = "Command")]
    #[form(placeholder = "my-proxy --listen {listen}")]
    pub command: Vec<String>,
    /// The hosts that go direct, in `NO_PROXY` form.
    #[schema(section = "program", title = "Hosts that go direct")]
    pub no_proxy: Option<Vec<String>>,
    /// Whether sessions whose settings are equal share one running program (default: each session has its own).
    #[schema(section = "program", title = "Share one program between sessions")]
    pub shared: Option<bool>,
    /// How many seconds the program has to start listening (default 10).
    #[schema(advanced, section = "program", title = "Start within")]
    pub start_seconds: Option<u64>,
}

/// A program that is the proxy.
#[derive(Provider)]
#[provider(NetworkRoute, id = "command", config = CommandSettings, form = CommandSettings)]
pub struct CommandRoute {
    settings: CommandSettings,
}

impl From<CommandSettings> for CommandRoute {
    fn from(settings: CommandSettings) -> Self {
        Self { settings }
    }
}

/// The program, killed when the lease ends.
struct Running(std::sync::Mutex<Option<crate::spawn::Child>>);

impl Drop for Running {
    fn drop(&mut self) {
        let held = self
            .0
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(mut child) = held.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl CommandRoute {
    fn problems(&self) -> Vec<String> {
        match self.settings.command.first() {
            Some(program) if !program.trim().is_empty() => Vec::new(),
            _ => vec!["command names no program".to_owned()],
        }
    }
}

impl NetworkRoute for CommandRoute {
    fn describe(&self) -> Result<RouteInfo, ProviderError> {
        let mut info = RouteInfo::new("Program");
        info.description = Some(
            "A program of yours that listens as a proxy: it is started when the route is used and stopped after."
                .to_owned(),
        );
        info.sharing = if self.settings.shared == Some(true) {
            Sharing::Shared
        } else {
            Sharing::PerSession
        };
        Ok(info)
    }

    fn check(&self, _request: OpenRequest) -> Result<Checked, ProviderError> {
        Ok(Checked::of(self.problems()))
    }

    fn open(&self, _request: OpenRequest) -> Result<Object<dyn RouteLease>, ProviderError> {
        if let Some(problem) = self.problems().first() {
            return Err(canoa::status::bad_value(problem));
        }
        let port = TcpListener::bind("127.0.0.1:0")
            .and_then(|probe| probe.local_addr())
            .map_err(|error| canoa::status::internal(format!("no port to listen on: {error}")))?
            .port();
        let listen = format!("127.0.0.1:{port}");
        let words: Vec<String> = self
            .settings
            .command
            .iter()
            .map(|word| word.replace("{listen}", &listen))
            .collect();
        let Some((program, args)) = words.split_first() else {
            return Err(canoa::status::bad_value("command names no program"));
        };
        let mut command = crate::spawn::Command::new(program);
        command
            .args(args)
            .stdin(crate::spawn::Stdio::Null)
            .stdout(crate::spawn::Stdio::Null)
            .stderr(crate::spawn::Stdio::Null)
            .own_group();
        let child = command.spawn().map_err(|error| {
            canoa::status::internal(format!("{program} could not start: {error}"))
        })?;
        let running = Running(std::sync::Mutex::new(Some(child)));
        let target: SocketAddr = listen
            .parse()
            .map_err(|_| canoa::status::internal("no address to listen on"))?;
        let wait = self.settings.start_seconds.unwrap_or(LISTEN_SECONDS);
        let started = Instant::now();
        while TcpStream::connect_timeout(&target, Duration::from_millis(200)).is_err() {
            if started.elapsed() > Duration::from_secs(wait) {
                return Err(canoa::status::internal(format!(
                    "{program} did not listen on {listen} within {wait} seconds"
                )));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let url = format!("http://{listen}");
        let no_proxy = self.settings.no_proxy.clone();
        let mut status = LeaseStatus::new(format!("through {program}, listening on {listen}"));
        status.address = Some(listen);
        Ok(Lease::new(
            move || {
                let mut reach = Reach::proxy(url.clone());
                reach.no_proxy.clone_from(&no_proxy);
                reach
            },
            status,
            running,
        )
        .into_object())
    }
}
