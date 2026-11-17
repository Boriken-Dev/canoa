//! The second step of a sign-in through the portal's own request handling: the pending sign-in between steps, its time and tries,
//! the counts that bound guessing a code, setting a code up (who may, and that nobody else can) and taking it away.
//!
//! The first half runs stand-in accounts that count each time they are asked. The second runs the real one-time code provider,
//! built beside the tests, over a file, with codes computed from the secret an authenticator app would be given.

use std::sync::OnceLock;

use std::path::Path;

use hmac::{Hmac, KeyInit, Mac as _};

use super::sign_in_policy::{
    Flow, flow_rig_apart, form_holding, index_at, methods_holding, proxied, set_to,
};
use super::*;

/// The sign-in a code follows: a password and then a code.
fn two_steps() -> serde_json::Value {
    serde_json::json!({"rule": [{"allow": [["password", "totp"], ["password", "recovery_code"]]}]})
}

fn two_step_rig(change: impl FnOnce(&mut Settings)) -> Rig<Flow> {
    flow_rig_apart(&two_steps(), change)
}

fn cookie_of(token: &str) -> String {
    format!("{}={token}", steps::PENDING)
}

/// A sign-in at its password step: the pending cookie the browser now holds.
fn first_step(rig: &Rig<Flow>, from: &str) -> String {
    let answer = form_holding(
        rig,
        Some(from),
        None,
        None,
        "method=password&user=alice&password=pw",
    );
    assert_eq!(answer.status, 303, "{}", answer.body);
    assert_eq!(answer.header("location"), Some(LOGIN));
    assert!(
        set_to(&answer, COOKIE).is_none(),
        "no session after a first step that is not enough"
    );
    cookie_of(&set_to(&answer, steps::PENDING).expect("a pending cookie"))
}

fn code_at(rig: &Rig<Flow>, from: &str, pending: &str, code: &str) -> Answer {
    form_holding(
        rig,
        Some(from),
        None,
        Some(pending),
        &format!("method=totp&code={code}"),
    )
}

fn calls(rig: &Rig<Flow>) -> usize {
    rig.accounts.asked.lock().unwrap().len()
}

#[test]
fn a_sign_in_that_waits_too_long_ends_and_the_right_code_is_then_not_asked() {
    let rig = two_step_rig(|settings| settings.pending = Duration::from_millis(300));
    let pending = first_step(&rig, "10.1.2.3");
    assert_eq!(
        methods_holding(&rig, Some("10.1.2.3"), Some(&pending))
            .0
            .as_deref(),
        Some("next")
    );
    thread::sleep(Duration::from_millis(450));
    let before = calls(&rig);
    let late = code_at(&rig, "10.1.2.3", &pending, "123456");
    assert_eq!(
        late.status, 403,
        "the code is not offered to a sign-in that is gone"
    );
    assert!(late.cookie().is_none());
    assert_eq!(calls(&rig), before, "nobody was asked");
    assert_eq!(
        methods_holding(&rig, Some("10.1.2.3"), Some(&pending)).0,
        None,
        "the browser is back at the first step"
    );
}

#[test]
fn a_sign_in_allows_so_many_tries_then_ends_without_asking_about_the_next_even_if_it_is_right() {
    let rig = two_step_rig(|settings| {
        settings.max_pending_tries = 2;
        settings.max_factor_failures_per_person = 100;
    });
    let pending = first_step(&rig, "10.1.2.3");
    for _ in 0..2 {
        assert_eq!(code_at(&rig, "10.1.2.3", &pending, "000000").status, 401);
    }
    let before = calls(&rig);
    let third = code_at(&rig, "10.1.2.3", &pending, "123456");
    assert_eq!(third.status, 401);
    assert!(
        third.body.contains("timed out"),
        "the page says to start again"
    );
    assert!(third.cookie().is_none(), "no session");
    assert_eq!(calls(&rig), before, "the right code was never checked");
    // The cookie names nothing now, and the cleared cookie is sent.
    assert!(
        third
            .headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("set-cookie")
                && value.starts_with(&format!("{}=;", steps::PENDING)))
    );
    assert_eq!(
        code_at(&rig, "10.1.2.3", &pending, "123456").status,
        403,
        "a code with no sign-in waiting is not offered"
    );
}

#[test]
fn starting_over_does_not_give_back_the_guesses_a_person_has_failed() {
    let rig = two_step_rig(|settings| {
        settings.max_pending_tries = 2;
        settings.max_factor_failures_per_person = 3;
    });
    let first = first_step(&rig, "10.1.2.3");
    for _ in 0..2 {
        assert_eq!(code_at(&rig, "10.1.2.3", &first, "000000").status, 401);
    }
    // A new sign-in, as the page's "start over" makes: its pending state has all its tries, the person has one failure left.
    let second = first_step(&rig, "10.1.2.3");
    assert_eq!(code_at(&rig, "10.1.2.3", &second, "000000").status, 401);
    let before = calls(&rig);
    let right = code_at(&rig, "10.1.2.3", &second, "123456");
    assert_eq!(right.status, 401, "three failures stop the fourth try");
    assert!(right.cookie().is_none());
    assert_eq!(calls(&rig), before, "and the code is not checked");
}

#[test]
fn a_person_who_is_guessed_at_from_many_addresses_is_refused_whichever_address_asks() {
    let rig = two_step_rig(|settings| settings.max_factor_failures_per_person = 3);
    for index in 0..3 {
        let from = format!("10.1.2.{index}");
        let pending = first_step(&rig, &from);
        assert_eq!(code_at(&rig, &from, &pending, "000000").status, 401);
    }
    // An address that has failed nothing, with the password and then the right code.
    let pending = first_step(&rig, "10.1.2.200");
    let before = calls(&rig);
    let right = code_at(&rig, "10.1.2.200", &pending, "123456");
    assert_eq!(right.status, 401);
    assert!(right.cookie().is_none());
    assert_eq!(
        calls(&rig),
        before,
        "the third party's authenticator was not asked"
    );
    assert_eq!(
        rig.inner
            .throttle
            .failures_from("10.1.2.200".parse().unwrap()),
        1,
        "the refused try counts against the address that made it"
    );
}

#[test]
fn a_persons_failures_are_forgotten_when_the_window_passes() {
    let rig = two_step_rig(|settings| {
        settings.max_factor_failures_per_person = 2;
        settings.failure_window = Duration::from_millis(400);
    });
    let pending = first_step(&rig, "10.1.2.3");
    for _ in 0..2 {
        assert_eq!(code_at(&rig, "10.1.2.3", &pending, "000000").status, 401);
    }
    assert_eq!(code_at(&rig, "10.1.2.3", &pending, "123456").status, 401);
    thread::sleep(Duration::from_millis(500));
    let answer = code_at(&rig, "10.1.2.3", &pending, "123456");
    assert_eq!(answer.status, 303, "{}", answer.body);
    assert!(set_to(&answer, COOKIE).is_some());
}

#[test]
fn the_address_and_account_pair_stops_guessing_with_a_too_many_failures_answer() {
    let rig = two_step_rig(|settings| {
        settings.max_failures_per_pair = 2;
        settings.max_factor_failures_per_person = 100;
    });
    let pending = first_step(&rig, "10.1.2.3");
    for _ in 0..2 {
        assert_eq!(code_at(&rig, "10.1.2.3", &pending, "000000").status, 401);
    }
    let before = calls(&rig);
    let stopped = code_at(&rig, "10.1.2.3", &pending, "123456");
    assert_eq!(stopped.status, 429);
    assert!(stopped.header("retry-after").is_some());
    assert_eq!(calls(&rig), before);
}

