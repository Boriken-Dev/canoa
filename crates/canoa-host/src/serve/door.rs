//! The door: one pipe or Unix socket per home, owner-only, that the browser's native messaging host relays to. A connection says
//! in its first message which entry it opens, and from there it is the ordinary link: the same proof, trust store and scope as
//! on an entry's address. The door admits nobody by itself.

use alloc::sync::Arc;
use core::sync::atomic::Ordering;
use core::time::Duration;
use std::path::{Path, PathBuf};
use std::thread;

use coqui_link::auth;
use coqui_transport::Channel as _;
use serde_json::{Value, json};

use super::{Entered, Served, Shared, connection};
use crate::native::{self, MAX_PREAMBLE};
use crate::pipe;

/// How long a connection has to say what it opens.
const PREAMBLE_WAIT: Duration = Duration::from_secs(5);
/// How often the accept loop looks to see whether the host is stopping.
const STOP_POLL: Duration = Duration::from_millis(100);

/// What `@control` and `@tools` run.
pub(super) type Doors = (Arc<Served>, Arc<Served>);

/// What the door serves.
pub(super) struct Door {
    listener: pipe::Listener,
    /// What `@hello` answers: a claim, with no proof, to a process of this user.
    hello: Value,
    served: Vec<Arc<Served>>,
    /// `@control` and `@tools`.
    control: Doors,
    home: PathBuf,
}

impl Door {
    pub(super) fn new(
        listener: pipe::Listener,
        hello: Value,
        served: Vec<Arc<Served>>,
        control: Doors,
        home: PathBuf,
    ) -> Self {
        Self {
            listener,
            hello,
            served,
            control,
            home,
        }
    }

    /// Takes connections until the host stops.
    pub(super) fn run(&self, shared: &Arc<Shared>) {
        while !shared.stopped.load(Ordering::SeqCst) {
            let stream = match self.listener.accept_timeout(Some(STOP_POLL)) {
                Ok(stream) => stream,
                Err(pipe::Error::TimedOut) => continue,
                Err(error) => {
                    tracing::warn!("a connection to the native door failed: {error}");
                    thread::sleep(STOP_POLL);
                    continue;
                }
            };
            let Some(entered) = shared.unproven.enter() else {
                tracing::warn!(
                    "a connection to the native door was dropped: too many are unproven"
                );
                continue;
            };
            let channel = pipe::PipeChannel::new(stream, MAX_PREAMBLE);
            let shared = Arc::clone(shared);
            let hello = self.hello.clone();
            let served = self.served.clone();
            let control = self.control.clone();
            let home = self.home.clone();
            let spawned = thread::Builder::new()
                .name("canoa-door".to_owned())
                .spawn(move || {
                    meet(
                        &shared,
                        (&hello, &served, &control),
                        &home,
                        &channel,
                        entered,
                    );
                    channel.close();
                });
            if let Err(error) = spawned {
                tracing::warn!("a connection to the native door was dropped: {error}");
            }
        }
    }
}

fn reply(channel: &pipe::PipeChannel, value: &Value) -> bool {
    channel.send(value.to_string().as_bytes()).is_ok()
}

fn meet(
    shared: &Arc<Shared>,
    (hello, served, (control, tools)): (&Value, &[Arc<Served>], &Doors),
    home: &Path,
    channel: &pipe::PipeChannel,
    entered: Entered,
) {
    let Ok(Some(first)) = channel.receive(Some(PREAMBLE_WAIT)) else {
        return;
    };
    let Some(wanted) = native::read_preamble(&first) else {
        reply(channel, &json!({ "error": "not a native door request" }));
        return;
    };
    let target = match wanted.open.as_str() {
        native::HELLO => {
            reply(channel, hello);
            return;
        }
        crate::control::NAME => control,
        native::TOOLS => tools,
        name => match served.iter().find(|served| served.name == name) {
            Some(served) => served,
            None => {
                reply(channel, &json!({ "error": "no such entry" }));
                return;
            }
        },
    };
    if !reply(channel, &json!({ "opened": true })) {
        return;
    }
    channel.set_max_message(auth::MAX_PROOF_MESSAGE);
    tracing::debug!(
        entry = target.name,
        origin = wanted.origin,
        "a browser reached the door"
    );
    // A key bound to the extension that paired it is not served to another origin.
    let origin = wanted.origin;
    // What offers tools to the host's sessions is the browser's own key, the one the extension's PIN pairing bound to its
    // origin: a program paired another way cannot pose as the browser there.
    let browser_only = wanted.open == native::TOOLS;
    let admit = |peer: &coqui_peer_webcrypto::PeerId| {
        let peer = peer.to_string();
        if browser_only {
            native::bound_to(home, &peer, &origin)
        } else {
            native::origin_allows(home, &peer, &origin)
        }
    };
    // Only a key the extension's PIN pairing bound to this origin speaks for the extension: what it says of a page's session
    // (which Office tools its pane holds) is taken from it and from no other key.
    let extension =
        |peer: &coqui_peer_webcrypto::PeerId| native::bound_to(home, &peer.to_string(), &origin);
    connection(
        shared,
        target,
        channel,
        entered,
        Some(&admit),
        Some(&extension),
    );
}

/// The origin of the host's own web server as `hello` claims it, for a page on this computer that the extension serves: `http` or
/// `https` by whether the server has a certificate, and the loopback address in place of an unspecified one. `None` when the
/// server takes its port from the system (the address is then not known before it binds).
pub(super) fn web_origin(web: &crate::config::WebConfig) -> Option<String> {
    use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
    if web.listen.port() == 0 {
        return None;
    }
    let ip = match web.listen.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    let scheme = if web.tls.is_some() { "https" } else { "http" };
    Some(format!(
        "{scheme}://{}",
        SocketAddr::new(ip, web.listen.port())
    ))
}

#[cfg(test)]
mod tests {
    use super::web_origin;
    use crate::config::{WebConfig, WebTls};

    fn web(listen: &str, tls: bool) -> WebConfig {
        let mut web = WebConfig::default();
        web.listen = listen.parse().unwrap();
        web.tls = tls.then(|| WebTls::new("web.crt", "web.key"));
        web
    }

    #[test]
    fn the_web_origin_names_the_scheme_the_server_speaks_and_a_loopback_for_an_unspecified_address()
    {
        assert_eq!(
            web_origin(&web("127.0.0.1:7800", false)).as_deref(),
            Some("http://127.0.0.1:7800")
        );
        assert_eq!(
            web_origin(&web("0.0.0.0:7800", true)).as_deref(),
            Some("https://127.0.0.1:7800")
        );
        assert_eq!(
            web_origin(&web("[::]:7800", true)).as_deref(),
            Some("https://[::1]:7800")
        );
    }

    #[test]
    fn a_port_the_system_picks_is_no_address_to_claim() {
        assert_eq!(web_origin(&web("127.0.0.1:0", false)), None);
    }
}
