//! The routes of the host over loopback stand-ins: a proxy that wants a login and a TLS server behind it. No test installs the
//! process's network: each makes its own state from a configuration and opens leases in it.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::fs;
use std::sync::Arc;

use super::*;
use crate::network::standins::{Ca, ClientAuth, StandInProxy, TlsServer};
use crate::network::tests::{Dir, USER};
use crate::store::tools::{HttpTransport, ToolsLimits, Transport};

/// The name the server answers for.
const NAME: &str = "origin.example.test";

/// The password the stand-in proxy wants: the value of a variable every test process has, so a route can name it as its password's
/// variable without any test setting an environment variable.
fn password() -> String {
    std::env::var("CARGO_PKG_NAME").expect("cargo sets it")
}

fn config(dir: &Dir, text: &str) -> Config {
    Config::from_str_in(text, &dir.0).expect("parses")
}

fn state(dir: &Dir, text: &str) -> State {
    State::of(&config(dir, text))
}

/// A server that answers for [`NAME`], and the proxy that is its only way out: loopback is never sent through a proxy, so the name
/// is one the proxy alone resolves.
struct Behind {
    _origin: TlsServer,
    proxy: StandInProxy,
    dir: Dir,
    ca_file: String,
}

fn behind(name: &str) -> Behind {
    let ca = Ca::new("Test CA");
    let server = ca.issue_server(&[NAME]);
    let origin = TlsServer::start(
        &server,
        &[],
        ClientAuth::Off,
        Arc::new(|_, _| b"hello".to_vec()),
    );
    let proxy = StandInProxy::start(USER, &password(), Some(origin.addr));
    let dir = Dir::new(name);
    let ca_file = dir.0.join("ca.pem");
    fs::write(&ca_file, ca.pem()).expect("writes");
    Behind {
        _origin: origin,
        proxy,
        dir,
        ca_file: ca_file.to_string_lossy().replace('\\', "/"),
    }
}

fn get(network: Arc<Network>) -> Result<String, String> {
    use std::io::Read as _;
    let mut reply = HttpTransport::on(network)
        .get(&format!("https://{NAME}/x"), &ToolsLimits::default())
        .map_err(|error| format!("{error:?}"))?;
    let mut text = String::new();
    reply
        .body
        .read_to_string(&mut text)
        .map_err(|error| error.to_string())?;
    Ok(text)
}

fn corp(world: &Behind) -> String {
    format!(
        "[network.routes.corp]\nprovider = \"manual\"\n\
         settings = {{ all_proxy = \"http://{}\", no_proxy = [], user = \"{USER}\", password_env = \"CARGO_PKG_NAME\", ca_file = \"{}\" }}\n",
        world.proxy.addr, world.ca_file
    )
}

#[test]
fn the_host_carries_every_built_in_route_and_names_them() {
    let ids: Vec<String> = RouteCatalog::built_in()
        .list()
        .into_iter()
        .map(|each| each.id)
        .collect();
    let mut ids = ids;
    ids.sort();
    assert_eq!(ids, ["command", "direct", "manual", "pac", "system"]);
    let dir = Dir::new("names");
    assert_eq!(
        names(&config(&dir, "[network.routes.a]\nprovider = \"manual\"\n")),
        ["system", "direct", "a"]
    );
}

#[test]
fn a_manual_route_sends_a_request_through_its_proxy_with_its_login() {
    let world = behind("manual");
    let state = state(&world.dir, &corp(&world));
    let held = held(&state, "corp", "host").expect("opens");
    assert_eq!(get(Arc::clone(&held.network)), Ok("hello".to_owned()));
    assert_eq!(
        world.proxy.allowed(USER, &password()),
        [format!("{NAME}:443")]
    );
}

#[test]
fn the_host_and_an_account_leave_by_their_own_routes() {
    let world = behind("account");
    let text = format!(
        "{}[host.accounts.work]\nkind = \"k\"\nid = \"id-1\"\nroute = \"corp\"\n[host.accounts.plain]\nkind = \"k\"\nid = \"id-2\"\n",
        corp(&world)
    );
    let state = state(&world.dir, &text);
    // The host's route is the system's: it has no proxy of its own, so a request of its own would not be sent to this one.
    assert_eq!(state.default, "system");
    let account = account_network_in(&state, "id-1")
        .expect("opens")
        .expect("the account names a route");
    assert_eq!(get(account), Ok("hello".to_owned()));
    assert_eq!(world.proxy.allowed(USER, &password()).len(), 1);
    // An account that names none leaves by the host's route.
    assert!(account_network_in(&state, "id-2").expect("none").is_none());
}