#[test]
fn a_pending_sign_in_belongs_to_the_browser_that_began_it() {
    let rig = two_step_rig(|_| {});
    let mine_cookie = first_step(&rig, "10.1.2.3");
    let before = calls(&rig);
    // Another browser, from the same address, with the right code and no cookie.
    let other = form_holding(
        &rig,
        Some("10.1.2.3"),
        None,
        None,
        "method=totp&code=123456",
    );
    assert_eq!(other.status, 403);
    // A cookie that is made up names nothing.
    let made_up = code_at(
        &rig,
        "10.1.2.3",
        &cookie_of("A".repeat(43).as_str()),
        "123456",
    );
    assert_eq!(made_up.status, 403);
    assert_eq!(calls(&rig), before);
    // The first step cannot be passed again while the pending sign-in only offers a code.
    let again = form_holding(
        &rig,
        Some("10.1.2.3"),
        None,
        Some(&mine_cookie),
        "method=password&user=alice&password=pw",
    );
    assert_eq!(again.status, 403);
    assert_eq!(calls(&rig), before);
}

#[test]
fn starting_over_ends_the_pending_sign_in() {
    let rig = two_step_rig(|_| {});
    let pending = first_step(&rig, "10.1.2.3");
    let page = request(
        &proxied(&rig),
        "GET",
        LOGIN,
        &[
            ("X-Forwarded-For", "10.1.2.3".to_owned()),
            ("Cookie", pending.clone()),
        ],
        "",
    );
    assert_eq!(page.status, 200);
    assert!(page.body.contains("Start over"));
    let cancelled = form_holding(
        &rig,
        Some("10.1.2.3"),
        None,
        Some(&pending),
        "method=cancel",
    );
    assert_eq!(cancelled.status, 303);
    assert_eq!(cancelled.header("location"), Some(LOGIN));
    assert_eq!(code_at(&rig, "10.1.2.3", &pending, "123456").status, 403);
    assert_eq!(
        methods_holding(&rig, Some("10.1.2.3"), Some(&pending)).0,
        None
    );
}

#[test]
fn the_page_of_the_next_step_asks_for_a_code_the_way_a_browser_fills_one_in() {
    let rig = flow_rig_apart(
        &serde_json::json!({"rule": [{"allow": [
            ["password", "totp"], ["password", "recovery_code"], ["password", "certificate"]
        ]}]}),
        |_| {},
    );
    let pending = first_step(&rig, "10.1.2.3");
    let page = request(
        &proxied(&rig),
        "GET",
        LOGIN,
        &[
            ("X-Forwarded-For", "10.1.2.3".to_owned()),
            ("Cookie", pending),
        ],
        "",
    );
    assert_eq!(page.status, 200);
    let body = &page.body;
    assert!(
        body.contains("autocomplete=\"one-time-code\"") && body.contains("inputmode=\"numeric\"")
    );
    assert!(body.contains("Use a recovery code"));
    assert!(body.contains("must present it") || body.contains("must present your certificate"));
    assert!(body.contains("value=\"cancel\""));
    assert!(
        !body.contains("name=\"password\""),
        "the first step is not asked again"
    );
    assert!(!body.contains("<script>") && !body.contains("onclick"));
    assert!(
        page.header("content-security-policy")
            .is_some_and(|policy| policy.contains("script-src 'self'"))
    );
    assert!(
        page.header("cache-control")
            .is_some_and(|value| value.contains("no-store"))
    );
}

#[test]
fn a_certificate_that_comes_with_the_password_is_tried_at_once_only_when_it_is_pinned_to_the_account()
 {
    let rig = flow_rig_apart(
        &serde_json::json!({"rule": [{"allow": [["password", "certificate"]]}]}),
        |_| {},
    );
    let same = form_holding(
        &rig,
        Some("10.1.2.3"),
        Some("alice"),
        None,
        "method=password&user=alice&password=pw",
    );
    assert_eq!((same.status, same.header("location")), (303, Some("/")));
    assert!(set_to(&same, COOKIE).is_some(), "one request, a session");
    // A certificate that is another person's, or no one's, is not tried: it would be a failure counted against a person who only
    // sent a password, and they are asked for the step instead.
    for carried in ["bob", "stranger"] {
        let before = calls(&rig);
        let counted_before = rig
            .inner
            .throttle
            .failures_from("10.1.2.3".parse().unwrap());
        let other = form_holding(
            &rig,
            Some("10.1.2.3"),
            Some(carried),
            None,
            "method=password&user=alice&password=pw",
        );
        assert_eq!(
            (other.status, other.header("location")),
            (303, Some(LOGIN)),
            "{carried}"
        );
        assert!(set_to(&other, COOKIE).is_none());
        assert!(
            set_to(&other, steps::PENDING).is_some(),
            "the sign-in goes on"
        );
        assert_eq!(
            calls(&rig),
            before + 1,
            "{carried}: only the password was asked"
        );
        assert_eq!(
            rig.inner
                .throttle
                .failures_from("10.1.2.3".parse().unwrap()),
            counted_before,
            "{carried}: nothing counted"
        );
    }
    let other = form_holding(
        &rig,
        Some("10.1.2.3"),
        Some("bob"),
        None,
        "method=password&user=alice&password=pw",
    );
    let pending = cookie_of(&set_to(&other, steps::PENDING).expect("the sign-in goes on"));
    let finished = form_holding(
        &rig,
        Some("10.1.2.3"),
        Some("alice"),
        Some(&pending),
        "method=certificate",
    );
    assert_eq!(finished.status, 303);
    assert!(set_to(&finished, COOKIE).is_some());
}

#[test]
fn a_session_that_a_code_opened_stands_at_resume_and_a_pending_sign_in_is_no_session() {
    let rig = two_step_rig(|_| {});
    let pending = first_step(&rig, "10.1.2.3");
    // Nothing behind the sign-in is given on a pending cookie.
    assert_eq!(
        request(
            &proxied(&rig),
            "GET",
            web::HOST_INDEX,
            &[("Cookie", pending.clone())],
            ""
        )
        .status,
        401
    );
    let done = code_at(&rig, "10.1.2.3", &pending, "123456");
    let session = format!("{COOKIE}={}", set_to(&done, COOKIE).unwrap());
    assert_eq!(index_at(&rig, &session, Some("10.1.2.3")).status, 200);
    // The pending cookie was cleared with the session.
    assert!(done.headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("set-cookie")
            && value.starts_with(&format!("{}=;", steps::PENDING))
    }));
}

// ---- the real one-time code provider ----

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// The library cargo built beside the test binary for the dev-dependency `name`.
fn plugin(name: &str) -> PathBuf {
    let prefix = format!("lib{name}_plugin-");
    std::fs::read_dir(exe_dir())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|file| {
            let name = file.file_name().to_string_lossy().into_owned();
            name.starts_with(&prefix) && name.ends_with(".so")
        })
        .max_by_key(|file| file.metadata().and_then(|meta| meta.modified()).ok())
        .map(|file| file.path())
        .unwrap_or_else(|| panic!("{name}'s plugin is built beside the tests"))
}

fn file_source(path: &Path) -> serde_json::Value {
    serde_json::json!({"source": {"file": {"path": path, "format": "toml"}}})
}

fn real_entry(provider: &str, config: &serde_json::Value) -> Entry {
    let mut entry = Entry::new(provider);
    entry.library = Some(plugin(provider));
    entry.config = config.as_object().cloned().unwrap_or_default();
    entry
}

