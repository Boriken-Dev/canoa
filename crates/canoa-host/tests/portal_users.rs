//! `canoa portal` with two real accounts, the operating system's own PAM and NSS, and systemd: each person signs in, gets a host
//! of their own that runs as them, sees only their own settings and sessions, cannot reach the other's sockets or files; an idle
//! host stops and comes back; a wrong password is refused without the accounts being asked.
//!
//! It needs root's help once, before and after: `tests/portal_users/setup.sh TARGET_DIR` makes three throwaway accounts
//! (`canoatestportal`, `canoatesta`, `canoatestb`), a PAM service of its own over a test password file, the template unit and a
//! sudoers rule for the two test units; `teardown.sh` removes them. Then, as a user that may run `sudo -n -u canoatest...`:
//!
//! ```text
//! CANOA_TEST_PORTAL_USERS=1 cargo test -p canoa-host --features portal --test portal_users -- --ignored --test-threads=1
//! ```
//!
//! The tests share the accounts, their hosts and the portal's socket, so they run one at a time. The second runs the portal with
//! a sign-in policy: a password only from 10.0.0.0/8, a certificate (pinned through the portal) from anywhere.
#![cfg(unix)]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::io::{BufRead as _, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use coqui_peer_webcrypto::PeerId;
use serde_json::{Value, json};
use support::{key, with_link_headers};

const PORTAL_USER: &str = "canoatestportal";
const PEOPLE: [(&str, &str); 2] = [("canoatesta", "pw-a-test"), ("canoatestb", "pw-b-test")];

struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Answer {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(each, _)| each.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn cookie(&self) -> Option<String> {
        self.headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("set-cookie"))
            .and_then(|(_, line)| line.split(';').next())
            .map(str::to_owned)
    }
}

/// Where the proxy reaches the portal: the browser's address in `X-Forwarded-For` is believed there, and nowhere else.
const PROXY_SOCKET: &str = "/run/canoatest-portal/portal.sock";

fn call(
    addr: SocketAddr,
    method: &str,
    path: &str,
    extra: &[(&str, String)],
    body: &str,
) -> Answer {
    let stream = TcpStream::connect(addr).expect("the portal answers");
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .expect("a timeout");
    call_on(stream, &addr.to_string(), method, path, extra, body)
}

fn call_unix(method: &str, path: &str, extra: &[(&str, String)], body: &str) -> Answer {
    let stream =
        std::os::unix::net::UnixStream::connect(PROXY_SOCKET).expect("the proxy's socket answers");
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .expect("a timeout");
    call_on(stream, "localhost", method, path, extra, body)
}

fn call_on<S: Read + Write>(
    mut stream: S,
    host: &str,
    method: &str,
    path: &str,
    extra: &[(&str, String)],
    body: &str,
) -> Answer {
    let mut text = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for (name, value) in extra {
        text.push_str(&format!("{name}: {value}\r\n"));
    }
    if !body.is_empty() {
        text.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    text.push_str("\r\n");
    text.push_str(body);
    stream.write_all(text.as_bytes()).expect("a request");
    let mut raw = Vec::new();
    let _ = stream.read_to_end(&mut raw);
    let raw = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    Answer {
        status,
        headers: lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
            .collect(),
        body: body.to_owned(),
    }
}

fn origin(addr: SocketAddr) -> String {
    format!("http://{addr}")
}

/// A sign-in as the proxy makes it, the browser at `from`.
fn sign_in(user: &str, password: &str, from: &str) -> Answer {
    call_unix(
        "POST",
        "/portal/login",
        &[
            ("Origin", "http://localhost".to_owned()),
            (
                "Content-Type",
                "application/x-www-form-urlencoded".to_owned(),
            ),
            ("X-Forwarded-For", from.to_owned()),
        ],
        &format!("user={user}&password={password}"),
    )
}

fn sudo_as(user: &str, args: &[&str]) -> std::process::Output {
    Command::new("sudo")
        .args(["-n", "-u", user])
        .args(args)
        .output()
        .expect("sudo runs")
}

fn uid_of(user: &str) -> u32 {
    let out = Command::new("id")
        .args(["-u", user])
        .output()
        .expect("id runs");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .expect("a uid")
}

fn unit(person: &str) -> String {
    format!("canoatest@{person}.service")
}

fn systemctl(args: &[&str]) -> String {
    let out = Command::new("systemctl")
        .args(args)
        .output()
        .expect("systemctl runs");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn main_pid(person: &str) -> u32 {
    systemctl(&["show", &unit(person), "-p", "MainPID", "--value"])
        .parse()
        .unwrap_or(0)
}

fn active(person: &str) -> bool {
    systemctl(&["is-active", &unit(person)]) == "active"
}

/// The first user id on the `Uid:` line of a process's status: whose process it is.
fn process_uid(pid: u32) -> u32 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).expect("a running process");
    status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|uid| uid.parse().ok())
        .expect("a Uid line")
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(40);
    while Instant::now() < until {
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("timed out waiting for {what}");
}

