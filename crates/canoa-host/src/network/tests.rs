//! Every fetch and every program the host starts follows `[host.network]`, over loopback stand-ins: a proxy that wants a login, a TLS
//! server that may ask for a client certificate, and a test CA. No test installs the process's network.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use canoa::network::{ClientCertificate, Network, NetworkSettings, ProxyChoice, Secret};
use canoa::store::{Check, FetchRequest, InstallTools};
use coqui_syshttp::tls::{
    CertificateChoice, ClientCertificates, Identity, IdentitySource, Selector, TlsBuilder, Trust,
};
use coqui_syshttp::{Environment, Resolver};
use sha2::{Digest, Sha256};

use super::standins::{Ca, ClientAuth, Leaf, StandInProxy, TlsServer};
use crate::spawn::Command;
use crate::store::tools::{HttpTransport, LocalTools, ToolsLimits, Transport};

/// A directory removed when dropped.
pub(crate) struct Dir(pub(crate) PathBuf);

impl Dir {
    pub(crate) fn new(name: &str) -> Self {
        let mut nonce = [0u8; 4];
        getrandom::fill(&mut nonce).expect("random");
        let hex: String = data_encoding::HEXLOWER.encode(&nonce);
        let path = std::env::temp_dir().join(format!("canoa-net-{name}-{hex}"));
        fs::create_dir_all(&path).expect("a directory");
        Self(path)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(crate) const USER: &str = "alice";
pub(crate) const PASSWORD: &str = "s3cret@x";

/// Everything a request through the stand-in proxy needs: the CA, a server for `names` that is the proxy's only origin, and the
/// proxy.
pub(crate) struct World {
    pub(crate) ca: Ca,
    /// Kept so the proxy's only origin stays up.
    _origin: TlsServer,
    pub(crate) proxy: StandInProxy,
    pub(crate) dir: Dir,
}

pub(crate) fn world(name: &str, names: &[&str], body: &'static [u8]) -> World {
    let ca = Ca::new("Test CA");
    let server = ca.issue_server(names);
    let origin = TlsServer::start(
        &server,
        &[],
        ClientAuth::Off,
        Arc::new(move |path, _| {
            if path.starts_with("/v0.1/servers") {
                br#"{"servers":[]}"#.to_vec()
            } else {
                body.to_vec()
            }
        }),
    );
    let proxy = StandInProxy::start(USER, PASSWORD, Some(origin.addr));
    World {
        ca,
        _origin: origin,
        proxy,
        dir: Dir::new(name),
    }
}

/// The settings that send everything through the world's proxy with a login, trusting only its CA.
pub(crate) fn settings(world: &World, password: &str) -> NetworkSettings {
    let ca_file = world.dir.0.join("ca.pem");
    fs::write(&ca_file, world.ca.pem()).expect("writes");
    let mut settings = NetworkSettings::default();
    settings.proxy = ProxyChoice::Proxy(format!("http://{}", world.proxy.addr));
    settings.proxy_user = Some(USER.to_owned());
    settings.proxy_password = Some(Secret::new(password));
    settings.ca_file = Some(ca_file);
    settings
}

/// The settings' network.
pub(crate) fn through(world: &World) -> Arc<Network> {
    Arc::new(Network::new(&settings(world, PASSWORD)).expect("a network"))
}

fn sha(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(&Sha256::digest(bytes))
}

#[test]
fn a_store_download_goes_through_the_proxy_with_its_login() {
    let body = b"program bytes";
    let world = world("download", &["downloads.example.test"], body);
    let tools = LocalTools::new(
        Arc::new(HttpTransport::on(through(&world))),
        vec!["https://downloads.example.test".to_owned()],
    )
    .granted(&world.dir.0);
    let fetched = tools
        .fetch(FetchRequest::new(
            "https://downloads.example.test/f",
            "out.bin",
            Check::Sha256 { value: sha(body) },
            1000,
        ))
        .expect("fetched through the proxy");
    assert_eq!(fetched.sha256, sha(body));
    assert_eq!(fs::read(world.dir.0.join("out.bin")).expect("kept"), body);
    assert_eq!(
        world.proxy.allowed(USER, PASSWORD),
        ["downloads.example.test:443"]
    );
}

#[test]
fn a_proxy_that_refuses_the_login_gets_no_download() {
    let world = world("refused", &["downloads.example.test"], b"never");
    let network = Arc::new(Network::new(&settings(&world, "wrong")).expect("a network"));
    let tools = LocalTools::new(
        Arc::new(HttpTransport::on(network)),
        vec!["https://downloads.example.test".to_owned()],
    )
    .granted(&world.dir.0);
    let failed = tools
        .fetch(FetchRequest::new(
            "https://downloads.example.test/f",
            "out.bin",
            Check::Sha256 {
                value: sha(b"never"),
            },
            1000,
        ))
        .expect_err("the proxy asked for a login the settings do not have");
    assert!(!format!("{failed:?}").is_empty());
    assert!(world.proxy.allowed(USER, PASSWORD).is_empty());
    assert!(!world.dir.0.join("out.bin").exists());
}

#[test]
fn no_proxy_replaces_the_system_list_and_sends_a_host_direct() {
    // The proxy has no origin: a request that reached it would be refused, so a direct one is the only way to an answer.
    let ca = Ca::new("Test CA");
    let server = ca.issue_server(&["localhost"]);
    let origin = TlsServer::start(
        &server,
        &[],
        ClientAuth::Off,
        Arc::new(|_, _| b"direct".to_vec()),
    );
    let proxy = StandInProxy::start(USER, PASSWORD, None);
    let dir = Dir::new("direct");
    let ca_file = dir.0.join("ca.pem");
    fs::write(&ca_file, ca.pem()).expect("writes");
    let mut settings = NetworkSettings::default();
    settings.proxy = ProxyChoice::Proxy(format!("http://{USER}:{PASSWORD}@{}", proxy.addr));
    settings.ca_file = Some(ca_file);
    // Loopback always goes direct unless the list says `<-loopback>`; a list that names the host is the override under test.
    settings.no_proxy = Some(vec!["localhost".to_owned()]);
    let network = Network::new(&settings).expect("a network");
    let reply = HttpTransport::on(Arc::new(network))
        .get(
            &format!("https://localhost:{}/x", origin.addr.port()),
            &ToolsLimits::default(),
        )
        .expect("direct");
    assert_eq!(body(reply), "direct");
    assert!(proxy.seen().is_empty());
}

fn body(mut reply: crate::store::tools::Reply) -> String {
    let mut text = String::new();
    reply.body.read_to_string(&mut text).expect("reads");
    text
}

/// A server that asks for a client certificate and answers with whether it got one.
fn asking(ca: &Ca) -> (TlsServer, Leaf) {
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
    (origin, ca.issue_client("Alice Example"))
}

pub(crate) fn direct() -> Resolver {
    Resolver::builder()
        .environment(Environment::from_pairs([("PATH", "x")]))
        .system(false)
        .build()
}

#[test]
fn a_fetch_presents_the_certificate_the_settings_name() {
    let ca = Ca::new("Test CA");
    let (origin, client) = asking(&ca);
    let dir = Dir::new("mtls");
    let ca_file = dir.0.join("ca.pem");
    fs::write(&ca_file, ca.pem()).expect("writes");
    let identity_file = dir.0.join("client.pem");
    fs::write(
        &identity_file,
        format!("{}{}", client.chain_pem, client.key_pem),
    )
    .expect("writes");

    let url = format!("https://localhost:{}/x", origin.addr.port());
    let mut settings = NetworkSettings::default();
    settings.ca_file = Some(ca_file);
    settings.client_certificate = ClientCertificate::File(identity_file);
    let network = Arc::new(Network::new(&settings).expect("a network"));
    let reply = HttpTransport::on(network)
        .get(&url, &ToolsLimits::default())
        .expect("answered");
    assert_eq!(body(reply), "client certificate seen");
    assert_eq!(origin.presented().len(), 1);
    assert!(origin.presented()[0].is_some());

    // With none sent, the server that insists refuses the handshake: the override is what made the difference.
    settings.client_certificate = ClientCertificate::None;
    let without = Arc::new(Network::new(&settings).expect("a network"));
    assert!(
        HttpTransport::on(without)
            .get(&url, &ToolsLimits::default())
            .is_err()
    );
}

/// A keystore holding one identity: the stand-in for the operating system's.
#[derive(Debug)]
pub(crate) struct Keystore(pub(crate) Identity);

impl IdentitySource for Keystore {
    fn identities(&self) -> Result<Vec<Identity>, coqui_syshttp::tls::Error> {
        Ok(vec![self.0.clone()])
    }
}

#[test]
fn a_fetch_presents_the_keystores_certificate_the_server_names_a_ca_for() {
    let ca = Ca::new("Test CA");
    let (origin, client) = asking(&ca);
    let identity = Identity::from_pem(client.chain_pem.as_bytes(), client.key_pem.as_bytes())
        .expect("an identity");
    let selector = Selector::builder()
        .source(Keystore(identity))
        .send_sole_candidate(true)
        .build();
    let tls = TlsBuilder::default()
        .trust(Trust::Roots(vec![ca.der()]))
        .client_certificates(ClientCertificates::Selector(Arc::new(selector)));
    let network = Arc::new(Network::with(direct(), tls));
    let reply = HttpTransport::on(network)
        .get(
            &format!("https://localhost:{}/x", origin.addr.port()),
            &ToolsLimits::default(),
        )
        .expect("answered");
    assert_eq!(body(reply), "client certificate seen");
    assert_eq!(
        origin.presented()[0].as_deref(),
        Some(client.chain[0].as_ref())
    );
}

/// What a program started with `command` says its environment is.
pub(crate) fn environment_of(command: &mut Command) -> String {
    let output = command.output().expect("runs");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub(crate) fn printing_environment() -> Command {
    let mut command = if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.args(["/c", "set"]);
        command
    } else {
        let mut command = Command::new("sh");
        command.args(["-c", "env"]);
        command
    };
    command.env_clear();
    if cfg!(windows) {
        command.env(
            "SystemRoot",
            std::env::var("SystemRoot").unwrap_or_default(),
        );
    }
    command.env("PATH", std::env::var_os("PATH").unwrap_or_default());
    command
}

pub(crate) fn proxied(dir: &Path) -> Arc<Network> {
    let _ = dir;
    let mut settings = NetworkSettings::default();
    settings.proxy = ProxyChoice::Proxy("http://carol:pw@127.0.0.1:9".to_owned());
    Arc::new(Network::new(&settings).expect("a network"))
}

#[test]
fn a_program_started_with_no_proxy_of_its_own_is_given_the_network_s() {
    let dir = Dir::new("children");
    let mut command = printing_environment();
    command.network(proxied(&dir.0));
    let seen = environment_of(&mut command);
    for name in ["HTTP_PROXY", "HTTPS_PROXY"] {
        assert!(
            seen.lines()
                .any(|line| line == format!("{name}=http://carol:pw@127.0.0.1:9")),
            "{name} in {seen}"
        );
    }
    assert!(
        seen.lines()
            .any(|line| line == "NO_PROXY=localhost,127.0.0.1,::1"),
        "{seen}"
    );
}

#[test]
fn a_program_whose_environment_says_no_proxy_for_everything_is_given_no_proxy() {
    let dir = Dir::new("children-direct");
    let mut command = printing_environment();
    command.env("NO_PROXY", "*");
    command.network(proxied(&dir.0));
    let seen = environment_of(&mut command);
    assert!(!seen.to_ascii_lowercase().contains("_proxy=http"), "{seen}");
    assert!(!seen.contains("carol"), "{seen}");
    assert!(
        seen.lines()
            .any(|line| line.eq_ignore_ascii_case("NO_PROXY=*")),
        "{seen}"
    );
}

#[test]
fn a_program_whose_environment_names_a_proxy_keeps_it() {
    let dir = Dir::new("children-own");
    let mut command = printing_environment();
    command.env("HTTPS_PROXY", "http://mine:1");
    command.network(proxied(&dir.0));
    let seen = environment_of(&mut command);
    assert!(
        seen.lines().any(|line| line == "HTTPS_PROXY=http://mine:1"),
        "{seen}"
    );
    assert!(!seen.contains("carol"), "{seen}");
}

/// Where names are case-sensitive, a program that reads the lower-case proxy variable first goes direct when the person left it
/// empty, so the host's proxy has to replace it, not sit beside it.
#[test]
#[cfg(unix)]
fn a_proxy_variable_the_person_left_empty_does_not_shadow_the_one_the_host_adds() {
    let dir = Dir::new("children-empty");
    let mut command = printing_environment();
    command
        .env("https_proxy", "")
        .env("http_proxy", "")
        .env("no_proxy", "");
    command.network(proxied(&dir.0));
    let seen = environment_of(&mut command);
    assert!(
        seen.lines()
            .any(|line| line == "HTTPS_PROXY=http://carol:pw@127.0.0.1:9"),
        "{seen}"
    );
    for name in ["https_proxy", "http_proxy", "no_proxy"] {
        assert!(
            !seen
                .lines()
                .any(|line| line.starts_with(&format!("{name}="))),
            "{name} in {seen}"
        );
    }
    assert!(
        seen.lines()
            .any(|line| line == "NO_PROXY=localhost,127.0.0.1,::1"),
        "{seen}"
    );
}

#[test]
fn a_proxy_that_names_a_pac_script_is_asked_about_the_endpoints_a_program_reaches() {
    let dir = Dir::new("children-pac");
    let script = dir.0.join("proxy.pac");
    fs::write(
        &script,
        "function FindProxyForURL(url, host) { if (host == 'direct.example') return 'DIRECT'; return 'PROXY gateway.example:3128'; }",
    )
    .expect("writes");
    let mut settings = NetworkSettings::default();
    settings.proxy = ProxyChoice::Pac(script.to_string_lossy().into_owned());
    // The script runs on a worker with a time limit, and one that cannot be loaded in time is no opinion, so a machine busy
    // with other builds can answer nothing once and the network then keeps to it for half a minute: ask a fresh network again.
    let urls = [
        "https://api.example/".to_owned(),
        "https://direct.example/".to_owned(),
    ];
    let mut variables = Vec::new();
    for _ in 0..4 {
        let network = Network::new(&settings).expect("a network");
        variables = network.variables_for(&urls);
        if !variables.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    let value = |name: &str| {
        variables
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    assert_eq!(value("HTTPS_PROXY"), Some("http://gateway.example:3128"));
    assert!(value("NO_PROXY").is_some_and(|list| list.contains("direct.example")));
}

/// An agent no crate of Canoa's names, whose library says where its program goes.
mod standin_vendor {
    use canoa::vendor::{AgentVendor, VendorInfo};
    use coqui_acp_api::guatiao::Provider;
    use coqui_acp_api::guatiao::library::ProviderError;

    pub(super) const ADDRESS: &str = "https://api.zed-agent.test/";

    #[derive(Default, Debug, Provider)]
    #[provider(AgentVendor, id = "zed_endpoints_vendor", crate = ::coqui_acp_api::guatiao)]
    pub(super) struct ZedVendor;

    impl AgentVendor for ZedVendor {
        fn describe(&self) -> Result<VendorInfo, ProviderError> {
            let mut info = VendorInfo::new("zedendpoints", "Zed");
            info.endpoints = vec![ADDRESS.to_owned()];
            Ok(info)
        }
    }

    coqui_acp_api::guatiao::local_providers!(
        id = "zed-endpoints",
        version = "0.0.0",
        providers = [ZedVendor]
    );
}

#[test]
fn the_addresses_programs_reach_are_what_the_libraries_loaded_say() {
    use coqui_acp_api::guatiao::library::Registry;

    let before = super::program_endpoints();
    // What the bundled libraries and built-in stores say is there, each from its own description.
    for named in [
        "https://api.anthropic.com/",
        "https://api.openai.com/",
        "https://api.x.ai/",
        "https://registry.npmjs.org/",
        "https://pypi.org/",
        "https://nodejs.org/dist/",
    ] {
        assert!(before.iter().any(|url| url == named), "{named}: {before:?}");
    }
    // An agent a library adds says its own, and only once it is loaded.
    assert!(!before.iter().any(|url| url == standin_vendor::ADDRESS));
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    let loading = registry
        .register_local("zed-endpoints", standin_vendor::library)
        .expect("the library registers");
    assert!(loading.loaded().is_some());
    crate::vendors::note(&registry);
    let after = super::program_endpoints();
    assert!(after.iter().any(|url| url == standin_vendor::ADDRESS));
    crate::vendors::note(&registry);
    let again = super::program_endpoints();
    assert_eq!(
        again
            .iter()
            .filter(|url| *url == standin_vendor::ADDRESS)
            .count(),
        1,
        "an address already there is not added again"
    );
}

fn fetch_with(selector: Selector, ca: &Ca, origin: &TlsServer) -> Result<String, ()> {
    let tls = TlsBuilder::default()
        .trust(Trust::Roots(vec![ca.der()]))
        .client_certificates(ClientCertificates::Selector(Arc::new(selector)));
    HttpTransport::on(Arc::new(Network::with(direct(), tls)))
        .get(
            &format!("https://localhost:{}/x", origin.addr.port()),
            &ToolsLimits::default(),
        )
        .map(body)
        .map_err(|_| ())
}

#[test]
fn the_keystores_only_certificate_is_not_sent_unless_a_rule_or_policy_picks_it() {
    let ca = Ca::new("Test CA");
    let (origin, client) = asking(&ca);
    let identity = Identity::from_pem(client.chain_pem.as_bytes(), client.key_pem.as_bytes())
        .expect("an identity");
    let unasked = Selector::builder()
        .source(Keystore(identity.clone()))
        .build();
    assert!(fetch_with(unasked, &ca, &origin).is_err());
    assert!(origin.presented().iter().all(Option::is_none));
    let by_policy = Selector::builder()
        .source(Keystore(identity))
        .policy(Arc::new(|_| CertificateChoice::First))
        .build();
    assert_eq!(
        fetch_with(by_policy, &ca, &origin).as_deref(),
        Ok("client certificate seen")
    );
}
