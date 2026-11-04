//! `canoa-host serve` serving the web app beside its entries: from a directory or the site built in, over HTTP on loopback
//! or HTTPS, with `/canoa-host.json` naming the entries it serves.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use serde_json::Value;
use support::{Host, Scratch, example, host_binary, method, quoted};

fn site() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("site")
}

fn agents(count: usize) -> String {
    let program = quoted(&example("stdio_agent", false).to_string_lossy());
    (0..count)
        .map(|index| format!("[host.agents.echo{index}]\nprogram = {program}\n"))
        .collect()
}

/// The status and body of `GET path` over `stream`, a connection to `authority`.
/// The status, the head in lower case and the body of `GET path` over `stream`, naming `host`.
fn get_over(
    stream: &mut (impl std::io::Read + std::io::Write),
    host: &str,
    path: &str,
) -> (u16, String, Vec<u8>) {
    let request = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .expect("the request is sent");
    let mut received = Vec::new();
    let _ = stream.read_to_end(&mut received);
    let end = received
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("an answer")
        + 4;
    let status = String::from_utf8_lossy(&received[..end])
        .split(' ')
        .nth(1)
        .and_then(|status| status.parse().ok())
        .expect("a status");
    (
        status,
        String::from_utf8_lossy(&received[..end]).to_ascii_lowercase(),
        received[end..].to_vec(),
    )
}

fn get(url: &str, path: &str) -> (u16, Vec<u8>) {
    let authority = url.strip_prefix("http://").expect("an http URL");
    let mut tcp = TcpStream::connect(authority).expect("it connects");
    tcp.set_read_timeout(Some(support::WAIT))
        .expect("a timeout");
    let (status, _, body) = get_over(&mut tcp, authority, path);
    (status, body)
}

/// The web app's URL, from the line `serve` prints after its entries.
fn web_url(host: &Host) -> String {
    let line: Value = serde_json::from_str(&host.next_line()).expect("JSON");
    line["web"]
        .as_str()
        .expect("the web app's address")
        .to_owned()
}

#[test]
fn serve_web_serves_a_directory_and_lists_its_entries() {
    let site = site();
    let host = Host::start_with(
        &agents(2),
        2,
        &[
            "--web",
            &site.to_string_lossy(),
            "--web-listen",
            "127.0.0.1:0",
        ],
        &[],
    );
    let url = web_url(&host);
    let (status, index) = get(&url, "/");
    assert_eq!(status, 200);
    assert_eq!(
        index,
        std::fs::read(site.join("index.html")).expect("the fixture")
    );
    let (status, script) = get(&url, "/app.js");
    assert_eq!(status, 200);
    assert_eq!(
        script,
        std::fs::read(site.join("app.js")).expect("the fixture")
    );
    assert_eq!(get(&url, "/sessions/1").1, index, "an app route is the app");
    assert_eq!(get(&url, "/../Canoa.cfg").0, 404);

    let (status, listed) = get(&url, "/canoa-host.json");
    assert_eq!(status, 200);
    let listed: Value = serde_json::from_slice(&listed).expect("JSON");
    assert_eq!(listed["peer"], host.peer.to_string());
    assert_eq!(listed["method"], method().name());
    let entries: Vec<(String, String, String)> = listed["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .map(|entry| {
            (
                entry["name"].as_str().expect("a name").to_owned(),
                entry["kind"].as_str().expect("a kind").to_owned(),
                entry["url"].as_str().expect("a url").to_owned(),
            )
        })
        .collect();
    assert_eq!(
        entries,
        [
            (
                "echo0".to_owned(),
                "agent".to_owned(),
                host.url("echo0").to_owned()
            ),
            (
                "echo1".to_owned(),
                "agent".to_owned(),
                host.url("echo1").to_owned()
            ),
        ]
    );
}

#[test]
fn the_web_table_serves_a_directory_named_from_canoa_cfg() {
    let scratch = Scratch::new();
    let copy = scratch.0.join("site");
    std::fs::create_dir_all(&copy).expect("a site");
    std::fs::copy(site().join("index.html"), copy.join("index.html")).expect("an index");
    let config = format!(
        "{}[host.web]\ndir = \"site\"\nlisten = \"127.0.0.1:0\"\n",
        agents(1)
    );
    let host = Host::start_in(scratch, &config, 1, &[]);
    let url = web_url(&host);
    assert!(url.starts_with("http://127.0.0.1:"), "{url}");
    assert_eq!(
        get(&url, "/").1,
        std::fs::read(site().join("index.html")).expect("the fixture")
    );
}