fn pam_asked() -> usize {
    std::fs::read_to_string("/var/tmp/canoatest-pam-count")
        .map(|text| text.lines().count())
        .unwrap_or(0)
}

struct Portal {
    child: Child,
    addr: SocketAddr,
}

impl Drop for Portal {
    fn drop(&mut self) {
        // The portal runs as another user, so ending it is that user's to do.
        let _ = Command::new("sudo")
            .args(["-n", "kill", &self.child.id().to_string()])
            .status();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_portal() -> Portal {
    start_portal_with("/opt/canoatest/portal.toml")
}

fn start_portal_with(config: &str) -> Portal {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("a port")
        .local_addr()
        .expect("an address")
        .port();
    let listen = format!("127.0.0.1:{port}");
    let mut child = Command::new("sudo")
        .args(["-n", "-u", PORTAL_USER, "/opt/canoatest/canoa", "portal"])
        .args(["--config", config, "--listen", &listen])
        .env("CANOA_LOG", "info")
        .stdout(Stdio::piped())
        .spawn()
        .expect("the portal starts");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("piped"))
        .read_line(&mut line)
        .expect("the portal says where it listens");
    assert!(line.contains("portal"), "{line:?}");
    Portal {
        child,
        addr: listen.parse().expect("an address"),
    }
}

/// A person at a browser: a cookie, and a key of its own.
struct Browser {
    cookie: String,
    key: cabuya::auth::Identity,
    host: PeerId,
    addr: SocketAddr,
}

impl Browser {
    fn sign_in(addr: SocketAddr, person: &str, password: &str) -> Self {
        let answer = call(
            addr,
            "POST",
            "/portal/login",
            &[
                ("Origin", origin(addr)),
                (
                    "Content-Type",
                    "application/x-www-form-urlencoded".to_owned(),
                ),
            ],
            &format!("user={person}&password={password}"),
        );
        assert_eq!(answer.status, 303, "{}", answer.body);
        let cookie = answer.cookie().expect("a cookie");
        let index = call(
            addr,
            "GET",
            "/canoa.json",
            &[("Cookie", cookie.clone())],
            "",
        );
        assert_eq!(index.status, 200, "{}", index.body);
        let index: Value = serde_json::from_str(&index.body).expect("JSON");
        assert_eq!(index["user"], person);
        let host = index["peer"]
            .as_str()
            .expect("a peer")
            .parse()
            .expect("a peer id");
        Self {
            cookie,
            key: key(),
            host,
            addr,
        }
    }

    fn headers(&self) -> Vec<(String, String)> {
        vec![
            ("Cookie".to_owned(), self.cookie.clone()),
            ("Origin".to_owned(), origin(self.addr)),
        ]
    }

    fn url(&self, path: &str) -> String {
        format!("ws://{}{path}", self.addr)
    }

