//! A session's proxy ending TLS for the destinations its profile names, over loopback stand-ins: an origin that asks for a client
//! certificate, a program that trusts only what its session gave it, and the keystore of the person.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::fs;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use canoa::network::{BypassRules, ClientCertificate, Network, NetworkSettings, ProxyChoice};
use canoa::profile_item::NetworkEffect;
use coqui_syshttp::tls::{ClientCertificates, Identity, Selector, TlsBuilder, Trust};
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use super::tests::{TOKEN, ask, basic, connect};
use super::{Intercept, Middleman, Services, start};
use crate::network::session_ca::SessionCa;
use crate::network::standins::{Ca, ClientAuth, Leaf, TlsServer};
use crate::network::tests::{Dir, Keystore, direct};

/// What a program that trusts only `trust` (PEM) got from `https://{host}:{port}/x` through the proxy at `proxy`: the certificate the
/// far side showed it and the body of the answer; or, when the proxy would not connect, what it said.
fn program_gets(
    proxy: SocketAddr,
    (host, port): (&str, u16),
    trust: &str,
) -> Result<(Vec<u8>, String), String> {
    let (mut stream, head) = ask(
        proxy,
        &connect(&format!("{host}:{port}"), Some(&basic(TOKEN))),
    );
    if !head.starts_with("HTTP/1.1 200") {
        let mut rest = String::new();
        let _ = stream.read_to_string(&mut rest);
        return Err(format!("{head}{rest}"));
    }
    let mut roots = RootCertStore::empty();
    for der in CertificateDer::pem_slice_iter(trust.as_bytes()).flatten() {
        roots.add(der).map_err(|error| error.to_string())?;
    }
    let config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|error| error.to_string())?
            .with_root_certificates(roots)
            .with_no_client_auth();
    let name = ServerName::try_from(host.to_owned()).map_err(|error| error.to_string())?;
    let connection = ClientConnection::new(Arc::new(config), name).map_err(|e| e.to_string())?;
    let mut tls = StreamOwned::new(connection, stream);
    tls.write_all(
        format!("GET /x HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .map_err(|error| format!("the program's TLS failed: {error}"))?;
    let mut reply = Vec::new();
    let _ = tls.read_to_end(&mut reply);
    let shown = tls
        .conn
        .peer_certificates()
        .and_then(|chain| chain.first())
        .map(|der| der.as_ref().to_vec())
        .ok_or_else(|| "the far side showed no certificate".to_owned())?;
    let text = String::from_utf8_lossy(&reply).into_owned();
    let body = text.split_once("\r\n\r\n").map_or("", |(_, body)| body);
    Ok((shown, body.to_owned()))
}

/// A destination that insists on a client certificate and says whether it got one, and a client certificate its authority issued.
fn insisting(ca: &Ca) -> (TlsServer, Leaf, Leaf) {
    let server = ca.issue_server(&["localhost"]);
    let origin = TlsServer::start(
        &server,
        &[ca.der()],
        ClientAuth::Required,
        Arc::new(|_, seen| {
            if seen {
                b"client certificate seen".to_vec()
            } else {
                b"no client certificate".to_vec()
            }
        }),
    );
    (origin, server, ca.issue_client("Alice Example"))
}

/// A network that trusts `ca` and presents `client` from a file, as `[host.network]` says.
fn with_file(ca: &Ca, client: &Leaf, dir: &Dir) -> Arc<Network> {
    let (roots, identity) = (dir.0.join("ca.pem"), dir.0.join("client.pem"));
    fs::write(&roots, ca.pem()).expect("writes");
    fs::write(&identity, format!("{}{}", client.chain_pem, client.key_pem)).expect("writes");
    let mut settings = NetworkSettings::default();
    settings.proxy = ProxyChoice::None;
    settings.ca_file = Some(roots);
    settings.client_certificate = ClientCertificate::File(identity);
    Arc::new(Network::new(&settings).expect("a network"))
}

fn named(hosts: &[&str]) -> Vec<String> {
    hosts.iter().map(|host| (*host).to_owned()).collect()
}

/// A session's authority, limited to `hosts` as the proxy's own are.
fn authority_for(hosts: &[&str]) -> Arc<SessionCa> {
    Arc::new(SessionCa::new(&named(hosts)).expect("an authority"))
}

fn naming(hosts: &[&str], ca: &Arc<SessionCa>) -> Option<Arc<Intercept>> {
    Some(Arc::new(Intercept::new(&named(hosts), Arc::clone(ca))))
}

#[test]
fn a_destination_the_profile_names_gets_the_certificate_of_a_file_through_the_session_proxy() {
    let ca = Ca::new("Origin CA");
    let (origin, server, client) = insisting(&ca);
    let dir = Dir::new("ended-file");
    let session_ca = authority_for(&["localhost"]);
    let proxy = Middleman::start(
        with_file(&ca, &client, &dir),
        BypassRules::none(),
        TOKEN,
        naming(&["localhost"], &session_ca),
    )
    .expect("starts");
    // The program trusts the session's authority and nothing else: the origin's own authority is unknown to it.
    let (shown, body) = program_gets(
        proxy.addr,
        ("localhost", origin.addr.port()),
        session_ca.certificate_pem(),
    )
    .expect("reached");
    assert_eq!(body, "client certificate seen");
    assert_eq!(
        origin.presented(),
        [Some(client.chain[0].as_ref().to_vec())],
        "the origin saw the person's certificate"
    );
    assert_ne!(
        shown,
        server.chain[0].as_ref(),
        "the program was shown the session's certificate, not the origin's"
    );
}

#[test]
fn a_destination_the_profile_names_gets_the_keystores_certificate_the_origin_names_an_authority_for()
 {
    let ca = Ca::new("Origin CA");
    let (origin, _, client) = insisting(&ca);
    let identity =
        Identity::from_pem(client.chain_pem.as_bytes(), client.key_pem.as_bytes()).expect("one");
    let selector = Selector::builder().source(Keystore(identity)).build();
    let tls = TlsBuilder::default()
        .trust(Trust::Roots(vec![ca.der()]))
        .client_certificates(ClientCertificates::Selector(Arc::new(selector)));
    let session_ca = authority_for(&["*.localhost", "other.example.test"]);
    let proxy = Middleman::start(
        Arc::new(Network::with(direct(), tls)),
        BypassRules::none(),
        TOKEN,
        naming(&["*.localhost", "other.example.test"], &session_ca),
    )
    .expect("starts");
    let (_, body) = program_gets(
        proxy.addr,
        ("localhost", origin.addr.port()),
        session_ca.certificate_pem(),
    )
    .expect("reached");
    assert_eq!(body, "client certificate seen");
    assert_eq!(
        origin.presented(),
        [Some(client.chain[0].as_ref().to_vec())]
    );
}

#[test]
fn an_origin_that_wants_a_certificate_the_person_has_none_for_never_sees_one_nor_says_it_did() {
    let ca = Ca::new("Origin CA");
    let (origin, _, client) = insisting(&ca);
    let dir = Dir::new("ended-none");
    let roots = dir.0.join("ca.pem");
    fs::write(&roots, ca.pem()).expect("writes");
    let mut settings = NetworkSettings::default();
    settings.proxy = ProxyChoice::None;
    settings.ca_file = Some(roots);
    settings.client_certificate = ClientCertificate::None;
    let session_ca = authority_for(&["localhost"]);
    let proxy = Middleman::start(
        Arc::new(Network::new(&settings).expect("a network")),
        BypassRules::none(),
        TOKEN,
        naming(&["localhost"], &session_ca),
    )
    .expect("starts");
    // A server that insists says so after the handshake a client with no certificate finishes, so the program may be told at the
    // proxy or by a connection that ends; what it is never told is that the person's certificate was seen.
    match program_gets(
        proxy.addr,
        ("localhost", origin.addr.port()),
        session_ca.certificate_pem(),
    ) {
        Err(_) => {}
        Ok((_, body)) => assert_ne!(body, "client certificate seen"),
    }
    assert!(origin.presented().iter().all(Option::is_none));
    let _ = client;
}

#[test]
fn an_origin_the_networks_trust_does_not_vouch_for_is_never_reached_and_gets_no_certificate() {
    let ca = Ca::new("Origin CA");
    let (origin, _, _) = insisting(&ca);
    // No `ca_file`: the operating system's verifier decides, and it has never heard of the stand-in's authority.
    let mut settings = NetworkSettings::default();
    settings.proxy = ProxyChoice::None;
    let session_ca = authority_for(&["localhost"]);
    let proxy = Middleman::start(
        Arc::new(Network::new(&settings).expect("a network")),
        BypassRules::none(),
        TOKEN,
        naming(&["localhost"], &session_ca),
    )
    .expect("starts");
    let said = program_gets(
        proxy.addr,
        ("localhost", origin.addr.port()),
        session_ca.certificate_pem(),
    )
    .expect_err("refused");
    assert!(said.starts_with("HTTP/1.1 502"), "{said}");
    assert!(
        origin.presented().iter().all(Option::is_none),
        "no certificate of the person's went to an origin nobody vouched for"
    );
}

#[test]
fn a_destination_not_named_stays_a_tunnel_the_programs_own_tls_runs_through() {
    let ca = Ca::new("Origin CA");
    let server = ca.issue_server(&["localhost"]);
    let origin = TlsServer::start(
        &server,
        &[ca.der()],
        ClientAuth::Optional,
        Arc::new(|_, seen| {
            if seen {
                b"client certificate seen".to_vec()
            } else {
                b"no client certificate".to_vec()
            }
        }),
    );
    let client = ca.issue_client("Alice Example");
    let dir = Dir::new("tunnel");
    let session_ca = authority_for(&["other.example.test", "*.corp.example"]);
    let proxy = Middleman::start(
        with_file(&ca, &client, &dir),
        BypassRules::none(),
        TOKEN,
        naming(&["other.example.test", "*.corp.example"], &session_ca),
    )
    .expect("starts");
    // The program trusts the origin's authority, which only an untouched tunnel lets it reach.
    let (shown, body) =
        program_gets(proxy.addr, ("localhost", origin.addr.port()), &ca.pem()).expect("reached");
    assert_eq!(
        body, "no client certificate",
        "the proxy added nothing of the person's"
    );
    assert_eq!(
        shown,
        server.chain[0].as_ref(),
        "the program saw the origin's own certificate"
    );
    assert_eq!(origin.presented(), [None]);
}

#[test]
fn two_sessions_naming_the_same_destination_have_authorities_and_certificates_of_their_own() {
    let ca = Ca::new("Origin CA");
    let (origin, _, client) = insisting(&ca);
    let dir = Dir::new("two");
    let first = authority_for(&["localhost"]);
    let second = authority_for(&["localhost"]);
    assert_ne!(first.certificate_pem(), second.certificate_pem());
    let seen: Vec<Vec<u8>> = [&first, &second]
        .into_iter()
        .map(|session_ca| {
            let proxy = Middleman::start(
                with_file(&ca, &client, &dir),
                BypassRules::none(),
                TOKEN,
                naming(&["localhost"], session_ca),
            )
            .expect("starts");
            program_gets(
                proxy.addr,
                ("localhost", origin.addr.port()),
                session_ca.certificate_pem(),
            )
            .expect("reached")
            .0
        })
        .collect();
    assert_ne!(seen[0], seen[1], "each session signs its own");
    // What one session gave its programs to trust does not make the other's proxy believed.
    let proxy = Middleman::start(
        with_file(&ca, &client, &dir),
        BypassRules::none(),
        TOKEN,
        naming(&["localhost"], &second),
    )
    .expect("starts");
    let refused = program_gets(
        proxy.addr,
        ("localhost", origin.addr.port()),
        first.certificate_pem(),
    )
    .expect_err("the other session's authority is not this one's");
    assert!(refused.contains("TLS failed"), "{refused}");
}

#[test]
fn a_program_that_trusts_a_sessions_authority_is_refused_a_certificate_for_a_destination_it_does_not_name()
 {
    // The authority is limited to another host than the one this proxy answers for, which is what a certificate signed with a
    // leaked key would look like: the program trusts the authority and still refuses the certificate.
    let ca = Ca::new("Origin CA");
    let (origin, _, client) = insisting(&ca);
    let dir = Dir::new("outside");
    let session_ca = authority_for(&["other.example.test"]);
    let proxy = Middleman::start(
        with_file(&ca, &client, &dir),
        BypassRules::none(),
        TOKEN,
        naming(&["localhost"], &session_ca),
    )
    .expect("starts");
    let refused = program_gets(
        proxy.addr,
        ("localhost", origin.addr.port()),
        session_ca.certificate_pem(),
    )
    .expect_err("a certificate the authority may not sign is refused");
    assert!(refused.contains("TLS failed"), "{refused}");
}

#[test]
fn the_hosts_a_profile_names_are_matched_as_names_and_addresses() {
    let hosts: Vec<String> = ["Api.Example.com", "*.corp.example", "10.0.0.7", "::1"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    let ca = Arc::new(SessionCa::new(&hosts).expect("an authority"));
    let intercept = Intercept::new(&hosts, ca);
    for yes in [
        "api.example.com",
        "API.EXAMPLE.COM.",
        "corp.example",
        "a.b.corp.example",
        "10.0.0.7",
        "0:0:0:0:0:0:0:1",
    ] {
        assert!(intercept.covers(yes), "{yes}");
    }
    for no in [
        "example.com",
        "xapi.example.com",
        "evilcorp.example",
        "corp.example.evil.test",
        "10.0.0.70",
        "localhost",
    ] {
        assert!(!intercept.covers(no), "{no}");
    }
}

fn files_under(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).expect("lists") {
        let path = entry.expect("an entry").path();
        if path.is_dir() {
            found.extend(files_under(&path));
        } else {
            found.push(path.to_string_lossy().into_owned());
        }
    }
    found.sort();
    found
}

#[test]
fn what_a_session_is_given_to_trust_holds_the_authoritys_certificate_and_never_its_key() {
    let private = Dir::new("given-private");
    let mut asked = NetworkEffect::new("system");
    asked.intercept = vec!["api.example.test".to_owned()];
    let services = Services {
        private: Some(&private.0),
    };
    let session = start(&asked, &|_| None, &services).expect("starts");
    let value = |name: &str| {
        session
            .variables
            .iter()
            .find(|(each, _)| each == name)
            .map(|(_, value)| value.clone())
    };
    let roots = value("SSL_CERT_FILE").expect("a bundle");
    for name in [
        "REQUESTS_CA_BUNDLE",
        "CURL_CA_BUNDLE",
        "GIT_SSL_CAINFO",
        "PIP_CERT",
        "CARGO_HTTP_CAINFO",
        "NPM_CONFIG_CAFILE",
    ] {
        assert_eq!(value(name), Some(roots.clone()), "{name}");
    }
    let only = value("NODE_EXTRA_CA_CERTS").expect("the authority alone");
    assert_ne!(only, roots);
    for path in [&roots, &only] {
        assert!(
            Path::new(path).starts_with(&private.0),
            "{path} is in the session's own directory"
        );
    }
    // The authority alone for a program that adds to its roots; the authority first and then the system's for one that replaces them.
    let alone = fs::read_to_string(&only).expect("reads");
    let with_roots = fs::read_to_string(&roots).expect("reads");
    assert_eq!(alone.matches("BEGIN CERTIFICATE").count(), 1);
    assert!(with_roots.starts_with(&alone));
    for text in [&alone, &with_roots] {
        assert!(
            !text.contains("PRIVATE KEY"),
            "no key in what a program reads"
        );
    }
    // The authority's key is made in memory and written nowhere: the session's directory holds the two bundles, and every block in
    // them is a certificate.
    let kept = files_under(&private.0);
    assert_eq!(
        kept.len(),
        2,
        "the session's directory holds the two bundles: {kept:?}"
    );
    for file in &kept {
        let text = fs::read_to_string(file).expect("text");
        let blocks = text.matches("-----BEGIN ").count();
        assert_eq!(blocks, text.matches("-----BEGIN CERTIFICATE-----").count());
        assert_eq!(
            CertificateDer::pem_slice_iter(text.as_bytes())
                .flatten()
                .count(),
            blocks,
            "{file}: every block is a certificate"
        );
    }
    for (name, text) in &session.variables {
        assert!(!text.contains("PRIVATE KEY"), "{name}");
    }
    assert!(!session.description.contains("PRIVATE"));
    assert!(
        session.description.contains("api.example.test"),
        "the report says which hosts it ends TLS for: {}",
        session.description
    );
}

#[test]
fn two_sessions_that_name_one_host_are_given_different_authorities() {
    let (first_dir, second_dir) = (Dir::new("given-one"), Dir::new("given-two"));
    let mut asked = NetworkEffect::new("system");
    asked.intercept = vec!["api.example.test".to_owned()];
    let given = |dir: &Dir| {
        let services = Services {
            private: Some(&dir.0),
        };
        let session = start(&asked, &|_| None, &services).expect("starts");
        let roots = session
            .variables
            .iter()
            .find(|(name, _)| name == "NODE_EXTRA_CA_CERTS")
            .map(|(_, path)| path.clone())
            .expect("the authority alone");
        fs::read_to_string(roots).expect("reads")
    };
    assert_ne!(given(&first_dir), given(&second_dir));
}

#[test]
fn ending_tls_needs_the_proxy_the_host_runs_and_a_directory_to_keep_what_programs_trust_in() {
    let mut asked = NetworkEffect::new("system");
    asked.intercept = vec!["api.example.test".to_owned()];
    let why = start(&asked, &|_| None, &Services::default()).expect_err("refused");
    assert!(why.contains("private directory"), "{why}");
    asked.mode = "proxy".to_owned();
    asked.url = Some("http://gateway.example:3128".to_owned());
    assert!(
        asked
            .problem()
            .is_some_and(|why| why.contains("system or pac"))
    );
}