#[test]
fn routes_with_equal_settings_share_a_lease_and_others_do_not() {
    let world = behind("share");
    let text = format!(
        "{}[network.routes.twin]\nprovider = \"manual\"\n\
         settings = {{ all_proxy = \"http://{}\", no_proxy = [], user = \"{USER}\", password_env = \"CARGO_PKG_NAME\", ca_file = \"{}\" }}\n\
         [network.routes.other]\nprovider = \"manual\"\nsettings = {{ all_proxy = \"http://127.0.0.1:9\" }}\n",
        corp(&world),
        world.proxy.addr,
        world.ca_file
    );
    let state = state(&world.dir, &text);
    let first = held(&state, "corp", "host").expect("opens");
    let again = held(&state, "corp", "account:a").expect("opens");
    let twin = held(&state, "twin", "session:1").expect("opens");
    let other = held(&state, "other", "host").expect("opens");
    assert!(Arc::ptr_eq(&first, &again));
    assert!(Arc::ptr_eq(&first, &twin));
    assert!(!Arc::ptr_eq(&first, &other));
}

#[test]
fn a_route_the_host_lacks_or_a_provider_refuses_fails_in_words() {
    let dir = Dir::new("failures");
    let state = state(
        &dir,
        "[network.routes.empty]\nprovider = \"manual\"\n\
         [network.routes.lost]\nprovider = \"manual\"\nsettings = { all_proxy = \"p:1\", user = \"u\", password_env = \"CANOA_NO_SUCH_VARIABLE_FOR_TESTS\" }\n\
         [network.routes.nothing]\nprovider = \"no-such-provider\"\n",
    );
    let ghost = held(&state, "ghost", "host")
        .map(|_| ())
        .expect_err("no such route");
    assert_eq!(ghost.kind, FailureKind::Settings);
    assert!(ghost.to_string().contains("ghost"));
    let empty = held(&state, "empty", "host")
        .map(|_| ())
        .expect_err("names no proxy");
    assert!(empty.why.contains("name a proxy"), "{}", empty.why);
    let lost = held(&state, "lost", "host")
        .map(|_| ())
        .expect_err("the variable is not set");
    assert!(
        lost.why.contains("CANOA_NO_SUCH_VARIABLE_FOR_TESTS"),
        "{}",
        lost.why
    );
    assert!(!lost.why.contains("password is"), "{}", lost.why);
    let nothing = held(&state, "nothing", "host")
        .map(|_| ())
        .expect_err("no provider");
    assert!(nothing.why.contains("no-such-provider"), "{}", nothing.why);
}

#[test]
fn the_direct_route_goes_straight_and_the_system_route_follows_the_system() {
    let dir = Dir::new("builtins");
    let state = state(&dir, "");
    let direct = held(&state, "direct", "host").expect("opens");
    assert!(direct.network.goes_direct("https://example.test/"));
    assert_eq!(direct.status.description, "with no proxy");
    let system = held(&state, "system", "host").expect("opens");
    assert_eq!(system.status.description, "following the system");
}

#[test]
fn a_pac_script_chooses_the_proxy_of_a_route() {
    let world = behind("pac");
    let script = world.dir.0.join("proxy.pac");
    fs::write(
        &script,
        format!(
            "function FindProxyForURL(url, host) {{ return \"PROXY {}\"; }}\n",
            world.proxy.addr
        ),
    )
    .expect("writes");
    let text = format!(
        "[network.routes.script]\nprovider = \"pac\"\nsettings = {{ location = \"proxy.pac\", no_proxy = [], user = \"{USER}\", password_env = \"CARGO_PKG_NAME\", ca_file = \"{}\" }}\n",
        world.ca_file
    );
    let state = state(&world.dir, &text);
    let held = held(&state, "script", "host").expect("opens");
    assert_eq!(get(Arc::clone(&held.network)), Ok("hello".to_owned()));
    assert!(!world.proxy.allowed(USER, &password()).is_empty());
}

#[test]
fn a_manual_route_judges_its_settings_with_nothing_started() {
    let catalog = RouteCatalog::built_in();
    let dir = Dir::new("judging");
    let config = config(&dir, "");
    let json = |text: &str| serde_json::from_str::<Value>(text).expect("JSON");
    assert!(problems_of(&catalog, &config, "manual", &json(r#"{"all_proxy":"p:1"}"#)).is_empty());
    assert!(!problems_of(&catalog, &config, "manual", &json("{}")).is_empty());
    assert!(
        !problems_of(
            &catalog,
            &config,
            "manual",
            &json(r#"{"all_proxy":"p:1","ca_file":"missing.pem"}"#)
        )
        .is_empty()
    );
    assert!(
        !problems_of(
            &catalog,
            &config,
            "manual",
            &json(r#"{"all_proxy":"p:1","other":1}"#)
        )
        .is_empty()
    );
    assert!(!problems_of(&catalog, &config, "pac", &json("{}")).is_empty());
    assert!(!problems_of(&catalog, &config, "command", &json("{}")).is_empty());
}
