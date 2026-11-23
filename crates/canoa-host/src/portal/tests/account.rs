//! `GET /portal/account` and the links between the portal's own pages, through the portal's own request handling and the real
//! password, one-time code, certificate and passkey providers: what each page says it may do for a session at an address, with
//! the policy's own rules, and that the page then does what was said.

use super::second_step::{PASSWORD, code_in, file_source, my_name, password_file, real_entry};
use super::sign_in_policy::{proxied, set_to};
use super::*;
use crate::portal::device::Device;

const INSIDE: &str = "10.6.20.5";
const OUTSIDE: &str = "203.0.113.7";

/// A password signs in from anywhere; a person shows their password again to set up a code or a passkey from inside 10.0.0.0/8
/// and nowhere else, the rule a portal that keeps a first code for the office writes.
fn rules() -> serde_json::Value {
    serde_json::json!({"rule": [
        {"purposes": ["enrol"], "networks": ["10.0.0.0/8"], "allow": [["password"]]},
        {"purposes": ["sign_in", "resume"], "allow": [["password"], ["certificate"]]},
    ]})
}

struct Portal {
    rig: Rig<Access>,
    name: String,
    dir: PathBuf,
}

impl Drop for Portal {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn portal(rules: &serde_json::Value, passkeys: bool) -> Portal {
    portal_with(rules, passkeys, |_| {})
}

fn portal_with(
    rules: &serde_json::Value,
    passkeys: bool,
    change: impl FnOnce(&mut Settings),
) -> Portal {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "canoa-account-{}-{}",
        std::process::id(),
        COUNT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let name = my_name();
    let passwords = password_file(&dir, &name);
    let mut totp = file_source(&dir.join("totp.toml"));
    totp["issuer"] = "Canoa Test".into();
    let mut certs = real_entry("coqui_access_mtls", &file_source(&dir.join("certs.toml")));
    certs.enrol = true;
    let mut authenticators = vec![
        real_entry("coqui_access_password", &file_source(&passwords)),
        real_entry("coqui_access_totp", &totp),
        certs,
    ];
    if passkeys {
        let mut key = file_source(&dir.join("passkeys.toml"));
        key["rp_id"] = "localhost".into();
        key["origins"] = serde_json::json!(["http://localhost"]);
        authenticators.push(real_entry("coqui_access_passkey", &key));
    }
    let own = Settings {
        min_uid: mine().max(1),
        authenticators,
        libraries_any_owner: true,
        ..Settings::default()
    };
    let access = Arc::new(Access::new(&own).unwrap());
    let rules = rules.as_object().unwrap().clone();
    let rig = rig_with(
        access,
        |settings| {
            settings.sign_in_policy = Some(Entry {
                provider: policy::RULES.to_owned(),
                library: None,
                config: rules,
                enrol: false,
            });
            settings.max_failures_per_pair = 100;
            settings.max_failures_per_address = 1000;
            settings.session = Duration::from_secs(600);
            change(settings);
        },
        0,
    );
    Portal { rig, name, dir }
}

impl Portal {
    fn send(&self, method: &str, path: &str, from: &str, cookies: &[&str], body: &str) -> Answer {
        let to = proxied(&self.rig);
        let mut extra = vec![("X-Forwarded-For", from.to_owned())];
        if method == "POST" {
            extra.push(("Origin", to.origin()));
            extra.push((
                "Content-Type",
                "application/x-www-form-urlencoded".to_owned(),
            ));
        }
        if !cookies.is_empty() {
            extra.push(("Cookie", cookies.join("; ")));
        }
        request(&to, method, path, &extra, body)
    }

    fn get(&self, path: &str, from: &str, cookies: &[&str]) -> Answer {
        self.send("GET", path, from, cookies, "")
    }

    /// A password sign-in from `from`: the session cookie.
    fn session(&self, from: &str) -> String {
        let answer = self.send(
            "POST",
            LOGIN,
            from,
            &[],
            &format!("method=password&user={}&password={PASSWORD}", self.name),
        );
        assert_eq!(answer.status, 303, "{}", answer.body);
        format!("{COOKIE}={}", set_to(&answer, COOKIE).expect("a session"))
    }

