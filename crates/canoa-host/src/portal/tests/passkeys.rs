//! Signing in by passkey, and the Passkeys page, through the portal's own request handling and the real passkey provider, with a
//! software device that signs what a browser would. The rules are the ones a portal that accepts a passkey wherever it accepts a
//! certificate runs: inside 10.0.0.0/8 a password, a certificate or a passkey; elsewhere a certificate or a passkey; and a
//! passkey is added only after showing a certificate or a passkey again.

use super::second_step::{PASSWORD, file_source, my_name, password_file, real_entry};
use super::sign_in_policy::{index_at, methods_at, methods_holding, proxied, set_to};
use super::*;
use crate::portal::device::Device;

/// The relying party and origin of the tests: the portal answers `localhost` on its socket.
const RP: &str = "localhost";
const ORIGIN: &str = "http://localhost";

const INSIDE: &str = "10.6.20.5";
const OUTSIDE: &str = "203.0.113.7";

/// The cookie that names the passkey challenge a browser holds (this module is called `passkeys` too).
const CEREMONY: &str = crate::portal::passkeys::COOKIE;

/// The rules of the owner's portal: a passkey where a certificate is, a password only inside, and a passkey added only after a
/// certificate or a passkey is shown again.
fn owner_rules() -> serde_json::Value {
    serde_json::json!({"rule": [
        {"purposes": ["enrol"], "allow": [["certificate"], ["passkey"]]},
        {"networks": ["10.0.0.0/8"], "allow": [["password"], ["certificate"], ["passkey"]]},
        {"allow": [["certificate"], ["passkey"]]},
    ]})
}

struct Keys {
    rig: Rig<Access>,
    name: String,
    dir: PathBuf,
}