/// The name of the account this test process runs as, which is the one the stand-in hosts run.
fn my_name() -> String {
    use coqui_access::Directory as _;
    use coqui_access::Lookup;
    let system = coqui_access_system::SystemAccounts::builder()
        .min_uid(0)
        .build()
        .unwrap();
    system
        .lookup(Lookup {
            by: "id".to_owned(),
            value: mine().to_string(),
            claims: None,
        })
        .unwrap()
        .expect("this process's user is in the directory")
        .name
}

/// The code an authenticator app shows `ahead` thirty-second steps from now, for the secret in a setup link.
fn code_in(link: &str, ahead: i64) -> String {
    let secret = link
        .split(['?', '&'])
        .find_map(|part| part.strip_prefix("secret="))
        .unwrap();
    let key = data_encoding::BASE32_NOPAD
        .decode(secret.as_bytes())
        .unwrap();
    let now = i64::try_from(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap();
    let mut mac = <Hmac<sha1::Sha1> as KeyInit>::new_from_slice(&key).unwrap();
    mac.update(&u64::try_from(now / 30 + ahead).unwrap().to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = usize::from(digest[19] & 0x0f);
    let number = u32::from_be_bytes([
        digest[offset] & 0x7f,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]);
    format!("{:06}", number % 1_000_000)
}

/// The portal over the real password file and one-time code providers, for the account this process runs as.
struct Real {
    rig: Rig<Access>,
    name: String,
    dir: PathBuf,
}

impl Drop for Real {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const PASSWORD: &str = "correct horse";

fn real(rules: &serde_json::Value, change: impl FnOnce(&mut Settings)) -> Real {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "canoa-second-{}-{}",
        std::process::id(),
        COUNT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let name = my_name();
    let passwords = dir.join("passwords.toml");
    // A password for the account, put through the provider's own account kind.
    {
        use coqui_access::guatiao::library::Registry;
        use coqui_access::{AccountPut, Accounts as AccountKind};
        let mut registry = Registry::new("seed", "0");
        registry.set_import_search(coqui_access::guatiao::library::ImportSearch::Library);
        registry
            .load_file(&plugin("coqui_access_password"))
            .unwrap();
        let config = access_value(&file_source(&passwords));
        let accounts: Arc<dyn AccountKind> = Arc::new(
            registry
                .offer::<dyn AccountKind>("coqui_access_password")
                .unwrap()
                .unwrap()
                .instantiate(&config)
                .unwrap(),
        );
        accounts
            .put(AccountPut {
                name: name.clone(),
                secret: Some(PASSWORD.to_owned()),
                ..AccountPut::default()
            })
            .unwrap();
    }
    let mut totp = file_source(&dir.join("totp.toml"));
    totp["skew"] = 10.into();
    totp["issuer"] = "Canoa Test".into();
    let own = Settings {
        min_uid: mine().max(1),
        authenticators: vec![
            real_entry("coqui_access_password", &file_source(&passwords)),
            real_entry("coqui_access_totp", &totp),
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
            change(settings);
        },
        0,
    );
    Real { rig, name, dir }
}

fn access_value(value: &serde_json::Value) -> coqui_access::guatiao::Value {
    access::value_of(value.as_object().unwrap()).unwrap()
}

impl Real {
    fn password_body(&self) -> String {
        format!("method=password&user={}&password={PASSWORD}", self.name)
    }

    fn form(&self, from: &str, cookies: &[&str], body: &str) -> Answer {
        let to = proxied(&self.rig);
        let mut extra = vec![
            ("Origin", to.origin()),
            (
                "Content-Type",
                "application/x-www-form-urlencoded".to_owned(),
            ),
            ("X-Forwarded-For", from.to_owned()),
        ];
        if !cookies.is_empty() {
            extra.push(("Cookie", cookies.join("; ")));
        }
        request(&to, "POST", LOGIN, &extra, body)
    }

    fn factor(&self, from: &str, cookies: &[&str], body: &str) -> Answer {
        let to = proxied(&self.rig);
        let mut extra = vec![
            ("Origin", to.origin()),
            (
                "Content-Type",
                "application/x-www-form-urlencoded".to_owned(),
            ),
            ("X-Forwarded-For", from.to_owned()),
        ];
        if !cookies.is_empty() {
            extra.push(("Cookie", cookies.join("; ")));
        }
        request(&to, "POST", FACTOR, &extra, body)
    }

    fn factor_page(&self, from: &str, cookies: &[&str]) -> Answer {
        let mut extra = vec![("X-Forwarded-For", from.to_owned())];
        if !cookies.is_empty() {
            extra.push(("Cookie", cookies.join("; ")));
        }
        request(&proxied(&self.rig), "GET", FACTOR, &extra, "")
    }

    /// A password sign-in; the cookie it sets and whether it is a session.
    fn password_step(&self, from: &str) -> Answer {
        self.form(from, &[], &self.password_body())
    }

    fn uid(&self) -> u32 {
        mine()
    }
}

fn session_cookie(answer: &Answer) -> String {
    format!("{COOKIE}={}", set_to(answer, COOKIE).expect("a session"))
}

fn pending_cookie_of(answer: &Answer) -> String {
    cookie_of(&set_to(answer, steps::PENDING).expect("a pending sign-in"))
}

/// The `otpauth://` link a setup page shows.
fn link_in(page: &str) -> String {
    let start = page.find("otpauth://").expect("a setup link on the page");
    let end = page[start..].find('<').unwrap() + start;
    page[start..end].replace("&amp;", "&")
}

/// The recovery codes a setup page shows.
fn recovery_codes_in(page: &str) -> Vec<String> {
    page.split("<li>")
        .skip(1)
        .map(|part| part.split("</li>").next().unwrap().to_owned())
        .collect()
}

/// A rule set where a code follows the password, and a person with only the password may set a code up.
fn first_use_may_set_up() -> serde_json::Value {
    serde_json::json!({"rule": [
        {"purposes": ["enrol_first"], "allow": [["password"]]},
        {"allow": [["password", "totp"], ["password", "recovery_code"]]},
    ]})
}

#[test]
fn a_person_with_no_code_is_asked_to_set_one_up_and_signs_in_with_the_code_they_confirm() {
    let real = real(&first_use_may_set_up(), |_| {});
    let first = real.password_step("10.1.2.3");
    assert_eq!(first.status, 303);
    let pending = pending_cookie_of(&first);
    // The page of the pending sign-in offers to set one up, and nothing is waiting yet.
    let page = request(
        &proxied(&real.rig),
        "GET",
        LOGIN,
        &[("Cookie", pending.clone())],
        "",
    );
    assert!(page.body.contains("Set up a code"), "{}", page.body);
    assert_eq!(
        real.rig.accounts.factor_standing(&real.name, real.uid()),
        FactorStanding::NotEnrolled
    );
    // Begin: the link as text and as a drawing with no script, and the recovery codes once.
    let begun = real.factor("10.1.2.3", &[&pending], "action=begin");
    assert_eq!(begun.status, 200, "{}", begun.body);
    assert!(begun.body.contains("<svg class=\"qr\""));
    assert!(!begun.body.contains("<script"));
    assert!(
        begun
            .header("cache-control")
            .is_some_and(|value| value.contains("no-store"))
    );
    let link = link_in(&begun.body);
    assert_eq!(recovery_codes_in(&begun.body).len(), 10);
    // Still not a factor, and a wrong code does not make it one.
    assert_eq!(
        real.rig.accounts.factor_standing(&real.name, real.uid()),
        FactorStanding::NotEnrolled
    );
    let wrong = real.factor("10.1.2.3", &[&pending], "action=confirm&code=000000");
    assert_eq!(wrong.status, 401);
    assert_eq!(
        real.rig.accounts.factor_standing(&real.name, real.uid()),
        FactorStanding::NotEnrolled
    );
    // The first code from the app confirms it, and that is the step the sign-in waited for.
    let done = real.factor(
        "10.1.2.3",
        &[&pending],
        &format!("action=confirm&code={}", code_in(&link, 0)),
    );
    assert_eq!(
        (done.status, done.header("location")),
        (303, Some("/")),
        "{}",
        done.body
    );
    let session = session_cookie(&done);
    assert_eq!(index_at(&real.rig, &session, Some("10.1.2.3")).status, 200);
    assert_eq!(
        real.rig.accounts.factor_standing(&real.name, real.uid()),
        FactorStanding::Enrolled
    );
    // The next sign-in asks for the code, and the code from the app works.
    let again = real.password_step("10.1.2.3");
    let pending = pending_cookie_of(&again);
    let no_setup = request(
        &proxied(&real.rig),
        "GET",
        LOGIN,
        &[("Cookie", pending.clone())],
        "",
    );
    assert!(!no_setup.body.contains("Set up a code"));
    let signed = real.form(
        "10.1.2.3",
        &[&pending],
        &format!("method=totp&code={}", code_in(&link, 1)),
    );
    assert_eq!(signed.status, 303, "{}", signed.body);
    assert_eq!(
        index_at(&real.rig, &session_cookie(&signed), Some("10.1.2.3")).status,
        200
    );
}

#[test]
fn an_administrator_resets_a_code_and_the_next_sign_in_sets_one_up_again_while_a_session_that_used_it_ends()
 {
    let real = real(&first_use_may_set_up(), |_| {});
    let pending = pending_cookie_of(&real.password_step("10.1.2.3"));
    let begun = real.factor("10.1.2.3", &[&pending], "action=begin");
    let link = link_in(&begun.body);
    let session = session_cookie(&real.factor(
        "10.1.2.3",
        &[&pending],
        &format!("action=confirm&code={}", code_in(&link, 0)),
    ));
    assert_eq!(index_at(&real.rig, &session, Some("10.1.2.3")).status, 200);
    // The administrator's reset, as `canoa portal factor reset` does it.
    assert_eq!(
        real.rig.accounts.reset_factor(&real.name, real.uid()),
        Ok(true)
    );
    assert_eq!(
        real.rig.accounts.reset_factor(&real.name, real.uid()),
        Ok(false)
    );
    assert_eq!(
        index_at(&real.rig, &session, Some("10.1.2.3")).status,
        401,
        "a session made with the code does not outlive it"
    );
    // The account is the administrator's now: a password does not set a code up again, even where a rule lets a first one be.
    assert_eq!(
        real.rig.accounts.factor_standing(&real.name, real.uid()),
        FactorStanding::Reset
    );
    // The recovery codes went with it.
    let pending = pending_cookie_of(&real.password_step("10.1.2.3"));
    let page = request(
        &proxied(&real.rig),
        "GET",
        LOGIN,
        &[("Cookie", pending.clone())],
        "",
    );
    assert!(
        !page.body.contains("Set up a code") && page.body.contains("ask an administrator"),
        "{}",
        page.body
    );
    for body in ["action=begin", "action=confirm&code=123456"] {
        let answer = real.factor("10.1.2.3", &[&pending], body);
        assert_eq!(answer.status, 403, "{body}");
        assert!(!answer.body.contains("otpauth://"));
    }
    assert_eq!(
        real.rig.accounts.factor_standing(&real.name, real.uid()),
        FactorStanding::Reset,
        "nothing was begun"
    );
    let recovery = recovery_codes_in(&begun.body).remove(0);
    assert_eq!(
        real.form(
            "10.1.2.3",
            &[&pending],
            &format!("method=recovery_code&code={recovery}")
        )
        .status,
        401
    );
}

#[test]
fn with_no_enrol_rule_nobody_sets_a_code_up_for_themselves_and_nothing_is_begun() {
    let real = real(
        &serde_json::json!({"rule": [{"allow": [["password", "totp"]]}]}),
        |_| {},
    );
    let pending = pending_cookie_of(&real.password_step("10.1.2.3"));
    let page = request(
        &proxied(&real.rig),
        "GET",
        LOGIN,
        &[("Cookie", pending.clone())],
        "",
    );
    assert!(!page.body.contains("Set up a code") && page.body.contains("ask an administrator"));
    for body in [
        "action=begin",
        "action=confirm&code=123456",
        "action=step&method=password",
    ] {
        let answer = real.factor("10.1.2.3", &[&pending], body);
        assert!(answer.status == 403, "{body}: {}", answer.status);
        assert!(!answer.body.contains("otpauth://"));
    }
    let shown = real.factor_page("10.1.2.3", &[&pending]);
    assert!(
        shown.body.contains("administrator")
            && !shown.body.contains("name=\"action\" value=\"begin\"")
    );
    assert_eq!(
        real.rig.accounts.factor_standing(&real.name, real.uid()),
        FactorStanding::NotEnrolled
    );
    assert!(
        !real.dir.join("totp.toml.pending").exists()
            || std::fs::read_to_string(real.dir.join("totp.toml.pending"))
                .unwrap()
                .trim()
                .is_empty(),
        "no setup was begun"
    );
}

#[test]
fn setting_a_code_up_needs_an_account_a_cookie_and_the_page_it_was_sent_from() {
    let real = real(&first_use_may_set_up(), |_| {});
    // No cookie at all.
    for body in [
        "action=begin",
        "action=confirm&code=123456",
        "action=step&method=password",
    ] {
        assert_eq!(real.factor("10.1.2.3", &[], body).status, 401, "{body}");
    }
    assert_eq!(real.factor_page("10.1.2.3", &[]).status, 303);
    // A cookie that names nothing.
    let nothing = cookie_of(&"B".repeat(43));
    assert_eq!(
        real.factor("10.1.2.3", &[&nothing], "action=begin").status,
        401
    );
    // A pending sign-in's cookie begins a setup only from the site's own page.
    let pending = pending_cookie_of(&real.password_step("10.1.2.3"));
    let to = proxied(&real.rig);
    let cross = request(
        &to,
        "POST",
        FACTOR,
        &[
            (
                "Content-Type",
                "application/x-www-form-urlencoded".to_owned(),
            ),
            ("Cookie", pending.clone()),
            ("X-Forwarded-For", "10.1.2.3".to_owned()),
        ],
        "action=begin",
    );
    assert_eq!(cross.status, 403);
    let foreign = request(
        &to,
        "POST",
        FACTOR,
        &[
            ("Origin", "http://elsewhere.example".to_owned()),
            (
                "Content-Type",
                "application/x-www-form-urlencoded".to_owned(),
            ),
            ("Cookie", pending),
            ("X-Forwarded-For", "10.1.2.3".to_owned()),
        ],
        "action=begin",
    );
    assert_eq!(foreign.status, 403);
    assert_eq!(
        real.rig.accounts.factor_standing(&real.name, real.uid()),
        FactorStanding::NotEnrolled
    );
}

#[test]
fn a_person_with_a_code_cannot_replace_it_while_signing_in_with_the_password_alone() {
    let real = real(&first_use_may_set_up(), |_| {});
    let pending = pending_cookie_of(&real.password_step("10.1.2.3"));
    let link = link_in(&real.factor("10.1.2.3", &[&pending], "action=begin").body);
    real.factor(
        "10.1.2.3",
        &[&pending],
        &format!("action=confirm&code={}", code_in(&link, 0)),
    );
    assert_eq!(
        real.rig.accounts.factor_standing(&real.name, real.uid()),
        FactorStanding::Enrolled
    );
    // An attacker with the password reaches the second step and asks to set a code up instead of giving one.
    let pending = pending_cookie_of(&real.password_step("203.0.113.9"));
    let attempt = real.factor("203.0.113.9", &[&pending], "action=begin");
    assert_eq!(attempt.status, 403);
    assert!(!attempt.body.contains("otpauth://"));
    let confirm = real.factor("203.0.113.9", &[&pending], "action=confirm&code=123456");
    assert_eq!(confirm.status, 403);
    // The real code still works afterwards.
    let signed = real.form(
        "10.1.2.3",
        &[&pending],
        &format!("method=totp&code={}", code_in(&link, 1)),
    );
    assert_eq!(signed.status, 303);
}

#[test]
fn a_signed_in_person_shows_the_factors_the_enrol_rule_asks_again_before_they_may_set_a_code_up() {
    // Signed in by password; setting a code up needs the password typed again.
    let real = real(
        &serde_json::json!({"rule": [
            {"purposes": ["enrol"], "allow": [["password"]]},
            {"allow": [["password"]]},
        ]}),
        |_| {},
    );
    let session = session_cookie(&real.password_step("10.1.2.3"));
    let page = real.factor_page("10.1.2.3", &[&session]);
    assert_eq!(page.status, 200);
    assert!(
        page.body.contains("show who you are again") && page.body.contains("name=\"password\"")
    );
    // The cookie alone begins nothing.
    let refused = real.factor("10.1.2.3", &[&session], "action=begin");
    assert_eq!(refused.status, 403);
    assert!(!refused.body.contains("otpauth://"));
    assert_eq!(
        real.factor("10.1.2.3", &[&session], "action=confirm&code=123456")
            .status,
        403
    );
    // A wrong password is a failed try and shows nothing.
    let wrong = real.factor(
        "10.1.2.3",
        &[&session],
        &format!(
            "action=step&method=password&user={}&password=wrong",
            real.name
        ),
    );
    assert_eq!(wrong.status, 401);
    assert!(set_to(&wrong, steps::PENDING).is_none());
    assert_eq!(
        real.factor("10.1.2.3", &[&session], "action=begin").status,
        403
    );
    // The right one opens the setup, for a short time, to that session alone.
    let shown = real.factor(
        "10.1.2.3",
        &[&session],
        &format!(
            "action=step&method=password&user={}&password={PASSWORD}",
            real.name
        ),
    );
    assert_eq!(shown.status, 200, "{}", shown.body);
    let enrol = pending_cookie_of(&shown);
    assert!(shown.body.contains("value=\"begin\""));
    let begun = real.factor("10.1.2.3", &[&session, &enrol], "action=begin");
    assert_eq!(begun.status, 200);
    let link = link_in(&begun.body);
    // Another session of the same person did not show the password: the enrol cookie is theirs alone.
    let other = session_cookie(&real.password_step("10.1.2.4"));
    assert_eq!(
        real.factor("10.1.2.4", &[&other, &enrol], "action=begin")
            .status,
        403
    );
    let done = real.factor(
        "10.1.2.3",
        &[&session, &enrol],
        &format!("action=confirm&code={}", code_in(&link, 0)),
    );
    assert_eq!(done.status, 200, "{}", done.body);
    assert!(done.body.contains("Your code is set up"));
    assert_eq!(
        real.rig.accounts.factor_standing(&real.name, real.uid()),
        FactorStanding::Enrolled
    );
    // It was one use: the setup is closed again.
    assert_eq!(
        real.factor("10.1.2.3", &[&session, &enrol], "action=begin")
            .status,
        403
    );
}

#[test]
fn a_code_that_belongs_to_another_account_is_not_accepted_for_this_one() {
    let real = real(&first_use_may_set_up(), |_| {});
    // Someone else has a code set up: their account is another user id.
    let other = mine() + 1;
    let begun = real
        .rig
        .accounts
        .begin_factor("someone", other, false)
        .unwrap();
    real.rig
        .accounts
        .confirm_factor("someone", other, &code_in(&begun.uri, 0))
        .unwrap();
    assert_eq!(
        real.rig.accounts.factor_standing("someone", other),
        FactorStanding::Enrolled
    );
    // This account has none; its sign-in is given the other's valid code.
    let pending = pending_cookie_of(&real.password_step("10.1.2.3"));
    let answer = real.form(
        "10.1.2.3",
        &[&pending],
        &format!("method=totp&code={}", code_in(&begun.uri, 1)),
    );
    assert_eq!(answer.status, 401, "{}", answer.body);
    assert!(set_to(&answer, COOKIE).is_none());
    // Nor a recovery code of theirs.
    let recovery = real.form(
        "10.1.2.3",
        &[&pending],
        &format!("method=recovery_code&code={}", begun.codes[0]),
    );
    assert_eq!(recovery.status, 401);
}

#[test]
fn wrong_codes_to_the_real_provider_stop_a_guesser_who_changes_address_and_the_right_code_then_fails()
 {
    let real = real(&first_use_may_set_up(), |settings| {
        settings.max_factor_failures_per_person = 3;
    });
    let pending = pending_cookie_of(&real.password_step("10.1.2.3"));
    let link = link_in(&real.factor("10.1.2.3", &[&pending], "action=begin").body);
    real.factor(
        "10.1.2.3",
        &[&pending],
        &format!("action=confirm&code={}", code_in(&link, 0)),
    );
    for index in 0..3 {
        let from = format!("10.9.9.{index}");
        let pending = pending_cookie_of(&real.password_step(&from));
        let wrong = real.form(&from, &[&pending], "method=totp&code=000000");
        assert_eq!(wrong.status, 401);
    }
    let from = "10.8.8.8";
    let pending = pending_cookie_of(&real.password_step(from));
    let right = real.form(
        from,
        &[&pending],
        &format!("method=totp&code={}", code_in(&link, 1)),
    );
    assert_eq!(
        right.status, 401,
        "three failures from other addresses stop this one"
    );
    assert!(set_to(&right, COOKIE).is_none());
}

// ---- nothing secret is written down ----

static LOG: OnceLock<Mutex<Vec<u8>>> = OnceLock::new();

#[derive(Clone, Copy)]
struct Capture;

impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        LOG.get_or_init(Mutex::default)
            .lock()
            .unwrap()
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        *self
    }
}

#[test]
fn neither_the_seed_nor_a_code_nor_the_password_is_written_to_the_log_or_an_error_page() {
    let installed = capture_installed();
    let real = real(&first_use_may_set_up(), |settings| {
        settings.max_failures_per_pair = 100;
        settings.max_failures_per_address = 1000;
    });
    let mut pages = String::new();
    let first = real.password_step("10.1.2.3");
    let pending = pending_cookie_of(&first);
    let begun = real.factor("10.1.2.3", &[&pending], "action=begin");
    let link = link_in(&begun.body);
    let secret = link
        .split(['?', '&'])
        .find_map(|part| part.strip_prefix("secret="))
        .unwrap()
        .to_owned();
    let recovery = recovery_codes_in(&begun.body);
    let wrong_confirm = real.factor("10.1.2.3", &[&pending], "action=confirm&code=135790");
    pages.push_str(&wrong_confirm.body);
    let done = real.factor(
        "10.1.2.3",
        &[&pending],
        &format!("action=confirm&code={}", code_in(&link, 0)),
    );
    // Failures of every kind with codes that are recognisable.
    let pending = pending_cookie_of(&real.password_step("10.1.2.3"));
    for body in [
        "method=totp&code=246801",
        "method=recovery_code&code=ZZZZZ-ZZZZZ",
        "method=password&user=nobody&password=hunter2-unique",
    ] {
        pages.push_str(&real.form("10.1.2.3", &[&pending], body).body);
    }
    let signed = real.form(
        "10.1.2.3",
        &[&pending],
        &format!("method=totp&code={}", code_in(&link, 2)),
    );
    assert_eq!(signed.status, 303);
    let used = recovery[0].clone();
    let pending = pending_cookie_of(&real.password_step("10.1.2.3"));
    let by_recovery = real.form(
        "10.1.2.3",
        &[&pending],
        &format!("method=recovery_code&code={used}"),
    );
    assert_eq!(by_recovery.status, 303);
    assert_eq!(done.status, 303);
    thread::sleep(Duration::from_millis(100));
    let log =
        String::from_utf8_lossy(&LOG.get_or_init(Mutex::default).lock().unwrap()).into_owned();
    if installed {
        assert!(log.contains("signed in"), "the log was captured");
        for secret_text in [
            secret.as_str(),
            link.as_str(),
            PASSWORD,
            "246801",
            "135790",
            "ZZZZZ-ZZZZZ",
            "hunter2-unique",
            used.as_str(),
            recovery[1].as_str(),
            code_in(&link, 1).as_str(),
            code_in(&link, 2).as_str(),
        ] {
            assert!(
                !log.contains(secret_text),
                "the log holds a secret: {secret_text}"
            );
        }
    }
    for secret_text in [
        secret.as_str(),
        "246801",
        "135790",
        "ZZZZZ-ZZZZZ",
        "hunter2-unique",
        PASSWORD,
    ] {
        assert!(
            !pages.contains(secret_text),
            "an error page echoes {secret_text}"
        );
    }
}

#[test]
fn a_setup_link_and_codes_are_only_in_the_one_answer_that_makes_them() {
    let real = real(&first_use_may_set_up(), |_| {});
    let pending = pending_cookie_of(&real.password_step("10.1.2.3"));
    let begun = real.factor("10.1.2.3", &[&pending], "action=begin");
    let link = link_in(&begun.body);
    let first_code = recovery_codes_in(&begun.body).remove(0);
    for answer in [
        real.factor_page("10.1.2.3", &[&pending]),
        request(
            &proxied(&real.rig),
            "GET",
            LOGIN,
            &[("Cookie", pending.clone())],
            "",
        ),
        request(
            &proxied(&real.rig),
            "GET",
            METHODS,
            &[("Cookie", pending.clone())],
            "",
        ),
    ] {
        assert!(!answer.body.contains(&link) && !answer.body.contains(&first_code));
    }
}

/// Serves the portal over the real password and one-time code providers on a loopback port, for the browser run
/// `clients/browser/chromium/e2e/portal_totp.mjs`, until the file named by `CANOA_TEST_SERVE_UNTIL` exists:
/// `CANOA_TEST_SERVE_UNTIL=/tmp/stop cargo test -p canoa-host --features portal --lib serve_for_a_browser -- --ignored --nocapture`.
#[test]
#[ignore = "serves until a file exists, for a browser run"]
fn serve_for_a_browser() {
    let until =
        PathBuf::from(std::env::var("CANOA_TEST_SERVE_UNTIL").expect("the file that ends the run"));
    let real = real(&first_use_may_set_up(), |settings| {
        settings.max_failures_per_pair = 50;
        settings.max_failures_per_address = 500;
        settings.session = Duration::from_secs(600);
    });
    println!(
        "{}",
        serde_json::json!({
            "serving": {
                "url": format!("http://localhost:{}", real.rig.addr.port()),
                "user": real.name,
                "password": PASSWORD,
            }
        })
    );
    while !until.exists() {
        thread::sleep(Duration::from_millis(200));
    }
}

// ---- setting a code up inside a sign-in is its own purpose ----

/// A policy where a signed-in person may set a code up with their password, and a sign-in may not.
fn enrol_but_not_first() -> serde_json::Value {
    serde_json::json!({"rule": [
        {"purposes": ["enrol"], "allow": [["password"]]},
        {"allow": [["password", "totp"], ["password", "recovery_code"]]},
    ]})
}

#[test]
fn a_rule_for_enrol_does_not_let_a_sign_in_set_a_code_up() {
    let real = real(&enrol_but_not_first(), |_| {});
    // Whoever holds the password is at the second step of an account that has no code.
    let pending = pending_cookie_of(&real.password_step("203.0.113.9"));
    let page = request(
        &proxied(&real.rig),
        "GET",
        LOGIN,
        &[("Cookie", pending.clone())],
        "",
    );
    assert!(
        !page.body.contains("Set up a code") && page.body.contains("ask an administrator"),
        "{}",
        page.body
    );
    for body in ["action=begin", "action=confirm&code=123456"] {
        let answer = real.factor("203.0.113.9", &[&pending], body);
        assert_eq!(answer.status, 403, "{body}");
        assert!(!answer.body.contains("otpauth://") && !answer.body.contains("svg class=\"qr\""));
    }
    assert_eq!(
        real.rig.accounts.factor_standing(&real.name, real.uid()),
        FactorStanding::NotEnrolled
    );
    assert!(
        !real.dir.join("totp.toml.pending").exists()
            || std::fs::read_to_string(real.dir.join("totp.toml.pending"))
                .unwrap()
                .trim()
                .is_empty(),
        "no setup was begun"
    );
}

#[test]
fn setting_a_code_up_in_a_sign_in_is_allowed_by_the_rule_that_names_it_where_it_names() {
    let real = real(
        &serde_json::json!({"rule": [
            {"purposes": ["enrol_first"], "networks": ["10.0.0.0/8"], "allow": [["password"]]},
            {"allow": [["password", "totp"]]},
        ]}),
        |_| {},
    );
    let outside = pending_cookie_of(&real.password_step("203.0.113.9"));
    let denied = real.factor("203.0.113.9", &[&outside], "action=begin");
    assert_eq!(denied.status, 403);
    assert!(!denied.body.contains("otpauth://"));
    let inside = pending_cookie_of(&real.password_step("10.1.2.3"));
    let begun = real.factor("10.1.2.3", &[&inside], "action=begin");
    assert_eq!(begun.status, 200, "{}", begun.body);
    assert!(begun.body.contains("otpauth://"));
}

#[test]
fn a_signed_in_person_is_asked_as_enrol_and_a_rule_for_enrol_first_does_not_let_them_in() {
    // The other way round: the sign-in rule names enrol_first only, so a session has no rule that lets it set a code up.
    let real = real(
        &serde_json::json!({"rule": [
            {"purposes": ["enrol_first"], "allow": [["password"]]},
            {"allow": [["password"]]},
        ]}),
        |_| {},
    );
    let session = session_cookie(&real.password_step("10.1.2.3"));
    let page = real.factor_page("10.1.2.3", &[&session]);
    assert!(page.body.contains("administrator"), "{}", page.body);
    assert_eq!(
        real.factor("10.1.2.3", &[&session], "action=begin").status,
        403
    );
}

#[test]
fn a_first_setup_during_a_sign_in_is_logged_at_warn() {
    let installed = capture_installed();
    let real = real(&first_use_may_set_up(), |_| {});
    let pending = pending_cookie_of(&real.password_step("10.1.2.3"));
    let begun = real.factor("10.1.2.3", &[&pending], "action=begin");
    let link = link_in(&begun.body);
    real.factor(
        "10.1.2.3",
        &[&pending],
        &format!("action=confirm&code={}", code_in(&link, 0)),
    );
    thread::sleep(Duration::from_millis(100));
    if installed {
        let log = logged();
        let line = log
            .lines()
            .find(|line| line.contains("during a sign-in") && line.contains("began setting up"))
            .expect("the setup is logged");
        assert!(line.contains("WARN"), "{line}");
        assert!(log.lines().any(|line| line.contains("WARN")
            && line.contains("set up a one-time code during a sign-in")));
    }
}

// ---- a reset reaches every use of a session ----

/// The administrator's way to give an account a new code, as `canoa portal factor enrol NAME` and then `--code` do it.
fn administrator_sets_up(real: &Real, ahead: i64) -> String {
    let accounts = &real.rig.accounts;
    let begun = accounts
        .begin_factor(&real.name, real.uid(), false)
        .unwrap();
    accounts
        .confirm_factor(&real.name, real.uid(), &code_in(&begun.uri, ahead))
        .unwrap();
    accounts.clear_reset(&real.name, real.uid()).unwrap();
    begun.uri
}

fn enrolled(real: &Real) -> (String, String) {
    let pending = pending_cookie_of(&real.password_step("10.1.2.3"));
    let link = link_in(&real.factor("10.1.2.3", &[&pending], "action=begin").body);
    real.factor(
        "10.1.2.3",
        &[&pending],
        &format!("action=confirm&code={}", code_in(&link, 0)),
    );
    let again = pending_cookie_of(&real.password_step("10.1.2.3"));
    let signed = real.form(
        "10.1.2.3",
        &[&again],
        &format!("method=totp&code={}", code_in(&link, 1)),
    );
    (session_cookie(&signed), link)
}

#[test]
fn a_code_set_up_again_does_not_carry_the_sessions_made_with_the_one_it_replaced() {
    let real = real(&first_use_may_set_up(), |_| {});
    let (session, _) = enrolled(&real);
    assert_eq!(index_at(&real.rig, &session, Some("10.1.2.3")).status, 200);
    // The administrator resets the account and gives it a new code before the session is next used: it stands in no state the
    // account is in now, since the code that made it is not the account's.
    thread::sleep(Duration::from_millis(1100));
    assert_eq!(
        real.rig.accounts.reset_factor(&real.name, real.uid()),
        Ok(true)
    );
    administrator_sets_up(&real, 0);
    assert_eq!(
        real.rig.accounts.factor_standing(&real.name, real.uid()),
        FactorStanding::Enrolled
    );
    assert_eq!(index_at(&real.rig, &session, Some("10.1.2.3")).status, 401);
}

#[test]
fn a_reset_ends_the_channels_of_a_session_made_with_a_code_within_a_few_seconds() {
    let real = real(&first_use_may_set_up(), |_| {});
    let (session, _) = enrolled(&real);
    let origin = origin(&real.rig);
    let mut socket = open_channel(&real.rig, "/portal/ws/entry/x", &session, &origin).unwrap();
    assert!(exchange(&mut socket, b"a").ends_with("entry:a"));
    assert_eq!(
        real.rig.accounts.reset_factor(&real.name, real.uid()),
        Ok(true)
    );
    let ended = (0..90).any(|_| {
        thread::sleep(Duration::from_millis(100));
        socket
            .send(tungstenite::Message::Binary(b"b".to_vec().into()))
            .is_err()
            || socket.read().is_err()
    });
    assert!(ended, "the channel outlived the code that made its session");
}

#[test]
fn a_session_made_without_a_code_is_not_ended_by_anyones_reset() {
    let real = real(
        &serde_json::json!({"rule": [{"allow": [["password"]]}]}),
        |_| {},
    );
    let session = session_cookie(&real.password_step("10.1.2.3"));
    assert_eq!(
        real.rig.accounts.reset_factor(&real.name, real.uid()),
        Ok(false)
    );
    assert_eq!(index_at(&real.rig, &session, Some("10.1.2.3")).status, 200);
}

// ---- guessing at a person is logged, and bounded by the day ----

#[test]
fn the_persons_limit_tripping_is_logged_at_warn_without_a_name() {
    let installed = capture_installed();
    let rig = two_step_rig(|settings| settings.max_factor_failures_per_person = 2);
    let pending = first_step(&rig, "10.77.1.1");
    for _ in 0..3 {
        code_at(&rig, "10.77.1.1", &pending, "000000");
    }
    thread::sleep(Duration::from_millis(100));
    if installed {
        let log = logged();
        let line = log
            .lines()
            .find(|line| line.contains("may be guessed at") && line.contains("10.77.1.1"))
            .expect("the limit is logged");
        assert!(
            line.contains("WARN") && line.contains("10.77.1.1"),
            "{line}"
        );
        assert!(line.contains("alice"), "{line}");
    }
}

#[test]
fn a_guesser_who_keeps_under_the_windows_limit_is_stopped_by_the_days() {
    let rig = two_step_rig(|settings| {
        settings.max_factor_failures_per_person = 100;
        settings.max_factor_failures_per_day = 3;
        settings.factor_day = Duration::from_millis(700);
        // The window is shorter than the day, so only the day's count still holds the failures.
        settings.failure_window = Duration::from_millis(150);
    });
    for _ in 0..3 {
        let pending = first_step(&rig, "10.1.2.3");
        assert_eq!(code_at(&rig, "10.1.2.3", &pending, "000000").status, 401);
        thread::sleep(Duration::from_millis(200));
    }
    let pending = first_step(&rig, "10.1.2.3");
    let before = calls(&rig);
    let right = code_at(&rig, "10.1.2.3", &pending, "123456");
    assert_eq!(right.status, 401);
    assert!(set_to(&right, COOKIE).is_none());
    assert_eq!(
        calls(&rig),
        before,
        "the right code is not checked once the day's count is full"
    );
    thread::sleep(Duration::from_millis(800));
    let pending = first_step(&rig, "10.1.2.3");
    assert_eq!(code_at(&rig, "10.1.2.3", &pending, "123456").status, 303);
}

// ---- a code belongs to an account's name as well as its user id ----

#[test]
fn a_code_is_the_accounts_by_its_name_and_not_by_a_user_id_alone() {
    let real = real(&first_use_may_set_up(), |_| {});
    let accounts = &real.rig.accounts;
    let begun = accounts
        .begin_factor(&real.name, real.uid(), false)
        .unwrap();
    accounts
        .confirm_factor(&real.name, real.uid(), &code_in(&begun.uri, 0))
        .unwrap();
    // The same user id made again for another account has none of this one's, and is the administrator's to set up.
    assert_eq!(
        accounts.factor_standing("someone-else", real.uid()),
        FactorStanding::Reset
    );
    assert_eq!(
        accounts.check_code(
            "someone-else",
            real.uid(),
            Method::Totp,
            &code_in(&begun.uri, 1),
            &[]
        ),
        CodeCheck::Refused
    );
    assert_eq!(
        accounts.check_code(
            "someone-else",
            real.uid(),
            Method::RecoveryCode,
            &begun.codes[0],
            &[]
        ),
        CodeCheck::Refused
    );
    assert!(matches!(
        accounts.check_code(
            &real.name,
            real.uid(),
            Method::Totp,
            &code_in(&begun.uri, 1),
            &[]
        ),
        CodeCheck::Valid { .. }
    ));
    // The listing names the record by both, and a reset by that id takes it away for an account that is gone.
    let listed = accounts.list_factors().unwrap();
    assert_eq!(listed[0].id, format!("{}:{}", real.uid(), real.name));
    assert_eq!(accounts.reset_by_id(&listed[0].id), Ok(true));
    assert!(accounts.reset_by_id("not-a-key").is_err());
    assert!(accounts.reset_by_id("12:../x").is_err());
    assert_eq!(
        accounts.factor_standing(&real.name, real.uid()),
        FactorStanding::Reset
    );
    assert!(accounts.list_factors().unwrap()[0].reset);
}

// ---- the log of a test run ----

fn capture_installed() -> bool {
    static INSTALLED: OnceLock<bool> = OnceLock::new();
    *INSTALLED.get_or_init(|| {
        tracing::subscriber::set_global_default(
            tracing_subscriber::fmt()
                .with_writer(Capture)
                .with_ansi(false)
                .without_time()
                .with_max_level(tracing::Level::TRACE)
                .finish(),
        )
        .is_ok()
    })
}

fn logged() -> String {
    String::from_utf8_lossy(&LOG.get_or_init(Mutex::default).lock().unwrap()).into_owned()
}

#[test]
fn a_limit_tripping_is_warned_of_once_a_window_and_the_rest_are_said_at_info() {
    let installed = capture_installed();
    let rig = two_step_rig(|settings| {
        settings.max_factor_failures_per_person = 2;
        settings.failure_window = Duration::from_millis(900);
        settings.max_failures_per_pair = 100;
    });
    let pending = first_step(&rig, "10.88.2.1");
    // Two wrong codes fill the window; every try after is refused, and a guesser makes them in numbers.
    for _ in 0..5 {
        code_at(&rig, "10.88.2.1", &pending, "000000");
    }
    thread::sleep(Duration::from_millis(1000));
    let again = first_step(&rig, "10.88.2.1");
    for _ in 0..4 {
        code_at(&rig, "10.88.2.1", &again, "000000");
    }
    thread::sleep(Duration::from_millis(100));
    if installed {
        let log = logged();
        let warns = log
            .lines()
            .filter(|line| line.contains("10.88.2.1") && line.contains("may be guessed at"))
            .count();
        let infos = log
            .lines()
            .filter(|line| {
                line.contains("10.88.2.1")
                    && line.contains("INFO")
                    && line.contains("second-step try for alice")
            })
            .count();
        assert_eq!(warns, 2, "one in each window");
        assert!(
            infos >= 3,
            "the others are at info with the account: {infos}"
        );
    }
}

#[test]
fn an_unlock_clears_a_persons_counts_in_a_running_portal_once_and_failures_count_again_after() {
    let real = real(&first_use_may_set_up(), |settings| {
        settings.max_factor_failures_per_person = 2;
        settings.max_failures_per_pair = 100;
        settings.max_failures_per_address = 1000;
    });
    let pending = pending_cookie_of(&real.password_step("10.1.2.3"));
    let link = link_in(&real.factor("10.1.2.3", &[&pending], "action=begin").body);
    real.factor(
        "10.1.2.3",
        &[&pending],
        &format!("action=confirm&code={}", code_in(&link, 0)),
    );
    // Someone else's guesses fill the person's count.
    let trip = |from: &str| {
        let pending = pending_cookie_of(&real.password_step(from));
        for _ in 0..2 {
            real.form(from, &[&pending], "method=totp&code=000000");
        }
    };
    trip("10.9.1.1");
    let pending = pending_cookie_of(&real.password_step("10.1.2.3"));
    let refused = real.form(
        "10.1.2.3",
        &[&pending],
        &format!("method=totp&code={}", code_in(&link, 1)),
    );
    assert_eq!(refused.status, 401, "the person is locked out by the count");
    // The administrator's unlock, as `canoa portal factor unlock NAME` writes it.
    thread::sleep(Duration::from_millis(5));
    real.rig.accounts.unlock(&real.name, real.uid()).unwrap();
    let pending = pending_cookie_of(&real.password_step("10.1.2.3"));
    let signed = real.form(
        "10.1.2.3",
        &[&pending],
        &format!("method=totp&code={}", code_in(&link, 1)),
    );
    assert_eq!(signed.status, 303, "{}", signed.body);
    // The unlock is acted on once: failures after it count, and fill the count again.
    trip("10.9.1.2");
    let pending = pending_cookie_of(&real.password_step("10.1.2.3"));
    let refused = real.form(
        "10.1.2.3",
        &[&pending],
        &format!("method=totp&code={}", code_in(&link, 2)),
    );
    assert_eq!(refused.status, 401);
}

#[test]
fn a_code_table_that_cannot_be_read_answers_503_and_keeps_the_session_and_its_channel() {
    let real = real(&first_use_may_set_up(), |_| {});
    let (session, _) = enrolled(&real);
    let origin = origin(&real.rig);
    let mut socket = open_channel(&real.rig, "/portal/ws/entry/x", &session, &origin).unwrap();
    assert!(exchange(&mut socket, b"a").ends_with("entry:a"));
    let table = real.dir.join("totp.toml");
    let kept = std::fs::read(&table).unwrap();
    std::fs::write(&table, b"[[accounts\nnot a table").unwrap();
    // A request cannot be served, and says so; the session is not ended by what could not be read.
    assert_eq!(index_at(&real.rig, &session, Some("10.1.2.3")).status, 503);
    // A channel looks again every few seconds, finds it cannot tell, and stays.
    thread::sleep(Duration::from_millis(4000));
    assert!(
        exchange(&mut socket, b"b").ends_with("entry:b"),
        "the channel was ended by a read error"
    );
    std::fs::write(&table, kept).unwrap();
    assert_eq!(index_at(&real.rig, &session, Some("10.1.2.3")).status, 200);
}

// ---- replacing a code needs the code ----

fn replace_with(enrol: &[&str]) -> serde_json::Value {
    let enrol: Vec<_> = enrol.iter().map(|each| serde_json::json!(each)).collect();
    serde_json::json!({"rule": [
        {"purposes": ["enrol_first"], "allow": [["password"]]},
        {"purposes": ["enrol"], "allow": [enrol]},
        {"allow": [["password", "totp"], ["password", "recovery_code"]]},
    ]})
}

fn show_password(real: &Real, session: &str) -> String {
    let shown = real.factor(
        "10.1.2.3",
        &[session],
        &format!(
            "action=step&method=password&user={}&password={PASSWORD}",
            real.name
        ),
    );
    assert_eq!(shown.status, 200, "{}", shown.body);
    pending_cookie_of(&shown)
}

#[test]
fn a_person_with_a_code_cannot_replace_it_by_showing_a_password_alone() {
    let real = real(&replace_with(&["password"]), |_| {});
    let (session, _) = enrolled(&real);
    let enrol = show_password(&real, &session);
    let begun = real.factor("10.1.2.3", &[&session, &enrol], "action=begin");
    assert_ne!(begun.status, 200, "{}", begun.body);
    assert!(!begun.body.contains("otpauth://"));
    // The code in use stands and nothing waits to replace it.
    assert_eq!(
        real.rig.accounts.factor_standing(&real.name, real.uid()),
        FactorStanding::Enrolled
    );
    assert!(
        real.rig
            .accounts
            .list_factors()
            .unwrap()
            .iter()
            .all(|entry| !entry.waiting)
    );
}

#[test]
fn a_person_with_a_code_replaces_it_after_showing_the_code_again() {
    let real = real(&replace_with(&["password", "totp"]), |_| {});
    let (session, link) = enrolled(&real);
    let enrol = show_password(&real, &session);
    let shown = real.factor(
        "10.1.2.3",
        &[&session, &enrol],
        &format!("action=step&method=totp&code={}", code_in(&link, 2)),
    );
    assert_eq!(shown.status, 200, "{}", shown.body);
    let begun = real.factor("10.1.2.3", &[&session, &enrol], "action=begin");
    assert_eq!(begun.status, 200, "{}", begun.body);
    let fresh = link_in(&begun.body);
    assert_ne!(fresh, link);
    let done = real.factor(
        "10.1.2.3",
        &[&session, &enrol],
        &format!("action=confirm&code={}", code_in(&fresh, 0)),
    );
    assert_eq!(done.status, 200, "{}", done.body);
}
