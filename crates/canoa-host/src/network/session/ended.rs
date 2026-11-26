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
use canoa::profile_item::{ClientCertificateHost, RouteEffect};
use coqui_syshttp::tls::{ClientCertificates, Identity, Selector, TlsBuilder, Trust};
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use super::tests::{TOKEN, ask, basic, connect};
use super::{Intercept, Middleman, Services, start};
use crate::network::proxy_ca::ProxyCa;
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

/// Entries for `patterns`, each presenting the keystores' own choice.
fn entries(patterns: &[&str]) -> Vec<ClientCertificateHost> {
    patterns
        .iter()
        .map(|pattern| ClientCertificateHost::new(*pattern))
        .collect()
}

/// An authority of its own, limited to `hosts` as a proxy's is when they are all plain hosts.
fn authority_for(hosts: &[&str]) -> Arc<ProxyCa> {
    Arc::new(ProxyCa::new(Some(&named(hosts))).expect("an authority"))
}

/// An authority of its own that vouches for every host, as a proxy's is when a pattern is a regular expression.
fn unlimited() -> Arc<ProxyCa> {
    Arc::new(ProxyCa::new(None).expect("an authority"))
}

fn naming(hosts: &[&str], ca: &Arc<ProxyCa>) -> Option<Arc<Intercept>> {
    Some(Arc::new(
        Intercept::new(&entries(hosts), Arc::clone(ca)).expect("patterns"),
    ))
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
    let selector = Selector::builder()
        .source(Keystore(identity))
        .send_sole_candidate(true)
        .build();
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
fn proxies_of_one_process_sign_one_destination_with_one_certificate_made_once() {
    let ca = Ca::new("Origin CA");
    let (origin, _, client) = insisting(&ca);
    let dir = Dir::new("two");
    // Two proxies of different settings (so two of them, as two sessions' items would make) that answer with one authority.
    let shared = unlimited();
    let seen: Vec<Vec<u8>> = (0..2)
        .map(|_| {
            let proxy = Middleman::start(
                with_file(&ca, &client, &dir),
                BypassRules::none(),
                TOKEN,
                naming(&["localhost"], &shared),
            )
            .expect("starts");
            program_gets(
                proxy.addr,
                ("localhost", origin.addr.port()),
                shared.certificate_pem(),
            )
            .expect("reached")
            .0
        })
        .collect();
    assert_eq!(
        seen[0], seen[1],
        "the certificate made for the destination is kept"
    );
    assert_eq!(shared.made(), 1);
}

#[test]
fn what_one_authority_gave_its_programs_to_trust_does_not_make_another_authoritys_proxy_believed() {
    let ca = Ca::new("Origin CA");
    let (origin, _, client) = insisting(&ca);
    let dir = Dir::new("other");
    let (first, second) = (authority_for(&["localhost"]), authority_for(&["localhost"]));
    assert_ne!(first.certificate_pem(), second.certificate_pem());
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
    .expect_err("the other authority is not this one");
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
fn the_hosts_a_profile_names_are_matched_as_names_addresses_and_regular_expressions() {
    let patterns = [
        "Api.Example.com",
        "*.corp.example",
        "10.0.0.7",
        "::1",
        "(db|cache)[.]internal[.]test",
    ];
    let intercept = Intercept::new(&entries(&patterns), unlimited()).expect("patterns");
    for yes in [
        "api.example.com",
        "API.EXAMPLE.COM.",
        "corp.example",
        "a.b.corp.example",
        "10.0.0.7",
        "0:0:0:0:0:0:0:1",
        "db.internal.test",
        "CACHE.internal.test",
    ] {
        assert!(intercept.rule_for(yes).is_some(), "{yes}");
    }
    for no in [
        "example.com",
        "xapi.example.com",
        "evilcorp.example",
        "corp.example.evil.test",
        "10.0.0.70",
        "localhost",
        "x.db.internal.test",
        "db.internal.test.evil.test",
    ] {
        assert!(intercept.rule_for(no).is_none(), "{no}");
    }
}

#[test]
fn the_first_entry_that_matches_a_destination_says_which_certificate_it_gets() {
    let ours = "ab".repeat(32);
    let mut list = vec![
        ClientCertificateHost::presenting("(db|cache)[.]internal[.]test", ours.clone()),
        ClientCertificateHost::new(".*"),
    ];
    let intercept = Intercept::new(&list, unlimited()).expect("patterns");
    assert_eq!(
        intercept
            .rule_for("db.internal.test")
            .and_then(|rule| rule.thumbprint.clone()),
        Some(ours.clone())
    );
    assert_eq!(
        intercept
            .rule_for("other.test")
            .map(|rule| rule.thumbprint.clone()),
        Some(None),
        "the catch-all presents what the keystores choose"
    );
    list.reverse();
    let intercept = Intercept::new(&list, unlimited()).expect("patterns");
    assert_eq!(
        intercept
            .rule_for("db.internal.test")
            .map(|rule| rule.thumbprint.clone()),
        Some(None),
        "the order is the profile's"
    );
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
    let mut asked = RouteEffect::new();
    asked.intercept = entries(&["api.example.test"]);
    let services = Services {
        private: Some(&private.0),
    };
    let session = start(&asked, &[], &services).expect("starts");
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
fn sessions_that_name_the_same_hosts_are_given_the_same_authority_and_the_limit_decides_which() {
    let given = |patterns: &[&str], dir: &Dir| {
        let mut asked = RouteEffect::new();
        // A bypass entry of its own keeps this test's proxies apart from the other tests' running ones.
        asked.bypass = vec!["authority.example.test".to_owned()];
        asked.intercept = entries(patterns);
        let services = Services {
            private: Some(&dir.0),
        };
        let session = start(&asked, &[], &services).expect("starts");
        let roots = session
            .variables
            .iter()
            .find(|(name, _)| name == "NODE_EXTRA_CA_CERTS")
            .map(|(_, path)| path.clone())
            .expect("the authority alone");
        (fs::read_to_string(roots).expect("reads"), session)
    };
    let dirs: Vec<Dir> = (0..5).map(|n| Dir::new(&format!("given-{n}"))).collect();
    let (one, _keep_one) = given(&["api.example.test"], &dirs[0]);
    let (again, _keep_again) = given(&["API.example.test"], &dirs[1]);
    assert_eq!(
        one, again,
        "plain hosts the same: one authority, whatever the session"
    );
    let (more, _keep_more) = given(&["api.example.test", "*.corp.example"], &dirs[2]);
    assert_ne!(
        one, more,
        "another set of hosts is another limit, so another authority"
    );
    let (expression, _keep_expression) = given(&["api[.]example[.]test"], &dirs[3]);
    let (all, _keep_all) = given(&[".*"], &dirs[4]);
    assert_eq!(
        expression, all,
        "a pattern that is a regular expression is the one authority that has no limit"
    );
    assert_ne!(expression, one);
}

#[test]
fn ending_tls_needs_a_directory_to_keep_what_programs_trust_in() {
    let mut asked = RouteEffect::new();
    asked.intercept = entries(&["api.example.test"]);
    let why = start(&asked, &[], &Services::default()).expect_err("refused");
    assert!(why.contains("private directory"), "{why}");
}

/// A keystore of the person's holding Alice's and Bob's certificates, both issued by the origin's authority, and the network that
/// presents them: with two to choose between and nothing to pick one, the keystores' own choice is to send none.
fn two_identities(ca: &Ca) -> (Arc<Network>, Identity, Identity) {
    let identity = |leaf: &Leaf| {
        Identity::from_pem(leaf.chain_pem.as_bytes(), leaf.key_pem.as_bytes()).expect("one")
    };
    let (alice, bob) = (
        identity(&ca.issue_client("Alice Example")),
        identity(&ca.issue_client("Bob Example")),
    );
    let selector = Selector::builder()
        .source(vec![alice.clone(), bob.clone()])
        .build();
    let tls = TlsBuilder::default()
        .trust(Trust::Roots(vec![ca.der()]))
        .client_certificates(ClientCertificates::Selector(Arc::new(selector)));
    (Arc::new(Network::with(direct(), tls)), alice, bob)
}

fn thumbprint(identity: &Identity) -> String {
    data_encoding::HEXLOWER.encode(identity.info().fingerprint())
}

#[test]
fn an_entry_that_names_a_certificate_presents_that_one_and_the_keystores_own_choice_presents_none_of_two()
 {
    let ca = Ca::new("Origin CA");
    let (origin, _, _) = insisting(&ca);
    let (network, alice, bob) = two_identities(&ca);
    let session_ca = unlimited();
    for (who, wanted) in [("Bob", &bob), ("Alice", &alice)] {
        let proxy = Middleman::start(
            Arc::clone(&network),
            BypassRules::none(),
            TOKEN,
            Some(Arc::new(
                Intercept::new(
                    &[ClientCertificateHost::presenting(".*", thumbprint(wanted))],
                    Arc::clone(&session_ca),
                )
                .expect("patterns"),
            )),
        )
        .expect("starts");
        let (_, body) = program_gets(
            proxy.addr,
            ("localhost", origin.addr.port()),
            session_ca.certificate_pem(),
        )
        .expect("reached");
        assert_eq!(body, "client certificate seen", "{who}");
        assert_eq!(
            origin.presented().last(),
            Some(&Some(wanted.chain()[0].as_ref().to_vec())),
            "the origin saw {who}'s certificate"
        );
    }
    let seen = origin.presented().len();
    let proxy = Middleman::start(
        Arc::clone(&network),
        BypassRules::none(),
        TOKEN,
        naming(&["localhost"], &session_ca),
    )
    .expect("starts");
    match program_gets(
        proxy.addr,
        ("localhost", origin.addr.port()),
        session_ca.certificate_pem(),
    ) {
        Err(_) => {}
        Ok((_, body)) => assert_ne!(body, "client certificate seen"),
    }
    assert_eq!(
        origin.presented()[seen..]
            .iter()
            .filter(|certificate| certificate.is_some())
            .count(),
        0,
        "with two certificates to choose between and no one to choose, system presents none"
    );
}

#[test]
fn an_entry_that_names_a_certificate_the_keystores_do_not_hold_is_refused_before_any_connection() {
    let ca = Ca::new("Origin CA");
    let (origin, _, _) = insisting(&ca);
    let (network, alice, _) = two_identities(&ca);
    let session_ca = unlimited();
    let unknown = "cd".repeat(32);
    let proxy_for = |wanted: &str| {
        Middleman::start(
            Arc::clone(&network),
            BypassRules::none(),
            TOKEN,
            Some(Arc::new(
                Intercept::new(
                    &[ClientCertificateHost::presenting(".*", wanted)],
                    Arc::clone(&session_ca),
                )
                .expect("patterns"),
            )),
        )
        .expect("starts")
    };
    let proxy = proxy_for(&unknown);
    let said = program_gets(
        proxy.addr,
        ("localhost", origin.addr.port()),
        session_ca.certificate_pem(),
    )
    .expect_err("no such certificate");
    assert!(
        said.starts_with("HTTP/1.1 502") && said.contains("thumbprint"),
        "{said}"
    );
    assert!(origin.presented().is_empty(), "no connection was made");
    // A thumbprint is read in any case and with colons, so Alice's in capitals is hers.
    let spelled = thumbprint(&alice)
        .to_uppercase()
        .as_bytes()
        .chunks(2)
        .map(|pair| String::from_utf8_lossy(pair).into_owned())
        .collect::<Vec<_>>()
        .join(":");
    let proxy = proxy_for(&spelled);
    let (_, body) = program_gets(
        proxy.addr,
        ("localhost", origin.addr.port()),
        session_ca.certificate_pem(),
    )
    .expect("reached");
    assert_eq!(body, "client certificate seen");
    assert_eq!(
        origin.presented(),
        [Some(alice.chain()[0].as_ref().to_vec())]
    );
}

#[test]
fn a_regular_expression_ends_tls_for_the_hosts_it_matches_and_a_catch_all_for_every_one() {
    let ca = Ca::new("Origin CA");
    let dir = Dir::new("expressions");
    for (pattern, matches) in [
        ("loc.*host", true),
        (".*", true),
        ("(l|L)ocal(host)?", true),
        ("(api|auth)[.]example[.]test", false),
        ("localhos", false),
    ] {
        let (origin, _, client) = insisting(&ca);
        let session_ca = unlimited();
        let proxy = Middleman::start(
            with_file(&ca, &client, &dir),
            BypassRules::none(),
            TOKEN,
            naming(&[pattern], &session_ca),
        )
        .expect("starts");
        let outcome = program_gets(
            proxy.addr,
            ("localhost", origin.addr.port()),
            session_ca.certificate_pem(),
        );
        if matches {
            let (_, body) = outcome.unwrap_or_else(|why| panic!("{pattern}: {why}"));
            assert_eq!(body, "client certificate seen", "{pattern}");
            assert_eq!(
                origin.presented(),
                [Some(client.chain[0].as_ref().to_vec())],
                "{pattern}"
            );
        } else {
            let why = outcome.expect_err("a tunnel the program's own TLS cannot finish");
            assert!(why.contains("TLS failed"), "{pattern}: {why}");
            assert!(
                origin.presented().iter().all(Option::is_none),
                "{pattern}: the proxy added nothing of the person's"
            );
        }
    }
}

fn variable(session: &super::SessionNetwork, name: &str) -> String {
    session
        .variables
        .iter()
        .find(|(each, _)| each == name)
        .map(|(_, value)| value.clone())
        .expect("a variable")
}

/// The proxy address in a session's variables.
fn proxy_of(session: &super::SessionNetwork) -> SocketAddr {
    variable(session, "HTTPS_PROXY")
        .rsplit_once('@')
        .and_then(|(_, address)| address.parse().ok())
        .expect("a proxy address")
}

fn reachable(address: SocketAddr) -> bool {
    std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_millis(500)).is_ok()
}

#[test]
fn sessions_whose_proxy_settings_are_equal_share_one_proxy_and_its_secret_and_different_ones_do_not()
 {
    let dirs: Vec<Dir> = (0..4).map(|n| Dir::new(&format!("shared-{n}"))).collect();
    let session = |bypass: &str, identity: Option<&str>, dir: &Dir| {
        let mut asked = RouteEffect::new();
        asked.bypass = vec![bypass.to_owned()];
        asked.intercept = vec![match identity {
            Some(thumbprint) => {
                ClientCertificateHost::presenting("api[.]example[.]test", thumbprint)
            }
            None => ClientCertificateHost::new("api[.]example[.]test"),
        }];
        let services = Services {
            private: Some(&dir.0),
        };
        start(&asked, &[], &services).expect("starts")
    };
    let first = session("shared.example.test", None, &dirs[0]);
    let second = session("shared.example.test", None, &dirs[1]);
    assert_eq!(
        variable(&first, "HTTPS_PROXY"),
        variable(&second, "HTTPS_PROXY"),
        "one proxy and one secret for sessions that ask for the same"
    );
    assert!(Arc::ptr_eq(&first.network, &second.network));
    let bypass_differs = session("shared-other.example.test", None, &dirs[2]);
    let identity_differs = session("shared.example.test", Some(&"ab".repeat(32)), &dirs[3]);
    for other in [&bypass_differs, &identity_differs] {
        assert_ne!(
            proxy_of(other),
            proxy_of(&first),
            "settings that differ get a proxy of their own"
        );
        assert_ne!(
            variable(other, "HTTPS_PROXY"),
            variable(&first, "HTTPS_PROXY")
        );
    }
    assert_ne!(proxy_of(&bypass_differs), proxy_of(&identity_differs));
    // What each session's programs are told to trust is the one authority's certificate, in a file of each session's own directory.
    let trusted = |session: &super::SessionNetwork| {
        fs::read_to_string(variable(session, "NODE_EXTRA_CA_CERTS")).expect("reads")
    };
    assert_eq!(trusted(&first), trusted(&second));
    assert_ne!(
        variable(&first, "NODE_EXTRA_CA_CERTS"),
        variable(&second, "NODE_EXTRA_CA_CERTS")
    );
    // The proxy ends with the last session that uses it, and not before.
    let address = proxy_of(&first);
    drop(first);
    assert!(reachable(address), "a session still uses it");
    drop(second);
    assert!(!reachable(address), "no session uses it");
    assert!(
        reachable(proxy_of(&bypass_differs)),
        "the others are not its business"
    );
}

fn running_proxy() -> Result<super::Running, String> {
    let token = super::new_token().map_err(|error| error.to_string())?;
    let network = Network::current();
    let proxy = Middleman::start(Arc::clone(&network), BypassRules::none(), &token, None)
        .map_err(|error| error.to_string())?;
    Ok(super::Running {
        proxy,
        token,
        network,
        intercept: None,
    })
}

#[test]
fn a_proxy_no_session_uses_is_kept_for_the_idle_time_for_the_next_session_and_then_ended() {
    let registry = super::Registry::new();
    registry.set_idle(std::time::Duration::from_secs(2));
    let mut asked = RouteEffect::new();
    asked.bypass = vec!["idle.example.test".to_owned()];
    let key = super::ProxyKey::of(&asked, &[], None);
    let first = registry.acquire(key.clone(), running_proxy).expect("made");
    let address = first.running.proxy.addr;
    drop(first);
    assert_eq!(registry.running(), 1, "kept for the idle time");
    assert!(reachable(address));
    // The next session within the time is given the same proxy, and the earlier timer, now out of date, ends nothing.
    std::thread::sleep(std::time::Duration::from_millis(1000));
    let again = registry
        .acquire(key.clone(), || Err("a proxy is running".to_owned()))
        .expect("the one that was kept");
    assert_eq!(again.running.proxy.addr, address);
    drop(again);
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(
        registry.running(),
        1,
        "the idle time began again with the last session"
    );
    let started = std::time::Instant::now();
    while registry.running() > 0 && started.elapsed() < std::time::Duration::from_secs(10) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(registry.running(), 0, "ended once idle for the time");
    // The proxy's threads are stopped just after it leaves the registry.
    while reachable(address) && started.elapsed() < std::time::Duration::from_secs(10) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(!reachable(address));
}