impl Drop for Keys {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn keys(rules: &serde_json::Value, change: impl FnOnce(&mut Settings)) -> Keys {
    keys_with(rules, &serde_json::json!({}), change)
}

/// [`keys`] with `extra` among the passkey provider's settings.
fn keys_with(
    rules: &serde_json::Value,
    extra: &serde_json::Value,
    change: impl FnOnce(&mut Settings),
) -> Keys {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "canoa-passkeys-{}-{}",
        std::process::id(),
        COUNT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let name = my_name();
    let passwords = password_file(&dir, &name);
    let mut passkey = file_source(&dir.join("passkeys.toml"));
    passkey["rp_id"] = RP.into();
    passkey["origins"] = serde_json::json!([ORIGIN]);
    for (key, value) in extra.as_object().unwrap() {
        passkey[key] = value.clone();
    }
    let own = Settings {
        min_uid: mine().max(1),
        authenticators: vec![
            real_entry("coqui_access_password", &file_source(&passwords)),
            real_entry("coqui_access_mtls", &file_source(&dir.join("certs.toml"))),
            real_entry("coqui_access_passkey", &passkey),
        ],
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
    Keys { rig, name, dir }
}

fn urlencoded(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

/// `name=value` for the cookie `name` an answer sets.
fn cookie_of(answer: &Answer, name: &str) -> String {
    format!(
        "{name}={}",
        set_to(answer, name).unwrap_or_else(|| panic!("{name} was set: {}", answer.body))
    )
}

/// The options a challenge answer carries, as the text a device is given.
fn options_of(answer: &Answer) -> String {
    assert_eq!(answer.status, 200, "{}", answer.body);
    let body: serde_json::Value = serde_json::from_str(&answer.body).unwrap();
    body["options"].to_string()
}

impl Keys {
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

    fn post(&self, path: &str, from: &str, cookies: &[&str], body: &str) -> Answer {
        self.send("POST", path, from, cookies, body)
    }

    /// A password sign-in from `from`: the session cookie, when it is one.
    fn password_in(&self, from: &str) -> Answer {
        self.post(
            LOGIN,
            from,
            &[],
            &format!("method=password&user={}&password={PASSWORD}", self.name),
        )
    }

    /// A challenge for the sign-in a browser at `from` is at, holding `cookies`.
    fn challenge(&self, from: &str, cookies: &[&str]) -> Answer {
        self.post(PASSKEY, from, cookies, "")
    }

    /// The sign-in by passkey the login page runs: a challenge, the device's answer, and the form. The answer to the form.
    fn passkey_in(&self, device: &mut Device, from: &str, held: &[&str]) -> Answer {
        let asked = self.challenge(from, held);
        let ceremony = cookie_of(&asked, CEREMONY);
        let answer = device.authenticate(ORIGIN, &options_of(&asked)).unwrap();
        let mut cookies = held.to_vec();
        cookies.push(&ceremony);
        self.post(
            LOGIN,
            from,
            &cookies,
            &format!("method=passkey&answer={}", urlencoded(&answer)),
        )
    }

    /// Adds a passkey to the account through the provider, as the page does after its steps: the device's credential.
    fn add(&self, device: &mut Device, label: &str) -> String {
        let accounts = &self.rig.accounts;
        let options = accounts.begin_passkey(&self.name, mine()).unwrap();
        let answer = device.register(ORIGIN, &options).unwrap();
        accounts
            .confirm_passkey(&self.name, mine(), &answer, label)
            .unwrap()
    }

    fn held(&self) -> Vec<PasskeyInfo> {
        self.rig.accounts.passkeys(&self.name, mine()).unwrap()
    }

    /// A session opened by the device's passkey from `from`.
    fn session_by_passkey(&self, device: &mut Device, from: &str) -> String {
        let answer = self.passkey_in(device, from, &[]);
        assert_eq!(answer.status, 303, "{}", answer.body);
        cookie_of(&answer, COOKIE)
    }
}

#[test]
fn the_methods_and_the_page_offer_a_passkey_wherever_a_certificate_is_and_a_password_only_inside() {
    let keys = keys(&owner_rules(), |_| {});
    let rig = &keys.rig;
    assert_eq!(
        methods_at(rig, Some(OUTSIDE)),
        ["certificate", "passkey"],
        "outside 10/8"
    );
    assert_eq!(
        methods_at(rig, Some(INSIDE)),
        ["password", "certificate", "passkey"]
    );
    for other in [Some("2001:db8::1"), None, Some("not an address")] {
        assert_eq!(
            methods_at(rig, other),
            ["certificate", "passkey"],
            "{other:?}"
        );
    }
    // The certificate is the policy's first way in from outside, so it is the tab shown; the passkey is the next.
    let outside = keys.get(LOGIN, OUTSIDE, &[]);
    assert!(
        outside.body.contains("Sign in with your certificate")
            && outside.body.contains("id=\"tab-passkey\"")
            && !outside.body.contains("data-passkey=\"get\"")
            && !outside.body.contains("name=\"password\""),
        "{}",
        outside.body
    );
    let outside = keys.get(&format!("{LOGIN}?method=passkey"), OUTSIDE, &[]);
    assert!(
        outside.body.contains("data-passkey=\"get\"")
            && outside.body.contains("Sign in with a passkey")
            && !outside.body.contains("name=\"password\""),
        "{}",
        outside.body
    );
    // The passkey form is hidden until the script runs, and says so where it is the one shown without it.
    assert!(
        outside.body.contains("<div hidden data-gated><form")
            && outside.body.contains("<noscript>"),
        "{}",
        outside.body
    );
    let policy = outside.header("content-security-policy").unwrap();
    assert!(
        policy.contains("script-src 'self'")
            && policy.contains("connect-src 'self'")
            && !policy.contains("unsafe-inline"),
        "{policy}"
    );
    let script = keys.get(page::JS_PATH, OUTSIDE, &[]);
    assert!(script.body.contains("navigator.credentials.get"));
    assert!(!script.body.contains("eval(") && !script.body.contains("innerHTML"));
}

#[test]
fn a_passkey_signs_in_from_outside_10_8_and_a_password_from_there_is_refused_unasked() {
    let keys = keys(&owner_rules(), |settings| {
        settings.max_failures_per_pair = 1;
        settings.max_failures_per_address = 5;
    });
    let mut device = Device::new();
    let id = keys.add(&mut device, "laptop");
    // A password from outside is refused before any provider is asked, and counts against no one.
    let outside: IpAddr = OUTSIDE.parse().unwrap();
    let refused = keys.post(
        LOGIN,
        OUTSIDE,
        &[],
        &format!("method=password&user={}&password={PASSWORD}", keys.name),
    );
    assert_eq!(refused.status, 403);
    assert_eq!(keys.rig.inner.throttle.failures_from(outside), 0);
    // A passkey signs in, from the same address, with no name typed.
    let signed = keys.passkey_in(&mut device, OUTSIDE, &[]);
    assert_eq!(
        (signed.status, signed.header("location")),
        (303, Some("/")),
        "{}",
        signed.body
    );
    let session = cookie_of(&signed, COOKIE);
    assert_eq!(index_at(&keys.rig, &session, Some(OUTSIDE)).status, 200);
    // The session says how it was made and by which passkey.
    let token = session.strip_prefix(&format!("{COOKIE}=")).unwrap();
    let made = keys.rig.inner.sessions.get(token).unwrap();
    assert_eq!(made.method, Some(Method::Passkey));
    assert_eq!(made.passkey.as_deref(), Some(id.as_str()));
    assert_eq!(keys.rig.inner.throttle.failures_from(outside), 0);
    // A passkey of an account the system does not admit is a failed try that counts.
    let mut stranger = Device::new();
    {
        let accounts = &keys.rig.accounts;
        let options = accounts
            .begin_passkey("canoa-no-such-person", 4242)
            .unwrap();
        let answer = stranger.register(ORIGIN, &options).unwrap();
        accounts
            .confirm_passkey("canoa-no-such-person", 4242, &answer, "x")
            .unwrap();
    }
    let inside: IpAddr = INSIDE.parse().unwrap();
    let failed = keys.passkey_in(&mut stranger, INSIDE, &[]);
    assert_eq!(failed.status, 401, "{}", failed.body);
    assert!(failed.body.contains("That passkey was not accepted."));
    assert!(set_to(&failed, COOKIE).is_none());
    assert_eq!(keys.rig.inner.throttle.failures_from(inside), 1);
}

#[test]
fn a_challenge_has_the_portals_host_as_its_relying_party_and_asks_for_the_person_to_be_verified() {
    let keys = keys(&owner_rules(), |_| {});
    let asked = keys.challenge(OUTSIDE, &[]);
    assert_eq!(asked.status, 200);
    let options: serde_json::Value = serde_json::from_str(&options_of(&asked)).unwrap();
    assert_eq!(options["publicKey"]["rpId"], RP);
    assert_eq!(options["publicKey"]["userVerification"], "required");
    // No name was typed, so no account is named: the browser offers the passkeys it holds for this site.
    assert!(
        options["publicKey"]["allowCredentials"]
            .as_array()
            .is_none_or(Vec::is_empty)
    );
    let cookie = asked
        .headers
        .iter()
        .find(|(name, value)| {
            name.eq_ignore_ascii_case("set-cookie") && value.starts_with(CEREMONY)
        })
        .unwrap()
        .1
        .clone();
    for needed in [
        "Secure",
        "HttpOnly",
        "SameSite=Strict",
        "Path=/",
        "Max-Age=",
    ] {
        assert!(cookie.contains(needed), "{cookie}");
    }
    assert!(!cookie.contains("Domain"), "{cookie}");
    assert_eq!(asked.header("cache-control"), Some("no-store"));
}

#[test]
fn an_answer_is_taken_once_and_only_from_the_browser_that_holds_its_challenge() {
    let keys = keys(&owner_rules(), |_| {});
    let mut device = Device::new();
    keys.add(&mut device, "laptop");
    let from: IpAddr = OUTSIDE.parse().unwrap();
    // The answer is used once: the challenge is gone with it.
    let asked = keys.challenge(OUTSIDE, &[]);
    let ceremony = cookie_of(&asked, CEREMONY);
    let answer = device.authenticate(ORIGIN, &options_of(&asked)).unwrap();
    let body = format!("method=passkey&answer={}", urlencoded(&answer));
    let first = keys.post(LOGIN, OUTSIDE, &[&ceremony], &body);
    assert_eq!(first.status, 303, "{}", first.body);
    let again = keys.post(LOGIN, OUTSIDE, &[&ceremony], &body);
    assert_eq!(again.status, 401);
    assert!(set_to(&again, COOKIE).is_none());
    assert_eq!(keys.rig.inner.throttle.failures_from(from), 1);
    // An answer with no cookie is a failed try, and so is one with a cookie that names nothing.
    let asked = keys.challenge(OUTSIDE, &[]);
    let answer = device.authenticate(ORIGIN, &options_of(&asked)).unwrap();
    let body = format!("method=passkey&answer={}", urlencoded(&answer));
    assert_eq!(keys.post(LOGIN, OUTSIDE, &[], &body).status, 401);
    let made_up = format!("{}={}", CEREMONY, "A".repeat(43));
    assert_eq!(keys.post(LOGIN, OUTSIDE, &[&made_up], &body).status, 401);
    assert_eq!(keys.rig.inner.throttle.failures_from(from), 3);
    // Another browser's cookie does not take it: the answer was signed over the first browser's challenge.
    let mine_ = keys.challenge(OUTSIDE, &[]);
    let theirs = keys.challenge(OUTSIDE, &[]);
    let answer = device.authenticate(ORIGIN, &options_of(&mine_)).unwrap();
    let body = format!("method=passkey&answer={}", urlencoded(&answer));
    let wrong = cookie_of(&theirs, CEREMONY);
    let crossed = keys.post(LOGIN, OUTSIDE, &[&wrong], &body);
    assert_eq!(crossed.status, 401);
    assert!(set_to(&crossed, COOKIE).is_none());
    // And the one it was signed over is gone with the other, for it was taken only by the cookie that names it.
    let right = cookie_of(&mine_, CEREMONY);
    assert_eq!(keys.post(LOGIN, OUTSIDE, &[&right], &body).status, 303);
}

#[test]
fn no_challenge_is_made_where_a_passkey_is_not_offered_and_none_for_a_request_not_from_the_page() {
    let keys = keys(
        &serde_json::json!({"rule": [{"allow": [["password"]]}]}),
        |_| {},
    );
    assert_eq!(methods_at(&keys.rig, Some(OUTSIDE)), ["password"]);
    let refused = keys.challenge(OUTSIDE, &[]);
    assert_eq!(refused.status, 403);
    assert!(set_to(&refused, CEREMONY).is_none());
    assert_eq!(keys.rig.inner.ceremonies.waiting(), 0);
    assert!(
        !keys.dir.join("passkeys.toml.ceremonies").exists(),
        "the provider was never asked"
    );
    // A sign-in by a passkey is refused too, before anything counts.
    let answer = keys.post(LOGIN, OUTSIDE, &[], "method=passkey&answer=%7B%7D");
    assert_eq!(answer.status, 403);
    assert_eq!(
        keys.rig
            .inner
            .throttle
            .failures_from(OUTSIDE.parse().unwrap()),
        0
    );
    // Where it is offered, a challenge asked for by another site's page is not made.
    let open = self::keys(&owner_rules(), |_| {});
    let to = proxied(&open.rig);
    let foreign = request(
        &to,
        "POST",
        PASSKEY,
        &[
            ("Origin", "https://evil.example".to_owned()),
            ("X-Forwarded-For", OUTSIDE.to_owned()),
        ],
        "",
    );
    assert_eq!(foreign.status, 403);
    let none = request(
        &to,
        "POST",
        PASSKEY,
        &[("X-Forwarded-For", OUTSIDE.to_owned())],
        "",
    );
    assert_eq!(none.status, 403);
    assert_eq!(open.rig.inner.ceremonies.waiting(), 0);
}

#[test]
fn challenges_are_bounded_for_one_address_and_in_all() {
    let keys = keys(&owner_rules(), |settings| {
        settings.max_passkey_ceremonies = 3;
    });
    // The provider holds eight challenges for one address; the portal holds three in all.
    for index in 0..3 {
        let from = format!("203.0.113.{}", 10 + index);
        assert_eq!(keys.challenge(&from, &[]).status, 200, "{from}");
    }
    let full = keys.challenge("203.0.113.99", &[]);
    assert_eq!(full.status, 429);
    assert!(full.header("retry-after").is_some());
    // A browser asking again gives up the one it held, and so is not turned away.
    let held = keys.challenge("203.0.113.10", &[]);
    assert_eq!(held.status, 429, "the store is full of others'");
    let roomy = self::keys(&owner_rules(), |_| {});
    let mut last = 0;
    for _ in 0..9 {
        last = roomy.challenge(OUTSIDE, &[]).status;
    }
    assert_eq!(last, 429, "the provider's own bound for one address");
    assert_eq!(roomy.challenge("203.0.113.8", &[]).status, 200);
}

#[test]
fn failed_answers_stop_at_the_addresss_limit_and_another_address_is_not_slowed() {
    let keys = keys(&owner_rules(), |settings| {
        settings.max_failures_per_address = 2;
        settings.max_failures_per_pair = 2;
    });
    let junk = "method=passkey&answer=%7B%22id%22%3A%22x%22%7D";
    for _ in 0..2 {
        assert_eq!(keys.post(LOGIN, OUTSIDE, &[], junk).status, 401);
    }
    let stopped = keys.post(LOGIN, OUTSIDE, &[], junk);
    assert_eq!(stopped.status, 429);
    assert!(stopped.header("retry-after").is_some());
    assert_eq!(keys.post(LOGIN, "203.0.113.8", &[], junk).status, 401);
}

#[test]
fn a_passkey_after_a_password_asks_for_that_accounts_passkeys_and_a_challenge_for_no_one_is_no_step()
 {
    let rules = serde_json::json!({"rule": [{"allow": [["password", "passkey"], ["passkey"]]}]});
    let keys = keys(&rules, |_| {});
    let mut device = Device::new();
    keys.add(&mut device, "laptop");
    // A challenge made before the password names no account.
    let early = keys.challenge(INSIDE, &[]);
    let early_cookie = cookie_of(&early, CEREMONY);
    let early_answer = device.authenticate(ORIGIN, &options_of(&early)).unwrap();
    let first = keys.password_in(INSIDE);
    assert_eq!(
        (first.status, first.header("location")),
        (303, Some(LOGIN)),
        "{}",
        first.body
    );
    assert!(
        set_to(&first, COOKIE).is_none(),
        "the password is not enough"
    );
    let pending = cookie_of(&first, steps::PENDING);
    let (step, methods) = methods_holding(&keys.rig, Some(INSIDE), Some(&pending));
    assert_eq!(step.as_deref(), Some("next"));
    assert_eq!(methods, ["passkey"]);
    let page = keys.get(LOGIN, INSIDE, &[&pending]);
    assert!(page.body.contains("data-passkey=\"get\"") && page.body.contains("One more step"));
    // The challenge made for no one is not the step this sign-in is at.
    let early_body = format!("method=passkey&answer={}", urlencoded(&early_answer));
    let crossed = keys.post(LOGIN, INSIDE, &[&pending, &early_cookie], &early_body);
    assert_eq!(crossed.status, 401, "{}", crossed.body);
    assert!(set_to(&crossed, COOKIE).is_none());
    // The one the step asks for names the account, and its passkey finishes the sign-in.
    let asked = keys.challenge(INSIDE, &[&pending]);
    let options: serde_json::Value = serde_json::from_str(&options_of(&asked)).unwrap();
    assert_eq!(
        options["publicKey"]["allowCredentials"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    let ceremony = cookie_of(&asked, CEREMONY);
    let answer = device.authenticate(ORIGIN, &options_of(&asked)).unwrap();
    let done = keys.post(
        LOGIN,
        INSIDE,
        &[&pending, &ceremony],
        &format!("method=passkey&answer={}", urlencoded(&answer)),
    );
    assert_eq!(done.status, 303, "{}", done.body);
    assert!(set_to(&done, COOKIE).is_some());
    let made = keys
        .rig
        .inner
        .sessions
        .get(&set_to(&done, COOKIE).unwrap())
        .unwrap();
    assert_eq!(
        made.done
            .iter()
            .map(|step| step.method.as_str())
            .collect::<Vec<_>>(),
        ["password", "passkey"]
    );
}

#[test]
fn a_challenge_made_for_another_account_does_not_finish_this_sign_in() {
    let rules = serde_json::json!({"rule": [{"allow": [["password", "passkey"]]}]});
    let keys = keys(&rules, |_| {});
    let mut device = Device::new();
    keys.add(&mut device, "laptop");
    let first = keys.password_in(INSIDE);
    let pending = cookie_of(&first, steps::PENDING);
    // A challenge the store holds for someone else, the browser's own cookie naming it.
    let token = keys
        .rig
        .inner
        .ceremonies
        .open(
            session::Ceremony {
                challenge: "not-this-accounts".to_owned(),
                account: Some(("somebody".to_owned(), mine() + 1)),
            },
            Duration::from_secs(60),
            None,
        )
        .unwrap();
    let held = format!("{}={token}", CEREMONY);
    let answer = keys.post(
        LOGIN,
        INSIDE,
        &[&pending, &held],
        "method=passkey&answer=%7B%22id%22%3A%22x%22%7D",
    );
    assert_eq!(answer.status, 401);
    assert!(set_to(&answer, COOKIE).is_none());
    assert_eq!(keys.rig.inner.ceremonies.waiting(), 0, "it was taken");
}

// ---- the Passkeys page ----

/// A session opened by password from inside.
fn inside_session(keys: &Keys) -> String {
    let answer = keys.password_in(INSIDE);
    assert_eq!(answer.status, 303, "{}", answer.body);
    cookie_of(&answer, COOKIE)
}

/// Shows the device's passkey again at the page, for the session: the pending cookie the steps are kept under.
fn show_again(keys: &Keys, device: &mut Device, session: &str, from: &str) -> String {
    let asked = keys.post(PASSKEY, from, &[session], "for=enrol");
    let ceremony = cookie_of(&asked, CEREMONY);
    let answer = device.authenticate(ORIGIN, &options_of(&asked)).unwrap();
    let stepped = keys.post(
        PASSKEYS,
        from,
        &[session, &ceremony],
        &format!("action=step&method=passkey&answer={}", urlencoded(&answer)),
    );
    assert_eq!(stepped.status, 200, "{}", stepped.body);
    assert!(
        stepped.body.contains("data-passkey=\"create\""),
        "the page offers to add one now: {}",
        stepped.body
    );
    cookie_of(&stepped, steps::PENDING)
}

#[test]
fn the_passkeys_page_needs_a_session_and_lists_what_the_account_holds_and_never_a_key() {
    let keys = keys(&owner_rules(), |_| {});
    let none = keys.get(PASSKEYS, INSIDE, &[]);
    assert_eq!((none.status, none.header("location")), (303, Some(LOGIN)));
    let mut device = Device::new();
    keys.add(&mut device, "my <laptop>");
    let session = inside_session(&keys);
    let page = keys.get(PASSKEYS, INSIDE, &[&session]);
    assert_eq!(page.status, 200);
    assert!(
        page.body.contains("my &lt;laptop&gt;") && !page.body.contains("my <laptop>"),
        "a label is escaped: {}",
        page.body
    );
    assert!(page.body.contains("never used") && page.body.contains("added "));
    let id = &keys.held()[0].id;
    assert!(page.body.contains(id.as_str()), "the id names it to a form");
    assert!(
        !page.body.to_lowercase().contains("public"),
        "no key is shown"
    );
    // The certificate page leads here, and nothing else shows the account's passkeys to another.
    let certificate = keys.get(CERTIFICATE, INSIDE, &[&session]);
    assert!(certificate.body.contains(PASSKEYS));
    let csp = page.header("content-security-policy").unwrap();
    assert!(csp.contains("script-src 'self'"));
}

#[test]
fn a_password_typed_inside_the_network_adds_no_passkey() {
    let keys = keys(&owner_rules(), |_| {});
    let mut device = Device::new();
    keys.add(&mut device, "laptop");
    let session = inside_session(&keys);
    // The certificate is the policy's first way in, so the passkey is its tab.
    let page = keys.get(&format!("{PASSKEYS}?method=passkey"), INSIDE, &[&session]);
    assert!(
        page.body.contains("show who you are again")
            && page.body.contains("data-passkey=\"get\"")
            && !page.body.contains("name=\"password\"")
            && !page.body.contains("data-passkey=\"create\""),
        "{}",
        page.body
    );
    // Nothing begins, whatever the browser sends.
    assert_eq!(
        keys.post(PASSKEYS, INSIDE, &[&session], "action=begin")
            .status,
        403
    );
    // A password is not what the rule asks to be shown again, even typed right.
    let typed = keys.post(
        PASSKEYS,
        INSIDE,
        &[&session],
        &format!(
            "action=step&method=password&user={}&password={PASSWORD}",
            keys.name
        ),
    );
    assert_eq!(typed.status, 403);
    // An answer with nothing begun is no registration.
    let confirm = keys.post(
        PASSKEYS,
        INSIDE,
        &[&session],
        "action=confirm&label=x&answer=%7B%22id%22%3A%22x%22%7D",
    );
    assert_eq!(confirm.status, 403);
    assert_eq!(keys.held().len(), 1);
    // The same from outside, where no password is offered at all.
    let outside = keys.get(PASSKEYS, OUTSIDE, &[&session]);
    assert_eq!(
        outside.status, 303,
        "a password session does not stand there"
    );
}

#[test]
fn a_passkey_is_added_after_showing_one_again_and_the_next_needs_it_again() {
    let keys = keys(&owner_rules(), |_| {});
    let mut device = Device::new();
    keys.add(&mut device, "laptop");
    let session = inside_session(&keys);
    let pending = show_again(&keys, &mut device, &session, INSIDE);
    // Begun: the options a browser creates a passkey from, for this account.
    let begun = keys.post(PASSKEYS, INSIDE, &[&session, &pending], "action=begin");
    assert_eq!(begun.status, 200, "{}", begun.body);
    let options: serde_json::Value = serde_json::from_str(&options_of(&begun)).unwrap();
    assert_eq!(options["publicKey"]["rp"]["id"], RP);
    assert_eq!(options["publicKey"]["user"]["name"], keys.name);
    assert_eq!(
        options["publicKey"]["excludeCredentials"]
            .as_array()
            .map(Vec::len),
        Some(1),
        "the device that holds one is not asked for another"
    );
    // A phone registers, and the page confirms it.
    let mut phone = Device::new();
    let answer = phone.register(ORIGIN, &options_of(&begun)).unwrap();
    let confirmed = keys.post(
        PASSKEYS,
        INSIDE,
        &[&session, &pending],
        &format!("action=confirm&label=phone&answer={}", urlencoded(&answer)),
    );
    assert_eq!(confirmed.status, 200, "{}", confirmed.body);
    assert!(confirmed.body.contains("This passkey now signs you in."));
    let held = keys.held();
    assert_eq!(held.len(), 2);
    assert!(
        held.iter()
            .any(|each| each.label.as_deref() == Some("phone"))
    );
    // The steps were used up with it: another needs them shown again.
    assert_eq!(
        keys.post(PASSKEYS, INSIDE, &[&session, &pending], "action=begin")
            .status,
        403
    );
    // The phone's passkey signs in, from outside, with no name typed.
    let by_phone = keys.passkey_in(&mut phone, OUTSIDE, &[]);
    assert_eq!(by_phone.status, 303, "{}", by_phone.body);
}

#[test]
fn what_one_session_showed_again_is_not_another_sessions() {
    let keys = keys(&owner_rules(), |_| {});
    let mut device = Device::new();
    keys.add(&mut device, "laptop");
    let mine_ = inside_session(&keys);
    let pending = show_again(&keys, &mut device, &mine_, INSIDE);
    let other = inside_session(&keys);
    assert_ne!(mine_, other);
    assert_eq!(
        keys.post(PASSKEYS, INSIDE, &[&other, &pending], "action=begin")
            .status,
        403,
        "another session holding the cookie of the steps"
    );
    assert_eq!(
        keys.post(PASSKEYS, INSIDE, &[&mine_, &pending], "action=begin")
            .status,
        200
    );
}

#[test]
fn a_session_a_passkey_opened_can_take_a_way_in_out_and_never_put_one_in() {
    let keys = keys(&owner_rules(), |_| {});
    let mut device = Device::new();
    let id = keys.add(&mut device, "laptop");
    let session = keys.session_by_passkey(&mut device, OUTSIDE);
    let page = keys.get(PASSKEYS, OUTSIDE, &[&session]);
    assert_eq!(page.status, 200);
    assert!(page.body.contains("show who you are again"));
    // A cookie alone adds nothing: begin and confirm are refused, and a challenge for the step is the account's own, which
    // only the device that holds the passkey can answer.
    assert_eq!(
        keys.post(PASSKEYS, OUTSIDE, &[&session], "action=begin")
            .status,
        403
    );
    // It may rename and remove: removing the passkey it was made with ends it.
    let renamed = keys.post(
        PASSKEYS,
        OUTSIDE,
        &[&session],
        &format!("action=label&id={}&label=desk", urlencoded(&id)),
    );
    assert!(
        renamed.body.contains("The passkey is renamed."),
        "{}",
        renamed.body
    );
    assert_eq!(keys.held()[0].label.as_deref(), Some("desk"));
    let removed = keys.post(
        PASSKEYS,
        OUTSIDE,
        &[&session],
        &format!("action=remove&id={}", urlencoded(&id)),
    );
    assert!(
        removed.body.contains("The passkey is removed."),
        "{}",
        removed.body
    );
    assert!(keys.held().is_empty());
    assert_eq!(index_at(&keys.rig, &session, Some(OUTSIDE)).status, 401);
}

#[test]
fn removing_a_passkey_ends_the_sessions_it_opened_and_only_those() {
    let keys = keys(&owner_rules(), |_| {});
    let (mut laptop, mut phone) = (Device::new(), Device::new());
    let laptop_id = keys.add(&mut laptop, "laptop");
    keys.add(&mut phone, "phone");
    let by_laptop = keys.session_by_passkey(&mut laptop, OUTSIDE);
    let by_phone = keys.session_by_passkey(&mut phone, OUTSIDE);
    let inside = inside_session(&keys);
    for session in [&by_laptop, &by_phone] {
        assert_eq!(index_at(&keys.rig, session, Some(OUTSIDE)).status, 200);
    }
    // The phone's session takes the laptop's passkey away.
    let removed = keys.post(
        PASSKEYS,
        OUTSIDE,
        &[&by_phone],
        &format!("action=remove&id={}", urlencoded(&laptop_id)),
    );
    assert!(
        removed.body.contains("The passkey is removed."),
        "{}",
        removed.body
    );
    assert_eq!(index_at(&keys.rig, &by_laptop, Some(OUTSIDE)).status, 401);
    assert_eq!(index_at(&keys.rig, &by_phone, Some(OUTSIDE)).status, 200);
    assert_eq!(index_at(&keys.rig, &inside, Some(INSIDE)).status, 200);
    // It signs no one in any more.
    assert_eq!(keys.passkey_in(&mut laptop, OUTSIDE, &[]).status, 401);
    // A passkey that is not the account's is no removal.
    let none = keys.post(
        PASSKEYS,
        OUTSIDE,
        &[&by_phone],
        "action=remove&id=not-a-passkey",
    );
    assert!(none.body.contains("not on your account"), "{}", none.body);
    assert_eq!(keys.held().len(), 1);
}

#[test]
fn an_administrators_removal_ends_a_session_it_opened_at_its_next_use() {
    let keys = keys(&owner_rules(), |_| {});
    let mut device = Device::new();
    keys.add(&mut device, "laptop");
    let session = keys.session_by_passkey(&mut device, OUTSIDE);
    assert_eq!(index_at(&keys.rig, &session, Some(OUTSIDE)).status, 200);
    assert_eq!(
        keys.rig.accounts.remove_passkeys_of(&keys.name, None),
        Ok(1)
    );
    assert_eq!(index_at(&keys.rig, &session, Some(OUTSIDE)).status, 401);
    let page = keys.get(PASSKEYS, OUTSIDE, &[&session]);
    assert_eq!(
        page.status, 303,
        "a session that ended is a browser to sign in again"
    );
}

#[test]
fn with_no_enrol_rule_nobody_adds_a_passkey_and_the_page_says_so() {
    let rules = serde_json::json!({"rule": [{"allow": [["password"], ["passkey"]]}]});
    let keys = keys(&rules, |_| {});
    let session = inside_session(&keys);
    let page = keys.get(PASSKEYS, INSIDE, &[&session]);
    assert_eq!(page.status, 200);
    assert!(page.body.contains("ask an administrator"), "{}", page.body);
    assert_eq!(
        keys.post(PASSKEYS, INSIDE, &[&session], "action=begin")
            .status,
        403
    );
    assert!(keys.held().is_empty());
}

#[test]
fn a_registration_nobody_began_is_refused_with_a_reason_and_a_change_needs_the_page_it_was_sent_from()
 {
    let keys = keys(&owner_rules(), |_| {});
    let mut device = Device::new();
    keys.add(&mut device, "laptop");
    let session = inside_session(&keys);
    let pending = show_again(&keys, &mut device, &session, INSIDE);
    let none = keys.post(
        PASSKEYS,
        INSIDE,
        &[&session, &pending],
        "action=confirm&label=x&answer=%7B%22id%22%3A%22x%22%7D",
    );
    assert_eq!(none.status, 400);
    assert!(
        none.body.contains("No passkey was being added"),
        "{}",
        none.body
    );
    // An answer that is not a passkey's is refused as one.
    let begun = keys.post(PASSKEYS, INSIDE, &[&session, &pending], "action=begin");
    assert_eq!(begun.status, 200);
    let junk = keys.post(
        PASSKEYS,
        INSIDE,
        &[&session, &pending],
        "action=confirm&label=x&answer=%7B%22id%22%3A%22x%22%7D",
    );
    assert_eq!(junk.status, 403, "{}", junk.body);
    assert!(junk.body.contains("That passkey was not accepted."));
    assert_eq!(keys.held().len(), 1);
    // A change from another site's page, with no cookie, or in another encoding is no change.
    let to = proxied(&keys.rig);
    let id = urlencoded(&keys.held()[0].id);
    let body = format!("action=remove&id={id}");
    for (extra, status) in [
        (
            vec![
                ("Origin", "https://evil.example".to_owned()),
                ("Cookie", session.clone()),
            ],
            403,
        ),
        (vec![("Cookie", session.clone())], 403),
        (vec![("Origin", to.origin())], 401),
        (
            vec![
                ("Origin", to.origin()),
                ("Cookie", session.clone()),
                ("Content-Type", "application/json".to_owned()),
            ],
            400,
        ),
    ] {
        let mut headers = extra;
        headers.push(("X-Forwarded-For", INSIDE.to_owned()));
        if !headers.iter().any(|(name, _)| *name == "Content-Type") {
            headers.push((
                "Content-Type",
                "application/x-www-form-urlencoded".to_owned(),
            ));
        }
        let answer = request(&to, "POST", PASSKEYS, &headers, &body);
        assert_eq!(answer.status, status, "{headers:?}");
    }
    assert_eq!(keys.held().len(), 1);
}

/// Serves the portal over the real password and passkey providers on a loopback port, for the browser run
/// `clients/browser/chromium/e2e/passkey.mjs`, until the file named by `CANOA_TEST_SERVE_UNTIL` exists:
/// `CANOA_TEST_SERVE_UNTIL=/tmp/stop cargo test -p canoa-host --features portal --lib serve_for_a_browser -- --ignored --nocapture`.
/// The loopback network stands where 10.0.0.0/8 does, a browser's port is any port, and a password shown again may add the first
/// passkey, which no passkey can yet.
#[test]
#[ignore = "serves until a file exists, for a browser run"]
fn serve_a_passkey_portal_for_a_browser() {
    let until =
        PathBuf::from(std::env::var("CANOA_TEST_SERVE_UNTIL").expect("the file that ends the run"));
    let rules = serde_json::json!({"rule": [
        {"purposes": ["enrol"], "allow": [["password"], ["passkey"]]},
        {"networks": ["127.0.0.0/8"], "allow": [["password"], ["passkey"]]},
        {"allow": [["passkey"]]},
    ]});
    let keys = keys_with(
        &rules,
        &serde_json::json!({"allow_any_port": true}),
        |settings| {
            settings.max_failures_per_pair = 50;
            settings.max_failures_per_address = 500;
        },
    );
    println!(
        "{}",
        serde_json::json!({
            "serving": {
                "url": format!("http://localhost:{}", keys.rig.addr.port()),
                "user": keys.name,
                "password": PASSWORD,
            }
        })
    );
    while !until.exists() {
        thread::sleep(Duration::from_millis(200));
    }
}

/// The rules as the portal's settings file writes them for a portal that takes a password, a certificate or a passkey inside
/// 10.0.0.0/8, a certificate or a passkey alone elsewhere, and a password only with a one-time code there.
const OWNER_TOML: &str = r#"
[[sign_in_policy.config.rule]]
purposes = ["enrol"]
allow = [["certificate"], ["passkey"], ["password", "totp"]]

[[sign_in_policy.config.rule]]
networks = ["10.0.0.0/8"]
allow = [["password"], ["certificate"], ["passkey"]]

[[sign_in_policy.config.rule]]
allow = [["certificate"], ["passkey"], ["password", "totp"]]
"#;

#[test]
fn the_rules_a_portal_settings_file_writes_for_passkeys_decide_as_the_guide_says() {
    let settings = Settings::from_toml(OWNER_TOML).unwrap();
    let policy = policy::Policy::new(
        &settings,
        &[
            Method::Password,
            Method::Certificate,
            Method::Totp,
            Method::RecoveryCode,
            Method::Passkey,
        ],
    )
    .unwrap();
    assert_eq!(policy.describe(), "3 rules");
    let me = || Some(policy::person("alice", 1000, &[]));
    let step = |method, provider: &str| policy::factor(method, provider, 1000);
    let decide = |purpose: &str, from: &str, done: Vec<coqui_access::Factor>| {
        let address: Option<IpAddr> = from.parse().ok();
        let person = if done.is_empty() { None } else { me() };
        policy
            .decide(policy::context(purpose, address, person, done))
            .unwrap()
    };
    use coqui_access::purposes::{ENROL, SIGN_IN};
    // What the page offers first: a passkey from anywhere a certificate is, a password alone only from inside.
    assert_eq!(
        decide(SIGN_IN, "10.6.20.5", vec![]).methods(),
        [Method::Password, Method::Certificate, Method::Passkey]
    );
    for outside in ["203.0.113.7", "2001:db8::1", "unknown"] {
        assert_eq!(
            decide(SIGN_IN, outside, vec![]).methods(),
            [Method::Certificate, Method::Passkey, Method::Password],
            "{outside}: the password that leads to a code"
        );
    }
    // A passkey is enough alone, from inside and from outside; a password is enough alone only inside.
    for from in ["10.6.20.5", "203.0.113.7"] {
        assert!(decide(SIGN_IN, from, vec![step(Method::Passkey, "p")]).complete);
    }
    assert!(decide(SIGN_IN, "10.6.20.5", vec![step(Method::Password, "p")]).complete);
    let outside = decide(SIGN_IN, "203.0.113.7", vec![step(Method::Password, "p")]);
    assert!(!outside.complete && outside.methods() == [Method::Totp]);
    assert!(
        decide(
            SIGN_IN,
            "203.0.113.7",
            vec![step(Method::Password, "p"), step(Method::Totp, "t")]
        )
        .complete
    );
    // Adding a passkey needs a certificate, a passkey, or a password and a code, from inside as from outside: a password alone,
    // the one thing inside the network accepts, is not enough.
    for from in ["10.6.20.5", "203.0.113.7"] {
        let password = decide(ENROL, from, vec![step(Method::Password, "p")]);
        assert!(!password.complete, "{from}");
        assert_eq!(password.methods(), [Method::Totp], "{from}");
        assert!(decide(ENROL, from, vec![step(Method::Passkey, "p")]).complete);
        assert!(decide(ENROL, from, vec![step(Method::Certificate, "c")]).complete);
        assert!(
            decide(
                ENROL,
                from,
                vec![step(Method::Password, "p"), step(Method::Totp, "t")]
            )
            .complete
        );
    }
}