/// `canoa-host serve` on `config` with `args`, which is expected to end at once.
fn serve_refused(config: &str, args: &[&str]) -> (Option<i32>, String) {
    let scratch = Scratch::new();
    let path = scratch.0.join("Canoa.cfg");
    std::fs::write(&path, config).expect("Canoa.cfg is written");
    let output = Command::new(host_binary())
        .arg("serve")
        .arg("--config")
        .arg(&path)
        .args(args)
        .args(["--method", method().name()])
        .env("COQUI_AGENTIC_PEER_HOME", scratch.0.join("home"))
        .output()
        .expect("the host runs");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn plain_http_beyond_loopback_is_refused_without_a_certificate() {
    let site = site().to_string_lossy().into_owned();
    let (status, said) = serve_refused(
        &format!("{}[host.web]\nlisten = \"0.0.0.0:0\"\n", agents(1)),
        &[],
    );
    assert_eq!(status, Some(2), "{said}");
    assert!(said.contains("cert and key"), "{said}");
    let (status, said) = serve_refused(&agents(1), &["--web", &site, "--web-listen", "0.0.0.0:0"]);
    assert_eq!(status, Some(2), "{said}");
    assert!(said.contains("beyond loopback"), "{said}");
    let (status, said) = serve_refused(&agents(1), &["--web-listen", "127.0.0.1:0"]);
    assert_eq!(status, Some(2), "{said}");
}

#[test]
fn a_site_that_is_not_one_is_refused() {
    let scratch = Scratch::new();
    let (status, said) = serve_refused(
        &agents(1),
        &[
            "--web",
            &scratch.0.to_string_lossy(),
            "--web-listen",
            "127.0.0.1:0",
        ],
    );
    assert_eq!(status, Some(2), "{said}");
    assert!(said.contains("index.html"), "{said}");
}

#[test]
fn a_site_holding_canoa_cfg_is_refused() {
    let scratch = Scratch::new();
    std::fs::copy(site().join("index.html"), scratch.0.join("index.html")).expect("an index");
    let path = scratch.0.join("Canoa.cfg");
    std::fs::write(&path, agents(1)).expect("Canoa.cfg is written");
    let output = Command::new(host_binary())
        .arg("serve")
        .arg("--config")
        .arg(&path)
        .args([
            "--web",
            &scratch.0.to_string_lossy(),
            "--web-listen",
            "127.0.0.1:0",
        ])
        .args(["--method", method().name()])
        .env("COQUI_AGENTIC_PEER_HOME", scratch.0.join("home"))
        .output()
        .expect("the host runs");
    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{said}");
    assert!(said.contains("keeps private"), "{said}");
}

#[cfg(not(feature = "web"))]
#[test]
fn without_the_feature_there_is_no_site_built_in() {
    let (status, said) = serve_refused(&agents(1), &["--web", "--web-listen", "127.0.0.1:0"]);
    assert_eq!(status, Some(2), "{said}");
    assert!(said.contains("web feature"), "{said}");
}

#[cfg(feature = "web")]
#[test]
fn with_the_feature_the_site_built_in_is_served() {
    let host = Host::start_with(
        &agents(1),
        1,
        &["--web", "--web-listen", "127.0.0.1:0"],
        &[],
    );
    let url = web_url(&host);
    let dist = PathBuf::from(env!("CANOA_WEB_DIST_BUILT"));
    let (status, index) = get(&url, "/");
    assert_eq!(status, 200);
    assert_eq!(
        index,
        std::fs::read(dist.join("index.html")).expect("an index")
    );
}

#[test]
fn https_serves_the_site_with_the_certificate_given() {
    let scratch = Scratch::new();
    let made =
        rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).expect("a certificate");
    std::fs::write(scratch.0.join("web.crt"), made.cert.pem()).expect("the certificate");
    std::fs::write(scratch.0.join("web.key"), made.signing_key.serialize_pem()).expect("the key");
    let config = format!(
        "{}[host.web]\ndir = {}\nlisten = \"127.0.0.1:0\"\ncert = \"web.crt\"\nkey = \"web.key\"\nhosts = [\"canoa.example\"]\n",
        agents(1),
        quoted(&site().to_string_lossy())
    );
    let host = Host::start_in(scratch, &config, 1, &[]);
    let url = web_url(&host);
    let authority = url
        .strip_prefix("https://")
        .expect("an https URL")
        .to_owned();

    let mut roots = rustls::RootCertStore::empty();
    roots.add(made.cert.der().clone()).expect("a root");
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let client = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from("localhost").expect("a name");
    let client = Arc::new(client);
    let over_tls = |host: &str| {
        let session =
            rustls::ClientConnection::new(Arc::clone(&client), name.clone()).expect("a session");
        let tcp = TcpStream::connect(&authority).expect("it connects");
        tcp.set_read_timeout(Some(support::WAIT))
            .expect("a timeout");
        get_over(&mut rustls::StreamOwned::new(session, tcp), host, "/")
    };
    let (status, head, index) = over_tls("localhost");
    assert_eq!(status, 200);
    assert_eq!(
        index,
        std::fs::read(site().join("index.html")).expect("the fixture")
    );
    assert!(
        !head.contains("strict-transport-security"),
        "a browser keeps no HTTPS rule for localhost: {head}"
    );
    let (status, head, _) = over_tls("canoa.example");
    assert_eq!(status, 200);
    assert!(
        head.contains("strict-transport-security: max-age=31536000\r\n"),
        "a name the host serves stays on HTTPS: {head}"
    );

    let mut plain = TcpStream::connect(&authority).expect("it connects");
    plain
        .set_read_timeout(Some(support::WAIT))
        .expect("a timeout");
    plain
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .expect("a plain request");
    let mut received = Vec::new();
    let _ = plain.read_to_end(&mut received);
    assert!(
        !received.starts_with(b"HTTP/1.1 200"),
        "no plain HTTP on an HTTPS listener"
    );
}