    /// What `GET /portal/account` answers the session `cookie` from `from`.
    fn account(&self, from: &str, cookie: &str) -> serde_json::Value {
        let answer = self.get(ACCOUNT, from, &[cookie]);
        assert_eq!(answer.status, 200, "{}", answer.body);
        assert_eq!(answer.header("cache-control"), Some("no-store"));
        serde_json::from_str(&answer.body).unwrap()
    }
}

#[test]
fn the_answer_is_for_a_session_and_for_nobody_else() {
    let portal = portal(&rules(), true);
    assert_eq!(portal.get(ACCOUNT, INSIDE, &[]).status, 401);
    assert_eq!(
        portal
            .get(ACCOUNT, INSIDE, &[&format!("{COOKIE}=nonsense")])
            .status,
        401
    );
    let cookie = portal.session(INSIDE);
    let ended = portal.send("POST", LOGOUT, INSIDE, &[&cookie], "");
    assert_eq!(ended.status, 303);
    assert_eq!(portal.get(ACCOUNT, INSIDE, &[&cookie]).status, 401);
}

#[test]
fn a_person_inside_the_network_may_set_up_a_code_and_a_passkey_and_one_outside_may_not() {
    let portal = portal(&rules(), true);
    let cookie = portal.session(INSIDE);
    let inside = portal.account(INSIDE, &cookie);
    assert_eq!(inside["user"], portal.name);
    assert_eq!(inside["code"]["path"], "/portal/factor");
    assert_eq!(inside["code"]["allowed"], true);
    assert_eq!(inside["code"]["set_up"], false);
    assert!(inside["code"].get("reason").is_none(), "{inside}");
    assert_eq!(inside["passkeys"]["path"], "/portal/passkeys");
    assert_eq!(inside["passkeys"]["allowed"], true);
    assert_eq!(inside["passkeys"]["count"], 0);
    assert_eq!(inside["sign_out"]["path"], "/portal/logout");
    assert_eq!(inside["sign_out"]["allowed"], true);
    // The same session at another address: the rule allows nothing there, and the answer says so with a reason.
    let outside = portal.account(OUTSIDE, &cookie);
    assert_eq!(outside["code"]["allowed"], false, "{outside}");
    assert_eq!(outside["code"]["set_up"], false);
    assert!(
        outside["code"]["reason"]
            .as_str()
            .is_some_and(|why| why.contains("not allowed from here")),
        "{outside}"
    );
    assert_eq!(outside["passkeys"]["allowed"], false, "{outside}");
    assert!(outside["passkeys"]["reason"].is_string(), "{outside}");
    assert_eq!(outside["sign_out"]["allowed"], true);
    // A page says what the answer said: where it was open, it offers to set a code up; where it was not, it offers nothing.
    let open = portal.get(FACTOR, INSIDE, &[&cookie]);
    assert!(
        open.body.contains("show who you are again"),
        "{}",
        open.body
    );
    let shut = portal.get(FACTOR, OUTSIDE, &[&cookie]);
    assert!(
        shut.body.contains("needs an administrator") && !shut.body.contains("name=\"password\""),
        "{}",
        shut.body
    );
    let held = portal.get(PASSKEYS, OUTSIDE, &[&cookie]);
    assert!(
        held.body.contains("not offer here") && !held.body.contains("data-passkey=\"create\""),
        "{}",
        held.body
    );
}

#[test]
fn a_code_that_is_set_up_is_said_to_be_and_a_passkey_held_keeps_its_page_open_from_anywhere() {
    let portal = portal(&rules(), true);
    // A passkey the account already holds: naming and removing it needs the session only, so the page is open outside too.
    let mut device = Device::new();
    let accounts = &portal.rig.accounts;
    let options = accounts.begin_passkey(&portal.name, mine()).unwrap();
    let answer = device.register("http://localhost", &options).unwrap();
    accounts
        .confirm_passkey(&portal.name, mine(), &answer, "laptop")
        .unwrap();
    let cookie = portal.session(INSIDE);
    let outside = portal.account(OUTSIDE, &cookie);
    assert_eq!(outside["passkeys"]["count"], 1, "{outside}");
    assert_eq!(outside["passkeys"]["allowed"], true, "{outside}");
    assert!(outside["passkeys"].get("reason").is_none());
    // An administrator's setup makes the code the account's.
    assert_eq!(portal.account(INSIDE, &cookie)["code"]["set_up"], false);
    let begun = accounts.begin_factor(&portal.name, mine(), false).unwrap();
    accounts
        .confirm_factor(&portal.name, mine(), &code_in(&begun.uri, 0))
        .unwrap();
    let inside = portal.account(INSIDE, &cookie);
    assert_eq!(inside["code"]["set_up"], true, "{inside}");
    // The rules ask for the password alone, which does not replace a code in use: the page offers nothing it could finish.
    assert_eq!(inside["code"]["allowed"], false, "{inside}");
    assert!(
        inside["code"]["reason"]
            .as_str()
            .is_some_and(|why| why.contains("set up already"))
    );
}

#[test]
fn a_portal_with_no_passkey_provider_leaves_the_passkeys_out_and_the_pages_link_only_what_it_has() {
    let portal = portal(&rules(), false);
    let cookie = portal.session(INSIDE);
    let account = portal.account(INSIDE, &cookie);
    assert!(account.get("passkeys").is_none(), "{account}");
    assert!(account.get("code").is_some() && account.get("certificate").is_some());
    let factor = portal.get(FACTOR, INSIDE, &[&cookie]).body;
    assert!(
        factor.contains(&format!("href=\"{CERTIFICATE}\"")) && !factor.contains(PASSKEYS),
        "{factor}"
    );
}

#[test]
fn the_certificate_page_is_open_when_one_is_pinned_or_offered_and_says_why_when_neither() {
    let portal = portal(&rules(), true);
    let cookie = portal.session(INSIDE);
    let account = portal.account(INSIDE, &cookie);
    assert_eq!(account["certificate"]["path"], "/portal/certificate");
    assert_eq!(account["certificate"]["pinned"], false);
    assert_eq!(account["certificate"]["allowed"], false, "{account}");
    assert!(
        account["certificate"]["reason"]
            .as_str()
            .is_some_and(|why| why.contains("no certificate")),
        "{account}"
    );
}

#[test]
fn each_page_links_to_the_others_and_back_to_the_app() {
    let portal = portal(&rules(), true);
    let cookie = portal.session(INSIDE);
    let page = |path: &str| portal.get(path, INSIDE, &[&cookie]).body;
    let (factor, passkeys, certificate) = (page(FACTOR), page(PASSKEYS), page(CERTIFICATE));
    for (body, here, others) in [
        (&factor, FACTOR, [PASSKEYS, CERTIFICATE]),
        (&passkeys, PASSKEYS, [FACTOR, CERTIFICATE]),
        (&certificate, CERTIFICATE, [FACTOR, PASSKEYS]),
    ] {
        for other in others {
            assert!(
                body.contains(&format!("href=\"{other}\"")),
                "{here}: {body}"
            );
        }
        assert!(
            !body.contains(&format!("<a href=\"{here}\"")),
            "{here} links to itself: {body}"
        );
        assert!(
            body.contains("<a href=\"/\">Continue to Canoa</a>"),
            "{here}"
        );
    }
}

/// `clients/browser/chromium/e2e/portal_account.mjs`, until the file named by `CANOA_TEST_SERVE_UNTIL` exists, with the web page
/// `app/tool/signin_page.dart` built into the folder `CANOA_TEST_SITE` as the portal's site:
/// `CANOA_TEST_SITE=<dir> CANOA_TEST_SERVE_UNTIL=/tmp/stop cargo test -p canoa-host --features portal --lib serve_the_account_pages_for_a_browser -- --ignored --nocapture`.
/// The loopback network stands where 10.0.0.0/8 does, and a password shown again may set a code up.
#[test]
#[ignore = "serves until a file exists, for a browser run"]
fn serve_the_account_pages_for_a_browser() {
    let until =
        PathBuf::from(std::env::var("CANOA_TEST_SERVE_UNTIL").expect("the file that ends the run"));
    let site = PathBuf::from(std::env::var("CANOA_TEST_SITE").expect("the folder of the web page"));
    let rules = serde_json::json!({"rule": [
        {"purposes": ["enrol"], "networks": ["127.0.0.0/8"], "allow": [["password"]]},
        {"purposes": ["sign_in", "resume"], "allow": [["password"]]},
    ]});
    let portal = portal_with(&rules, false, |settings| settings.site = Some(site));
    println!(
        "{}",
        serde_json::json!({
            "serving": {
                "url": format!("http://localhost:{}", portal.rig.addr.port()),
                "user": portal.name,
                "password": PASSWORD,
            }
        })
    );
    while !until.exists() {
        thread::sleep(Duration::from_millis(200));
    }
}
