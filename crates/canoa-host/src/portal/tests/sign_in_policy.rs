//! The sign-in policy through the portal's own request handling: what the page offers, a method refused before any provider is
//! asked, the decision made again when a credential names someone and when a session is used, and the shared flows run against
//! stand-in providers.

use coqui_access::guatiao::library::ProviderError;
use coqui_access::{SignInContext, SignInDecision, SignInPolicy};
use coqui_access_signin::vectors::FLOWS;

use super::*;

/// The rule docs/portal.md shows: a password only from 10.0.0.0/8, a certificate from anywhere.
const NETWORK_RULE: &str = r#"
[[sign_in_policy.config.rule]]
networks = ["10.0.0.0/8"]
allow = [["password"], ["certificate"]]

[[sign_in_policy.config.rule]]
allow = [["certificate"]]
"#;

fn network_rule() -> Option<Entry> {
    Settings::from_toml(NETWORK_RULE).unwrap().sign_in_policy
}

fn under_the_network_rule() -> Rig {
    rig(|settings| settings.sign_in_policy = network_rule(), 0)
}

fn proxied(rig: &Rig<impl Sized>) -> To {
    To::Unix(rig.sock.clone())
}

/// The methods `GET /portal/methods` answers a browser the proxy says is at `forwarded` (none: the proxy said nothing).
fn methods_at(rig: &Rig<impl Sized>, forwarded: Option<&str>) -> Vec<String> {
    let extra: Vec<(&str, String)> = forwarded
        .map(|address| ("X-Forwarded-For", address.to_owned()))
        .into_iter()
        .collect();
    let answer = request(&proxied(rig), "GET", METHODS, &extra, "");
    assert_eq!(answer.status, 200);
    let list: serde_json::Value = serde_json::from_str(&answer.body).unwrap();
    list["methods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|word| word.as_str().unwrap().to_owned())
        .collect()
}

/// A sign-in form posted from the proxy for a browser at `forwarded`.
fn form_at(
    rig: &Rig<impl Sized>,
    forwarded: Option<&str>,
    certificate: Option<&str>,
    body: &str,
) -> Answer {
    let to = proxied(rig);
    let mut extra = vec![
        ("Origin", to.origin()),
        (
            "Content-Type",
            "application/x-www-form-urlencoded".to_owned(),
        ),
    ];
    if let Some(address) = forwarded {
        extra.push(("X-Forwarded-For", address.to_owned()));
    }
    if let Some(cert) = certificate {
        extra.push(("X-Client-Cert", cert.to_owned()));
    }
    request(&to, "POST", LOGIN, &extra, body)
}

/// The person's host index, which is where a session is decided again, for a browser at `forwarded`.
fn index_at(rig: &Rig<impl Sized>, cookie: &str, forwarded: Option<&str>) -> Answer {
    let mut extra = vec![("Cookie", cookie.to_owned())];
    if let Some(address) = forwarded {
        extra.push(("X-Forwarded-For", address.to_owned()));
    }
    request(&proxied(rig), "GET", web::HOST_INDEX, &extra, "")
}

#[test]
fn the_page_and_the_methods_offer_what_the_network_rule_allows_from_where_the_browser_is() {
    let rig = under_the_network_rule();
    assert_eq!(
        methods_at(&rig, Some("10.1.2.3")),
        ["password", "certificate"]
    );
    for outside in ["203.0.113.7", "2001:db8::1", "11.0.0.1"] {
        assert_eq!(
            methods_at(&rig, Some(outside)),
            ["certificate"],
            "{outside}"
        );
    }
    // An address that is not known never matches a network, and an IPv4 address in IPv6 dress is the IPv4 address.
    assert_eq!(methods_at(&rig, None), ["certificate"]);
    assert_eq!(methods_at(&rig, Some("not an address")), ["certificate"]);
    assert_eq!(
        methods_at(&rig, Some("::ffff:10.1.2.3")),
        ["password", "certificate"]
    );
    let page = |forwarded: &str| {
        request(
            &proxied(&rig),
            "GET",
            LOGIN,
            &[("X-Forwarded-For", forwarded.to_owned())],
            "",
        )
        .body
    };
    let inside = page("10.1.2.3");
    assert!(
        inside.contains("name=\"password\"") && inside.contains("Sign in with your certificate")
    );
    let outside = page("203.0.113.7");
    assert!(
        !outside.contains("name=\"password\"") && outside.contains("Sign in with your certificate")
    );
}

#[test]
fn the_address_is_the_one_the_trusted_proxy_reports_and_unknown_when_it_reports_none() {
    // The proxy itself is inside 10/8: if the chain drops the browser's address, nobody is inside because of it.
    let rig = rig(
        |settings| {
            settings.sign_in_policy = network_rule();
            settings.trusted_proxies = vec!["10.0.0.1".parse().unwrap()];
            settings.client_address_header = "x-real-ip".to_owned();
        },
        0,
    );
    let at = |header: Option<&str>| {
        let extra: Vec<(&str, String)> = header
            .map(|value| ("X-Real-IP", value.to_owned()))
            .into_iter()
            .collect();
        let answer = request(&proxied(&rig), "GET", METHODS, &extra, "");
        let list: serde_json::Value = serde_json::from_str(&answer.body).unwrap();
        list["methods"].to_string()
    };
    assert_eq!(at(Some("10.1.2.3")), r#"["password","certificate"]"#);
    assert_eq!(at(Some("203.0.113.7")), r#"["certificate"]"#);
    assert_eq!(at(Some("10.0.0.1")), r#"["certificate"]"#);
    assert_eq!(at(None), r#"["certificate"]"#);
    // A header the browser sent on the loopback address is nobody's word: that connection's own peer is the address.
    let local = call(
        rig.addr,
        "GET",
        METHODS,
        &[("X-Real-IP", "10.1.2.3".to_owned())],
        "",
    );
    let list: serde_json::Value = serde_json::from_str(&local.body).unwrap();
    assert_eq!(list["methods"].to_string(), "[]");
}

#[test]
fn a_password_from_outside_is_refused_before_any_authenticator_and_counts_against_no_one() {
    let rig = rig(
        |settings| {
            settings.sign_in_policy = network_rule();
            settings.max_failures_per_pair = 1;
            settings.max_failures_per_address = 1;
        },
        0,
    );
    let outside: IpAddr = "203.0.113.7".parse().unwrap();
    for body in [
        "user=canoatesta&password=pw-a",
        "user=canoatesta&password=wrong",
        "method=password&user=nobody&password=x",
    ] {
        for _ in 0..3 {
            let answer = form_at(&rig, Some("203.0.113.7"), None, body);
            assert_eq!(answer.status, 403, "{body}");
            assert!(answer.cookie().is_none());
        }
    }
    assert_eq!(rig.accounts.asked.load(Ordering::SeqCst), 0);
    assert_eq!(rig.inner.throttle.failures_from(outside), 0);
    // Nothing the address did counts toward a limit: a certificate from it still gets in, and so does a password from inside.
    let cert = form_at(
        &rig,
        Some("203.0.113.7"),
        Some("MIIB-test-cert"),
        "method=certificate",
    );
    assert_eq!(cert.status, 303);
    let inside = form_at(
        &rig,
        Some("10.1.2.3"),
        None,
        "user=canoatesta&password=pw-a",
    );
    assert_eq!((inside.status, inside.header("location")), (303, Some("/")));
}

#[test]
fn a_certificate_signs_in_from_anywhere_under_the_network_rule() {
    let rig = under_the_network_rule();
    for from in [
        Some("203.0.113.7"),
        Some("10.1.2.3"),
        Some("2001:db8::1"),
        None,
    ] {
        let answer = form_at(&rig, from, Some("MIIB-test-cert"), "method=certificate");
        assert_eq!(
            (answer.status, answer.header("location")),
            (303, Some("/")),
            "{from:?}"
        );
        assert!(answer.cookie().is_some());
    }
}

#[test]
fn a_session_made_by_password_inside_ends_at_its_next_use_from_outside() {
    let rig = under_the_network_rule();
    let made = form_at(
        &rig,
        Some("10.1.2.3"),
        None,
        "user=canoatesta&password=pw-a",
    );
    let cookie = made.cookie().expect("a session");
    assert_eq!(index_at(&rig, &cookie, Some("10.1.2.3")).status, 200);
    let moved = index_at(&rig, &cookie, Some("203.0.113.7"));
    assert_eq!(moved.status, 401);
    // The session is gone, not paused: back inside it does not return.
    assert_eq!(index_at(&rig, &cookie, Some("10.1.2.3")).status, 401);
    let again = form_at(
        &rig,
        Some("10.1.2.3"),
        None,
        "user=canoatesta&password=pw-a",
    );
    assert_eq!(again.status, 303);
    // An address that is not known ends it too.
    let cookie = again.cookie().unwrap();
    assert_eq!(index_at(&rig, &cookie, None).status, 401);
}

#[test]
fn a_session_made_by_certificate_stands_from_anywhere_and_so_do_the_pair_and_the_channels() {
    let rig = under_the_network_rule();
    let made = form_at(
        &rig,
        Some("203.0.113.7"),
        Some("MIIB-test-cert"),
        "method=certificate",
    );
    let cookie = made.cookie().expect("a session");
    for from in [Some("203.0.113.7"), Some("10.1.2.3"), None] {
        assert_eq!(index_at(&rig, &cookie, from).status, 200, "{from:?}");
    }
}

#[test]
fn the_pair_and_a_channel_decide_the_session_again_too() {
    let rig = under_the_network_rule();
    let cookie = form_at(
        &rig,
        Some("10.1.2.3"),
        None,
        "user=canoatesta&password=pw-a",
    )
    .cookie()
    .unwrap();
    let to = proxied(&rig);
    let pair = |from: &str| {
        request(
            &to,
            "POST",
            "/portal/pair",
            &[
                ("Origin", to.origin()),
                ("Cookie", cookie.clone()),
                ("X-Forwarded-For", from.to_owned()),
                ("Content-Type", "application/json".to_owned()),
            ],
            r#"{"peer":"sha256:browser"}"#,
        )
    };
    assert_eq!(pair("10.1.2.3").status, 200);
    assert_eq!(pair("203.0.113.7").status, 401);
    // It ended the session: a channel from inside has none to open on.
    let again = form_at(
        &rig,
        Some("10.1.2.3"),
        None,
        "user=canoatesta&password=pw-a",
    )
    .cookie()
    .unwrap();
    let Err(status) = channel_from(&rig, &again, "203.0.113.7") else {
        panic!("a channel opened from where the session is not accepted");
    };
    assert_eq!(status, 401);
    assert_eq!(index_at(&rig, &again, Some("10.1.2.3")).status, 401);
}

/// Opens the control channel with `cookie` for a browser at `forwarded`; the status of a refusal.
fn channel_from(rig: &Rig, cookie: &str, forwarded: &str) -> Result<(), u16> {
    let to = proxied(rig);
    let answer = request(
        &to,
        "GET",
        "/portal/ws/control",
        &[
            ("Origin", to.origin()),
            ("Cookie", cookie.to_owned()),
            ("X-Forwarded-For", forwarded.to_owned()),
            ("Connection", "Upgrade".to_owned()),
            ("Upgrade", "websocket".to_owned()),
            ("Sec-WebSocket-Version", "13".to_owned()),
            ("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==".to_owned()),
        ],
        "",
    );
    if answer.status == 101 {
        Ok(())
    } else {
        Err(answer.status)
    }
}

#[test]
fn a_password_typed_again_to_pin_a_certificate_is_a_password_sent_and_the_policy_decides_it() {
    let rig = under_the_network_rule();
    // Signed in by certificate from outside, as the network rule allows.
    let cookie = form_at(
        &rig,
        Some("203.0.113.7"),
        Some("MIIB-test-cert"),
        "method=certificate",
    )
    .cookie()
    .unwrap();
    let to = proxied(&rig);
    let enrol = |from: &str| {
        request(
            &to,
            "POST",
            CERTIFICATE,
            &[
                ("Origin", to.origin()),
                ("Cookie", cookie.clone()),
                ("X-Forwarded-For", from.to_owned()),
                ("X-Client-Cert", "MIIB-new".to_owned()),
                (
                    "Content-Type",
                    "application/x-www-form-urlencoded".to_owned(),
                ),
            ],
            "action=enrol&password=pw-a",
        )
    };
    let before = rig.accounts.asked.load(Ordering::SeqCst);
    let refused = enrol("203.0.113.7");
    assert!(
        refused
            .body
            .contains("A password is not accepted from here.")
    );
    assert_eq!(
        rig.accounts.asked.load(Ordering::SeqCst),
        before,
        "no provider was asked"
    );
    assert!(
        rig.accounts
            .certificates
            .lock()
            .unwrap()
            .get("MIIB-new")
            .is_none()
    );
    // From inside the same password is accepted.
    let done = enrol("10.1.2.3");
    assert!(done.body.contains("now signs you in"), "{}", done.body);
}

/// A policy that cannot find out.
struct Unreachable;

impl SignInPolicy for Unreachable {
    fn decide(&self, _context: SignInContext) -> Result<SignInDecision, ProviderError> {
        Err(coqui_access::error::internal("the risk service is down"))
    }
}

#[test]
fn a_policy_that_cannot_decide_offers_nothing_and_asks_nobody() {
    let policy = policy::Policy::from_provider(Arc::new(Unreachable)).unwrap();
    let rig = rig_policy(
        Arc::new(Fixed {
            accounts: Mutex::new(
                [("canoatesta".to_owned(), ("pw-a".to_owned(), mine()))]
                    .into_iter()
                    .collect(),
            ),
            gone: Mutex::new(HashSet::new()),
            unsure: Mutex::new(HashSet::new()),
            asked: AtomicUsize::new(0),
            certificates: Mutex::new(HashMap::new()),
            last_client: Mutex::new(None),
            presented: Mutex::new(Vec::new()),
        }),
        |_| {},
        0,
        Some(policy),
    );
    let methods = request(&proxied(&rig), "GET", METHODS, &[], "");
    assert_eq!(methods.status, 503);
    assert!(!methods.body.contains("password"));
    let page = request(&proxied(&rig), "GET", LOGIN, &[], "");
    assert_eq!(page.status, 503);
    assert!(!page.body.contains("name=\"password\""));
    let answer = form_at(
        &rig,
        Some("10.1.2.3"),
        None,
        "user=canoatesta&password=pw-a",
    );
    assert_eq!(answer.status, 503);
    assert!(answer.cookie().is_none());
    assert_eq!(rig.accounts.asked.load(Ordering::SeqCst), 0);
}

#[test]
fn a_policy_that_names_a_provider_or_a_library_that_is_not_there_does_not_start() {
    for config in [
        "[sign_in_policy]\nprovider = \"nobody\"",
        "[sign_in_policy]\nlibrary = \"/opt/x.so\"",
        "[sign_in_policy]\nprovider = \"other\"\nlibrary = \"relative.so\"",
        "[[sign_in_policy.config.rule]]\nallow = [[\"password\"]]\nsurprise = 1",
        "[[sign_in_policy.config.rule]]\nallow = []",
        "[[sign_in_policy.config.rule]]\nnetworks = [\"nonsense\"]\nallow = [[\"password\"]]",
    ] {
        let built = Settings::from_toml(config)
            .and_then(|settings| policy::Policy::new(&settings, &[Method::Password]).map(drop));
        assert!(built.is_err(), "{config}");
    }
}

/// Stand-in providers for the shared flows: `local` accepts `alice` with `pw`, `ldap` accepts nobody, and a certificate names
/// `alice` or `bob`. It records which providers were asked.
struct Flow {
    asked: Mutex<Vec<String>>,
}

impl Flow {
    fn person(name: &str, provider: &str) -> SignIn {
        let groups = if name == "alice" {
            vec!["admins".to_owned()]
        } else {
            Vec::new()
        };
        SignIn::Admitted {
            user: name.to_owned(),
            uid: mine(),
            groups,
            provider: provider.to_owned(),
        }
    }

    fn providers(&self, providers: &[String]) -> Vec<String> {
        if providers.is_empty() {
            vec!["local".to_owned()]
        } else {
            providers.to_vec()
        }
    }
}

impl Accounts for Flow {
    fn methods(&self) -> Vec<Method> {
        vec![Method::Password, Method::Certificate]
    }

    fn sign_in(&self, user: &str, password: &str, providers: &[String]) -> SignIn {
        for provider in self.providers(providers) {
            self.asked.lock().unwrap().push(provider.clone());
            if provider == "local" && user == "alice" && password == "pw" {
                return Self::person("alice", &provider);
            }
        }
        SignIn::Refused
    }

    fn sign_in_certificate(
        &self,
        headers: &[(String, String)],
        _client: IpAddr,
        providers: &[String],
    ) -> SignIn {
        let provider = self.providers(providers).remove(0);
        self.asked.lock().unwrap().push(provider.clone());
        match headers
            .iter()
            .find(|(name, _)| name == "x-client-cert")
            .map(|(_, value)| value.as_str())
        {
            Some(name @ ("alice" | "bob")) => Self::person(name, &provider),
            _ => SignIn::Refused,
        }
    }

    fn still(&self, user: &str, _uid: u32) -> Standing {
        Standing::Present(if user == "alice" {
            vec!["admins".to_owned()]
        } else {
            Vec::new()
        })
    }
}

fn flow_rig(config: &serde_json::Value) -> Rig<Flow> {
    let entry = Entry {
        provider: policy::RULES.to_owned(),
        library: None,
        config: config.as_object().unwrap().clone(),
        enrol: false,
    };
    rig_with(
        Arc::new(Flow {
            asked: Mutex::new(Vec::new()),
        }),
        |settings| {
            settings.sign_in_policy = Some(entry);
            settings.max_failures_per_pair = 100;
            settings.max_failures_per_address = 1000;
        },
        0,
    )
}

/// Whether a flow needs a second factor, which the portal does not ask for yet.
fn needs_a_second_factor(flow: &serde_json::Value) -> bool {
    flow["steps"].as_array().unwrap().iter().any(|step| {
        step["expect"]["result"] == "next"
            || matches!(step["method"].as_str(), Some("totp" | "recovery_code"))
    })
}

#[test]
fn every_flow_without_a_second_factor_goes_through_the_portals_request_path_as_the_contract_says() {
    let document: serde_json::Value = serde_json::from_str(FLOWS).unwrap();
    let flows = document["flows"].as_array().unwrap();
    let (second, single): (Vec<_>, Vec<_>) =
        flows.iter().partition(|flow| needs_a_second_factor(flow));
    assert_eq!(single.len(), 8, "the flows that need no second factor");
    assert!(!second.is_empty());
    for flow in single {
        let name = flow["name"].as_str().unwrap();
        let rig = flow_rig(&flow["config"]);
        let mut cookie: Option<String> = None;
        for (index, step) in flow["steps"].as_array().unwrap().iter().enumerate() {
            let at = format!("{name}, step {index}");
            let address = step.get("address").and_then(serde_json::Value::as_str);
            let counted_from: IpAddr = address.map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |text| {
                text.parse().unwrap()
            });
            let calls_before = rig.accounts.asked.lock().unwrap().len();
            let counted_before = rig.inner.throttle.failures_from(counted_from);
            let (result, methods) = match step["action"].as_str().unwrap() {
                "offer" => ("methods", Some(methods_at(&rig, address))),
                "resume" => {
                    let answer = index_at(&rig, cookie.as_deref().unwrap(), address);
                    (
                        if answer.status == 200 {
                            "signed_in"
                        } else {
                            "failed"
                        },
                        None,
                    )
                }
                "present" => {
                    let text = |key: &str| step[key].as_str().unwrap_or_default();
                    let (certificate, body) = match text("method") {
                        "certificate" => (Some(text("name")), "method=certificate".to_owned()),
                        method => (
                            None,
                            format!(
                                "method={method}&user={}&password={}",
                                text("name"),
                                text("secret")
                            ),
                        ),
                    };
                    let answer = form_at(&rig, address, certificate, &body);
                    let result = match answer.status {
                        303 => {
                            cookie = answer.cookie();
                            "signed_in"
                        }
                        403 => "refused_unasked",
                        401 => "failed",
                        other => panic!("{at}: status {other}"),
                    };
                    (result, None)
                }
                other => panic!("{at}: an action {other}"),
            };
            let expect = &step["expect"];
            assert_eq!(result, expect["result"].as_str().unwrap(), "{at}");
            let asked = rig.accounts.asked.lock().unwrap()[calls_before..].to_vec();
            assert_eq!(asked.len() as i64, expect["authenticator_calls"], "{at}");
            let counted = rig.inner.throttle.failures_from(counted_from) - counted_before;
            assert_eq!(
                counted > 0,
                expect["throttled"] == true,
                "{at}: throttle count"
            );
            if let Some(list) = expect.get("methods") {
                let expected: Vec<String> = list
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|word| word.as_str().unwrap().to_owned())
                    .collect();
                assert_eq!(methods.unwrap(), expected, "{at}");
            }
            if let Some(list) = expect.get("asked") {
                let expected: Vec<&str> = list
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|word| word.as_str().unwrap())
                    .collect();
                assert_eq!(asked, expected, "{at}");
            }
        }
    }
}

#[test]
fn the_groups_the_directory_gives_reach_the_policy_at_sign_in_and_when_the_session_is_used() {
    // Administrators may not use a password at all; everyone else may.
    let config = serde_json::json!({"rule": [
        {"subjects": ["group:admins"], "allow": [["certificate"]]},
        {"allow": [["password"], ["certificate"]]},
    ]});
    let rig = flow_rig(&config);
    // Before anyone is named both are offered: the rule that narrows is not known to apply yet.
    assert_eq!(
        methods_at(&rig, Some("10.1.2.3")),
        ["certificate", "password"]
    );
    // alice is in admins: her valid password is refused as a wrong one, counted.
    let counted_from: IpAddr = "10.1.2.3".parse().unwrap();
    let refused = form_at(&rig, Some("10.1.2.3"), None, "user=alice&password=pw");
    assert_eq!(refused.status, 401);
    assert!(refused.cookie().is_none());
    assert_eq!(rig.inner.throttle.failures_from(counted_from), 1);
    // bob is in no group: the certificate sign-in of a person the second rule governs works, and so does a session of hers.
    let bob = form_at(&rig, Some("10.1.2.3"), Some("bob"), "method=certificate");
    assert_eq!(bob.status, 303);
    assert_eq!(
        index_at(&rig, &bob.cookie().unwrap(), Some("10.1.2.3")).status,
        200
    );
    let alice = form_at(&rig, Some("10.1.2.3"), Some("alice"), "method=certificate");
    assert_eq!(alice.status, 303);
    assert_eq!(
        index_at(&rig, &alice.cookie().unwrap(), Some("10.1.2.3")).status,
        200
    );
}

/// The rule the guide shows is the rule these tests run.
#[test]
fn the_guide_shows_the_network_rule_as_these_tests_run_it() {
    let guide = std::fs::read_to_string(format!(
        "{}/../../docs/portal.md",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    assert!(guide.contains(NETWORK_RULE.trim()));
}