    /// Connects through the portal to `path`, proves the key and runs `body`.
    fn link<R>(
        &self,
        path: &str,
        body: impl FnOnce(&support::Rpc<'_, coqui_transport_web::websocket::Channel>) -> R,
    ) -> Result<R, String> {
        with_link_headers(&self.url(path), self.headers(), &self.key, self.host, body)
    }

    /// The browser's first connection is turned away and waits in the host; the portal then introduces it.
    fn pair(&self) {
        let first = self.link("/portal/ws/entry/fixture", |rpc| rpc.initialize());
        assert!(
            !matches!(first, Ok(Ok(_))),
            "an unpaired key is not served: {first:?}"
        );
        let answer = call(
            self.addr,
            "POST",
            "/portal/pair",
            &[
                ("Cookie", self.cookie.clone()),
                ("Origin", origin(self.addr)),
                ("Content-Type", "application/json".to_owned()),
            ],
            &json!({"peer": self.key.peer_id().to_string()}).to_string(),
        );
        assert_eq!(answer.status, 200, "{}", answer.body);
    }
}

#[test]
#[ignore = "needs CANOA_TEST_PORTAL_USERS and the accounts tests/portal_users/setup.sh makes"]
fn two_people_each_get_their_own_host_as_themselves_and_reach_nothing_of_the_others() {
    if std::env::var_os("CANOA_TEST_PORTAL_USERS").is_none() {
        panic!("CANOA_TEST_PORTAL_USERS is not set: run tests/portal_users/setup.sh first");
    }
    // A host a failed run left is stopped first, so each run starts from nothing.
    for (person, _) in PEOPLE {
        let _ = Command::new("sudo")
            .args(["-n", "systemctl", "stop", &unit(person)])
            .status();
    }
    // A person's home keeps what an earlier run wrote, so this run's records and settings are its own.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let run = format!("{}-{stamp}", std::process::id());
    let (record, accent) = (format!("session-a-{run}"), format!("teal-{run}"));
    let portal = start_portal();
    let addr = portal.addr;

    // A wrong password is refused, and past the limit nothing reaches the accounts, the right password included.
    let before = pam_asked();
    for _ in 0..3 {
        let refused = sign_in("canoatesta", "not-the-password", "203.0.113.50");
        assert_eq!(refused.status, 401);
        assert!(refused.cookie().is_none());
    }
    assert_eq!(pam_asked() - before, 3);
    let blocked = sign_in("canoatesta", "pw-a-test", "203.0.113.50");
    assert_eq!(blocked.status, 429);
    assert_eq!(pam_asked() - before, 3, "a blocked sign-in reached PAM");
    // A name that is no account is refused the same way, and an account below the lowest user id is not admitted at all.
    assert_eq!(
        sign_in("no-such-canoa-account", "x", "203.0.113.51").status,
        401
    );
    assert_eq!(sign_in("root", "x", "203.0.113.52").status, 401);
    // The block is for that address and that account alone: another browser behind the proxy is not held to it.
    assert_eq!(
        sign_in("canoatesta", "pw-a-test", "203.0.113.50").status,
        429,
        "still blocked"
    );
    assert_eq!(
        sign_in("canoatesta", "pw-a-test", "203.0.113.99").status,
        303,
        "another browser is not"
    );
    assert!(!active("canoatesta"), "a refused sign-in started a host");
    // An account outside the sign-in group has the right password, and is refused all the same: no host, no cookie.
    let outsider = sign_in("canoatestc", "pw-c-test", "203.0.113.53");
    assert_eq!(outsider.status, 401);
    assert!(outsider.cookie().is_none());
    assert!(!active("canoatestc"));

    // Each signs in and gets a host that is theirs.
    let a = Browser::sign_in(addr, PEOPLE[0].0, PEOPLE[0].1);
    let b = Browser::sign_in(addr, PEOPLE[1].0, PEOPLE[1].1);
    let (pid_a, pid_b) = (main_pid("canoatesta"), main_pid("canoatestb"));
    assert!(pid_a > 0 && pid_b > 0 && pid_a != pid_b);
    assert_eq!(process_uid(pid_a), uid_of("canoatesta"));
    assert_eq!(process_uid(pid_b), uid_of("canoatestb"));
    let cwd = |pid: u32| std::fs::read_link(format!("/proc/{pid}/cwd")).ok();
    // Their working directories are in homes this user cannot read, so the link itself may be hidden; the owner of the
    // sockets' folders says the same.
    let _ = cwd(pid_a);
    for person in ["canoatesta", "canoatestb"] {
        let owner = sudo_as(
            person,
            &["stat", "-c", "%U", &format!("/run/canoatest/{person}")],
        );
        assert_eq!(String::from_utf8_lossy(&owner.stdout).trim(), person);
    }
    assert_ne!(a.host, b.host, "two hosts, two identities");

    // Each pairs the browser's own key and runs a session; the agent is that person's process.
    a.pair();
    b.pair();
    for (browser, person) in [(&a, "canoatesta"), (&b, "canoatestb")] {
        let work = format!("/home/{person}");
        let (text, agent) = browser
            .link("/portal/ws/entry/fixture", |rpc| {
                rpc.initialize()?;
                let session = rpc.request("session/new", json!({"cwd": work, "mcpServers": []}))?;
                let text = rpc.prompt(session["sessionId"].as_str().expect("an id"), "hello")?;
                let pid = rpc.request("_fixture/pid", json!({}))?["pid"]
                    .as_u64()
                    .expect("a pid");
                Ok::<_, Value>((text, pid as u32))
            })
            .expect("a paired key is served through the portal")
            .expect("a turn");
        assert_eq!(text, "echo: hello");
        assert_eq!(
            process_uid(agent),
            uid_of(person),
            "the agent runs as {person}"
        );
    }

    // Settings and sessions are each person's own, kept in their own home.
    let a_settings = a
        .link("/portal/ws/control", |rpc| {
            rpc.request(
                "config/set",
                json!({"section": "appearance", "value": {"accent": accent}}),
            )?;
            rpc.request(
                "ledger/put",
                json!({"id": record, "record": {"title": "a's"}}),
            )?;
            Ok::<_, Value>((
                rpc.request("config/get", json!({"section": "appearance"}))?,
                rpc.request("ledger/list", json!({}))?,
            ))
        })
        .expect("a reaches control")
        .expect("control answers");
    assert_eq!(
        a_settings.0["value"]["accent"],
        accent.as_str(),
        "{a_settings:?}"
    );
    assert!(a_settings.1.to_string().contains(&record), "{a_settings:?}");
    let b_sees = b
        .link("/portal/ws/control", |rpc| {
            Ok::<_, Value>((
                rpc.request("config/get", json!({"section": "appearance"}))?,
                rpc.request("ledger/list", json!({}))?,
            ))
        })
        .expect("b reaches control")
        .expect("control answers");
    assert!(
        b_sees.0["value"].is_null(),
        "b reads a's settings: {b_sees:?}"
    );
    assert!(
        !b_sees.1.to_string().contains(&record),
        "b lists a's session: {b_sees:?}"
    );
    let own_file = |person: &str| {
        sudo_as(
            person,
            &[
                "grep",
                "-c",
                &accent,
                &format!("/home/{person}/.config/Canoa/canoa.cfg"),
            ],
        )
    };
    assert!(
        own_file("canoatesta").status.success(),
        "a's setting is in a's home"
    );
    assert!(
        !own_file("canoatestb").status.success(),
        "a's setting is not in b's home"
    );

    // The folders are as the guide says: no one lists /run/canoatest, and each person's is theirs, group the portal's, no one else's.
    let stat = |path: &str| {
        let out = Command::new("stat")
            .args(["-c", "%a %U %G", path])
            .output()
            .expect("stat runs");
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    assert_eq!(stat("/run/canoatest"), "711 root root");
    assert_eq!(
        stat("/run/canoatest/canoatesta"),
        "710 canoatesta canoatestportal"
    );
    assert_eq!(
        stat(PROXY_SOCKET).split(' ').next(),
        Some("660"),
        "the proxy's socket is for its group only"
    );
    assert!(
        !sudo_as("canoatesta", &["ls", "/run/canoatest"])
            .status
            .success(),
        "a person lists /run/canoatest"
    );
    assert!(
        !sudo_as("canoatestb", &["ls", "/run/canoatest/canoatesta"])
            .status
            .success()
    );
    // The units are bounded, in the slice that bounds them all.
    assert_eq!(
        systemctl(&["show", &unit("canoatesta"), "-p", "Slice", "--value"]),
        "canoatest.slice"
    );
    assert_eq!(
        systemctl(&["show", &unit("canoatesta"), "-p", "TasksMax", "--value"]),
        "512"
    );
    assert_ne!(
        systemctl(&["show", &unit("canoatesta"), "-p", "MemoryMax", "--value"]),
        "infinity"
    );
    // A host will not run as another account's name or in another's folder, nor as root.
    for (who, dir) in [
        ("canoatesta", "/run/canoatest/canoatestb"),
        ("canoatestb", "/run/canoatest/canoatesta"),
    ] {
        let out = sudo_as(
            who,
            &[
                "/opt/canoatest/canoa",
                "serve",
                "--allow-empty",
                "--portal",
                dir,
                "--portal-user",
                PORTAL_USER,
            ],
        );
        assert_eq!(out.status.code(), Some(2), "{who} ran a host in {dir}");
    }
    // Nor the host of an account outside the groups, by itself or through the helper: each layer refuses on its own.
    let out = sudo_as(
        "canoatestc",
        &[
            "/opt/canoatest/canoa",
            "serve",
            "--allow-empty",
            "--portal",
            "/run/canoatest/canoatestc",
            "--portal-user",
            PORTAL_USER,
            "--portal-group",
            "canoatestusers",
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = Command::new("sudo")
        .args([
            "-n",
            "-u",
            PORTAL_USER,
            "sudo",
            "-n",
            "/opt/canoatest/canoa-instance",
            "start",
            "canoatest@canoatestc.service",
        ])
        .output()
        .expect("sudo runs");
    assert_eq!(
        out.status.code(),
        Some(2),
        "the helper started a host for an account outside the group"
    );
    assert!(!active("canoatestc"));
    // The root helper takes only the units of real people.
    for unit in [
        "canoatest@root.service",
        "canoatest@1000.service",
        "canoatest@daemon.service",
        "canoatest@a@b.service",
    ] {
        let out = Command::new("sudo")
            .args([
                "-n",
                "-u",
                PORTAL_USER,
                "sudo",
                "-n",
                "/opt/canoatest/canoa-instance",
                "start",
                unit,
            ])
            .output()
            .expect("sudo runs");
        assert_eq!(out.status.code(), Some(2), "the helper started {unit}");
    }

    // Neither can reach the other's sockets or files.
    for (from, to) in [("canoatesta", "canoatestb"), ("canoatestb", "canoatesta")] {
        for socket in ["portal.sock", "control.sock", "entries/fixture.sock"] {
            let path = format!("/run/canoatest/{to}/{socket}");
            let out = sudo_as(
                from,
                &[
                    "python3",
                    &uds_script(),
                    &path,
                    r#"{"op":"accept","peer":"sha256:00"}"#,
                ],
            );
            let said = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            assert!(
                said == "empty" || said.starts_with("refused"),
                "{from} got an answer from {to}'s {socket}: {said}"
            );
        }
        let listing = sudo_as(from, &["ls", &format!("/home/{to}")]);
        assert!(!listing.status.success(), "{from} lists {to}'s home");
        let read = sudo_as(
            from,
            &["cat", &format!("/home/{to}/.config/Canoa/canoa.cfg")],
        );
        assert!(!read.status.success(), "{from} reads {to}'s settings");
    }

    // A person's own folder is theirs to write, so a link in it can lead to another person's socket. The portal dials it for the
    // account whose session asked and finds another user's process there: nothing is given, and nothing is paired.
    let swap = |from: &str, to: &str, socket: &str| {
        let (own, theirs) = (
            format!("/run/canoatest/{from}/{socket}"),
            format!("/run/canoatest/{to}/{socket}"),
        );
        let out = sudo_as(
            "canoatesta",
            &[
                "sh",
                "-c",
                &format!("rm -f '{own}' && ln -s '{theirs}' '{own}'"),
            ],
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    swap("canoatesta", "canoatestb", "entries/fixture.sock");
    swap("canoatesta", "canoatestb", "portal.sock");
    let led = a.link("/portal/ws/entry/fixture", |rpc| rpc.initialize());
    assert!(
        led.is_err(),
        "a channel led through a link to b's host: {led:?}"
    );
    let through = call(
        addr,
        "GET",
        "/canoa.json",
        &[("Cookie", a.cookie.clone())],
        "",
    );
    assert_eq!(through.status, 503, "{}", through.body);
    let waiting = key();
    let pair_through = call(
        addr,
        "POST",
        "/portal/pair",
        &[
            ("Cookie", a.cookie.clone()),
            ("Origin", origin(addr)),
            ("Content-Type", "application/json".to_owned()),
        ],
        &json!({"peer": waiting.peer_id().to_string()}).to_string(),
    );
    assert_eq!(pair_through.status, 503, "{}", pair_through.body);
    for socket in ["entries/fixture.sock", "portal.sock"] {
        let own = format!("/run/canoatest/canoatesta/{socket}");
        sudo_as("canoatesta", &["rm", "-f", &own]);
    }

    // A host nobody uses stops, and comes back at the next use with the key still paired.
    let first_pid = main_pid("canoatesta");
    wait_for("an idle host to stop", || {
        !active("canoatesta") && !active("canoatestb")
    });
    let again = call(
        addr,
        "GET",
        "/canoa.json",
        &[("Cookie", a.cookie.clone())],
        "",
    );
    assert_eq!(again.status, 200, "{}", again.body);
    assert!(active("canoatesta"));
    assert_ne!(main_pid("canoatesta"), first_pid, "a new process");
    assert!(
        !active("canoatestb"),
        "b's host stays stopped until b is back"
    );
    let paired = a
        .link("/portal/ws/entry/fixture", |rpc| rpc.initialize())
        .expect("the key was kept");
    assert!(paired.is_ok(), "{paired:?}");
    // Signing out ends the session.
    let out = call(
        addr,
        "POST",
        "/portal/logout",
        &[("Cookie", a.cookie.clone()), ("Origin", origin(addr))],
        "",
    );
    assert_eq!(out.status, 303);
    assert_eq!(
        call(
            addr,
            "GET",
            "/canoa.json",
            &[("Cookie", a.cookie.clone())],
            ""
        )
        .status,
        401
    );
}

/// A sign-in as the proxy makes it, the browser at `from`, carrying `certificate` (DER in base64) when it has one.
fn sign_in_via_proxy(user: &str, password: &str, from: &str, certificate: Option<&str>) -> Answer {
    let mut extra = vec![
        ("Origin", "http://localhost".to_owned()),
        (
            "Content-Type",
            "application/x-www-form-urlencoded".to_owned(),
        ),
        ("X-Forwarded-For", from.to_owned()),
    ];
    if let Some(certificate) = certificate {
        extra.push(("X-Client-Cert", certificate.to_owned()));
    }
    call_unix(
        "POST",
        "/portal/login",
        &extra,
        &if certificate.is_some() && password.is_empty() {
            "method=certificate".to_owned()
        } else {
            format!("user={user}&password={password}")
        },
    )
}

/// The methods the portal offers a browser the proxy says is at `from`.
fn methods_for(from: &str) -> Value {
    let answer = call_unix(
        "GET",
        "/portal/methods",
        &[("X-Forwarded-For", from.to_owned())],
        "",
    );
    assert_eq!(answer.status, 200, "{}", answer.body);
    serde_json::from_str::<Value>(&answer.body).expect("json")["methods"].clone()
}

/// The host's index with `cookie`, the browser at `from`: where a session is decided again.
fn index_via_proxy(cookie: &str, from: &str) -> Answer {
    call_unix(
        "GET",
        "/canoa.json",
        &[
            ("Cookie", cookie.to_owned()),
            ("X-Forwarded-For", from.to_owned()),
        ],
        "",
    )
}

#[test]
#[ignore = "needs CANOA_TEST_PORTAL_USERS and the accounts tests/portal_users/setup.sh makes"]
fn the_network_rule_refuses_a_password_from_outside_and_takes_a_certificate_from_anywhere() {
    if std::env::var_os("CANOA_TEST_PORTAL_USERS").is_none() {
        panic!("CANOA_TEST_PORTAL_USERS is not set: run tests/portal_users/setup.sh first");
    }
    let _ = Command::new("sudo")
        .args(["-n", "systemctl", "stop", &unit("canoatesta")])
        .status();
    // A pin an earlier run made is not this run's.
    let _ = Command::new("sudo")
        .args([
            "-n",
            "-u",
            PORTAL_USER,
            "rm",
            "-f",
            "/var/lib/canoatest-portal/certificates.toml",
        ])
        .status();
    let made = rcgen::generate_simple_self_signed(vec!["canoa.example".to_owned()])
        .expect("a certificate");
    let certificate = data_encoding::BASE64.encode(made.cert.der());
    let _portal = start_portal_with("/opt/canoatest/portal_network.toml");
    let (inside, outside) = ("10.1.2.3", "203.0.113.7");

    // What the page offers, by where the proxy says the browser is.
    assert_eq!(methods_for(inside), json!(["password", "certificate"]));
    assert_eq!(methods_for(outside), json!(["certificate"]));
    assert_eq!(methods_for("2001:db8::1"), json!(["certificate"]));

    // A password from outside is refused before PAM is asked, as often as it is tried, and counts against nothing.
    let before = pam_asked();
    for password in [
        "pw-a-test",
        "not-the-password",
        "pw-a-test",
        "pw-a-test",
        "pw-a-test",
    ] {
        let refused = sign_in_via_proxy("canoatesta", password, outside, None);
        assert_eq!(refused.status, 403, "{}", refused.body);
        assert!(refused.cookie().is_none());
    }
    assert_eq!(pam_asked(), before, "a refused password reached PAM");

    // From inside it signs in, and the person is offered the certificate their browser carries.
    let by_password = sign_in_via_proxy("canoatesta", "pw-a-test", inside, Some(&certificate));
    assert_eq!(by_password.status, 303, "{}", by_password.body);
    assert_eq!(by_password.header("location"), Some("/portal/certificate"));
    assert_eq!(pam_asked(), before + 1);
    let password_cookie = by_password.cookie().expect("a session");
    assert_eq!(index_via_proxy(&password_cookie, inside).status, 200);
    let pinned = call_unix(
        "POST",
        "/portal/certificate",
        &[
            ("Origin", "http://localhost".to_owned()),
            ("Cookie", password_cookie.clone()),
            ("X-Forwarded-For", inside.to_owned()),
            ("X-Client-Cert", certificate.clone()),
            (
                "Content-Type",
                "application/x-www-form-urlencoded".to_owned(),
            ),
        ],
        "action=enrol",
    );
    assert_eq!(pinned.status, 200, "{}", pinned.body);
    assert!(pinned.body.contains("now signs you in"), "{}", pinned.body);

    // The password session does not follow the browser out of 10/8: its next use from outside ends it.
    assert_eq!(index_via_proxy(&password_cookie, outside).status, 401);
    assert_eq!(index_via_proxy(&password_cookie, inside).status, 401);

    // The certificate signs in from outside, as the person, and the session stands wherever the browser is.
    let by_certificate = sign_in_via_proxy("canoatesta", "", outside, Some(&certificate));
    assert_eq!(by_certificate.status, 303, "{}", by_certificate.body);
    assert_eq!(by_certificate.header("location"), Some("/"));
    let cookie = by_certificate.cookie().expect("a session");
    for from in [outside, inside] {
        let index = index_via_proxy(&cookie, from);
        assert_eq!(index.status, 200, "{}", index.body);
    }
    assert!(active("canoatesta"), "the person's host runs");
    assert_eq!(
        process_uid(main_pid("canoatesta")),
        uid_of("canoatesta"),
        "as the person"
    );
    // A certificate nobody pinned signs no one in, from anywhere.
    let other = rcgen::generate_simple_self_signed(vec!["other.example".to_owned()])
        .expect("a certificate");
    let unknown = data_encoding::BASE64.encode(other.cert.der());
    assert_eq!(
        sign_in_via_proxy("canoatesta", "", outside, Some(&unknown)).status,
        401
    );
    // Nothing the outside address did counted against the password a person types inside.
    assert_eq!(
        sign_in_via_proxy("canoatesta", "pw-a-test", inside, None).status,
        303
    );
    let _ = Command::new("sudo")
        .args(["-n", "systemctl", "stop", &unit("canoatesta")])
        .status();
}

/// A client certificate a CA signed that names `upn` in its alternative names, as a certificate from a directory's own CA does:
/// the CA as PEM and the certificate as the proxy forwards it (DER in base64).
struct TestCa {
    params: rcgen::CertificateParams,
    key: rcgen::KeyPair,
    pem: String,
}

impl TestCa {
    fn new(name: &str) -> Self {
        use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};
        let key = KeyPair::generate().expect("a key");
        let mut params = CertificateParams::new(Vec::<String>::new()).expect("parameters");
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.distinguished_name.push(DnType::CommonName, name);
        let pem = params.self_signed(&key).expect("a CA").pem();
        Self { params, key, pem }
    }

    /// A client certificate this CA signed that names `upn` in its alternative names, as the proxy forwards it (DER in base64).
    fn issue(&self, upn: &str) -> String {
        use rcgen::{
            CertificateParams, DnType, ExtendedKeyUsagePurpose, Issuer, KeyPair, OtherNameValue,
            SanType,
        };
        let key = KeyPair::generate().expect("a key");
        let mut params = CertificateParams::new(Vec::<String>::new()).expect("parameters");
        params
            .distinguished_name
            .push(DnType::CommonName, "a test person");
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        // The UPN is an `otherName` with the OID Microsoft gave it.
        params.subject_alt_names = vec![SanType::OtherName((
            vec![1, 3, 6, 1, 4, 1, 311, 20, 2, 3],
            OtherNameValue::Utf8String(upn.to_owned()),
        ))];
        let issuer = Issuer::from_params(&self.params, &self.key);
        let cert = params.signed_by(&key, &issuer).expect("a certificate");
        data_encoding::BASE64.encode(cert.der())
    }
}

#[test]
#[ignore = "needs CANOA_TEST_PORTAL_USERS and the accounts tests/portal_users/setup.sh makes"]
fn a_certificate_a_ca_signed_for_a_upn_signs_in_the_account_the_matcher_finds() {
    if std::env::var_os("CANOA_TEST_PORTAL_USERS").is_none() {
        panic!("CANOA_TEST_PORTAL_USERS is not set: run tests/portal_users/setup.sh first");
    }
    let _ = Command::new("sudo")
        .args(["-n", "systemctl", "stop", &unit("canoatesta")])
        .status();
    let ca = TestCa::new("Canoa test CA");
    let certificate = ca.issue("canoatesta@corp.example");
    // The CA the portal trusts is this test's own, written where only root may change it.
    let mut tee = Command::new("sudo")
        .args(["-n", "tee", "/etc/canoatest/clients-ca.pem"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("tee");
    tee.stdin
        .take()
        .expect("piped")
        .write_all(ca.pem.as_bytes())
        .expect("the CA is written");
    assert!(tee.wait().expect("tee ends").success());
    let _portal = start_portal_with("/opt/canoatest/portal_matcher.toml");
    let outside = "203.0.113.7";
    assert_eq!(methods_for(outside), json!(["certificate"]));

    // The certificate names canoatesta by its UPN, and the directory has that account: it signs in, from anywhere.
    let signed = sign_in_via_proxy("", "", outside, Some(&certificate));
    assert_eq!(signed.status, 303, "{}", signed.body);
    let cookie = signed.cookie().expect("a session");
    let session = call_unix(
        "GET",
        "/portal/session",
        &[
            ("Cookie", cookie.clone()),
            ("X-Forwarded-For", outside.to_owned()),
        ],
        "",
    );
    assert_eq!(session.status, 200, "{}", session.body);
    assert_eq!(
        serde_json::from_str::<Value>(&session.body).expect("json")["user"],
        "canoatesta"
    );
    assert_eq!(index_via_proxy(&cookie, outside).status, 200);
    assert_eq!(
        process_uid(main_pid("canoatesta")),
        uid_of("canoatesta"),
        "its host runs as the account the matcher found"
    );

    // Not this account: another realm's name, an account outside the sign-in group, and one the system does not have.
    for (index, upn) in [
        "canoatesta@evil.example",
        "canoatestc@corp.example",
        "nobodyatall@corp.example",
        "canoatestportal@corp.example",
    ]
    .into_iter()
    .enumerate()
    {
        let refused = ca.issue(upn);
        let answer =
            sign_in_via_proxy("", "", &format!("203.0.113.{}", 20 + index), Some(&refused));
        assert_eq!(answer.status, 401, "{upn}: {}", answer.body);
        assert!(answer.cookie().is_none());
    }

    // A certificate of a CA the portal does not trust is refused before any matching, whatever UPN it names.
    let stranger = TestCa::new("Another CA").issue("canoatesta@corp.example");
    assert_eq!(
        sign_in_via_proxy("", "", "203.0.113.99", Some(&stranger)).status,
        401
    );

    // The certificate on the loopback address, where no header is believed, never reaches the matcher.
    let local = call(
        _portal.addr,
        "POST",
        "/portal/login",
        &[
            ("Origin", origin(_portal.addr)),
            ("X-Client-Cert", certificate.clone()),
            (
                "Content-Type",
                "application/x-www-form-urlencoded".to_owned(),
            ),
        ],
        "method=certificate",
    );
    assert_eq!(local.status, 403, "{}", local.body);
    let _ = Command::new("sudo")
        .args(["-n", "systemctl", "stop", &unit("canoatesta")])
        .status();
}

/// Where setup.sh put the script, readable by every account.
fn uds_script() -> String {
    "/opt/canoatest/uds.py".to_owned()
}
