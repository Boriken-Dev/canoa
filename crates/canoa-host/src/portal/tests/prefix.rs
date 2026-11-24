//! The portal mounted under a path prefix (`base_path = "/canoa/"`): every route, redirect, form action, cookie and the site's
//! base carry the prefix, a request outside it is refused, and the portal at `/` answers as it always has.

use super::mount::{self, normalise};
use super::sign_in_policy::flow_rig_apart;
use super::*;

const BASE: &str = "/canoa/";

fn prefixed() -> Rig {
    rig(|settings| settings.base_path = BASE.to_owned(), 0)
}

fn session(rig: &Rig) -> String {
    let to = To::Unix(rig.sock.clone());
    let answer = request(
        &to,
        "POST",
        "/canoa/portal/login",
        &[
            ("Origin", to.origin()),
            (
                "Content-Type",
                "application/x-www-form-urlencoded".to_owned(),
            ),
            ("X-Forwarded-For", "198.51.100.7".to_owned()),
        ],
        "user=canoatesta&password=pw-a",
    );
    assert_eq!(answer.status, 303, "{}", answer.body);
    answer.cookie().expect("a cookie")
}

/// Every path a page holds in an attribute starts with the mount; the attributes are the ones the mount rewrites.
fn assert_mounted(page: &str, what: &str) {
    for head in [
        "href=\"",
        "action=\"",
        "src=\"",
        "data-options-url=\"",
        "<option value=\"",
    ] {
        for (at, _) in page.match_indices(head) {
            let value = &page[at + head.len()..];
            if value.starts_with('/') {
                assert!(
                    value.starts_with(BASE),
                    "{what}: {head}{} is not under {BASE}",
                    &value[..value.len().min(40)]
                );
            }
        }
    }
}

#[test]
fn a_request_outside_the_prefix_is_a_404_and_its_bare_address_goes_to_it() {
    let rig = prefixed();
    for path in [
        "/",
        "/portal/login",
        "/canoa2/",
        "/canoa.json",
        "/other/canoa/",
    ] {
        let answer = call(rig.addr, "GET", path, &[], "");
        assert_eq!(answer.status, 404, "{path}");
    }
    assert_eq!(call(rig.addr, "POST", "/portal/login", &[], "").status, 404);
    let bare = call(rig.addr, "GET", "/canoa", &[], "");
    assert_eq!((bare.status, bare.header("location")), (303, Some(BASE)));
    let bare = call(rig.addr, "GET", "/canoa?x=1", &[], "");
    assert_eq!((bare.status, bare.header("location")), (303, Some(BASE)));
}

#[test]
fn the_sign_in_page_links_forms_and_assets_under_the_prefix() {
    let rig = prefixed();
    let page = call(rig.addr, "GET", "/canoa/portal/login", &[], "");
    assert_eq!(page.status, 200);
    assert_mounted(&page.body, "login");
    for expected in [
        "href=\"/canoa/portal/login.css\"",
        "src=\"/canoa/portal/login.js\"",
        "action=\"/canoa/portal/login\"",
    ] {
        assert!(page.body.contains(expected), "{expected}");
    }
    assert!(!page.body.contains("=\"/portal/"), "{}", page.body);
    for asset in ["login.css", "login.js", "icon.svg"] {
        let answer = call(rig.addr, "GET", &format!("/canoa/portal/{asset}"), &[], "");
        assert_eq!(answer.status, 200, "{asset}");
        assert_eq!(
            call(rig.addr, "GET", &format!("/portal/{asset}"), &[], "").status,
            404
        );
    }
    let methods = call(rig.addr, "GET", "/canoa/portal/methods", &[], "");
    assert_eq!(methods.status, 200);
    // Without the sign-in, a page of the app goes to the sign-in under the prefix.
    let home = call(rig.addr, "GET", "/canoa/", &[], "");
    assert_eq!(
        (home.status, home.header("location")),
        (303, Some("/canoa/portal/login"))
    );
}

#[test]
fn the_page_with_the_chooser_carries_the_prefix_in_its_tabs_and_list() {
    let rig = prefixed();
    let proxied = To::Unix(rig.sock.clone());
    let page = request(
        &proxied,
        "GET",
        "/canoa/portal/login?method=certificate",
        &[("X-Forwarded-For", "10.1.2.3".to_owned())],
        "",
    );
    assert_eq!(page.status, 200);
    assert_mounted(&page.body, "chooser");
    assert!(
        page.body.contains("href=\"/canoa/portal/login?method="),
        "{}",
        page.body
    );
    assert!(
        page.body
            .contains("<option value=\"/canoa/portal/login?method=")
    );
}

#[test]
fn a_sign_in_sets_a_secure_cookie_for_the_prefix_only_and_the_site_is_based_there() {
    let rig = prefixed();
    let to = To::Unix(rig.sock.clone());
    let answer = request(
        &to,
        "POST",
        "/canoa/portal/login",
        &[
            ("Origin", to.origin()),
            (
                "Content-Type",
                "application/x-www-form-urlencoded".to_owned(),
            ),
            ("X-Forwarded-For", "198.51.100.7".to_owned()),
        ],
        "user=canoatesta&password=pw-a",
    );
    assert_eq!(
        (answer.status, answer.header("location")),
        (303, Some(BASE))
    );
    let line = answer.header("set-cookie").unwrap().to_owned();
    for attribute in [
        "__Secure-canoa.canoa=",
        "Path=/canoa/;",
        "HttpOnly",
        "Secure",
        "SameSite=Strict",
    ] {
        assert!(line.contains(attribute), "{attribute} in {line}");
    }
    assert!(
        !line.contains("__Host-") && !line.contains("Path=/;"),
        "{line}"
    );
    let cookie = answer.cookie().unwrap();
    // The session is the prefix's alone: the root's cookie name is not one.
    let root_named = format!("{COOKIE}={}", cookie.split_once('=').unwrap().1);
    let refused = call(rig.addr, "GET", "/canoa/", &[("Cookie", root_named)], "");
    assert_eq!(refused.status, 303);

    std::fs::write(
        rig.root.join("index.html"),
        "<!doctype html><head><base href=\"/\"></head>the app",
    )
    .unwrap();
    std::fs::write(rig.root.join("main.dart.js"), "code").unwrap();
    let home = call(
        rig.addr,
        "GET",
        "/canoa/",
        &[("Cookie", cookie.clone())],
        "",
    );
    assert_eq!(home.status, 200);
    assert!(
        home.body.contains("<base href=\"/canoa/\">"),
        "{}",
        home.body
    );
    // An address of the app inside the prefix is the app's page too, and a file is the file.
    let route = call(
        rig.addr,
        "GET",
        "/canoa/chat/1",
        &[("Cookie", cookie.clone())],
        "",
    );
    assert!(route.status == 200 && route.body.contains("<base href=\"/canoa/\">"));
    let file = call(
        rig.addr,
        "GET",
        "/canoa/main.dart.js",
        &[("Cookie", cookie.clone())],
        "",
    );
    assert_eq!((file.status, file.body.as_str()), (200, "code"));
    assert_eq!(
        call(rig.addr, "GET", "/main.dart.js", &[("Cookie", cookie)], "").status,
        404
    );
}

#[test]
fn the_index_the_session_the_pair_and_the_channels_are_under_the_prefix() {
    let rig = prefixed();
    let cookie = session(&rig);
    let index = call(
        rig.addr,
        "GET",
        "/canoa/canoa.json",
        &[("Cookie", cookie.clone())],
        "",
    );
    assert_eq!(index.status, 200, "{}", index.body);
    let json: serde_json::Value = serde_json::from_str(&index.body).unwrap();
    let host = rig.addr.to_string();
    let urls: Vec<&str> = json["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["url"].as_str().unwrap())
        .collect();
    assert_eq!(
        urls,
        [
            format!("ws://{host}/canoa/portal/ws/entry/x"),
            format!("ws://{host}/canoa/portal/ws/control")
        ]
    );
    let state = call(
        rig.addr,
        "GET",
        "/canoa/portal/session",
        &[("Cookie", cookie.clone())],
        "",
    );
    assert_eq!(state.status, 200);
    assert_eq!(
        call(
            rig.addr,
            "GET",
            "/portal/session",
            &[("Cookie", cookie.clone())],
            ""
        )
        .status,
        404
    );
    let origin = origin(&rig);
    let pair = call(
        rig.addr,
        "POST",
        "/canoa/portal/pair",
        &[
            ("Cookie", cookie.clone()),
            ("Origin", origin.clone()),
            ("Content-Type", "application/json".to_owned()),
        ],
        r#"{"peer":"sha256:browserkey"}"#,
    );
    assert_eq!(pair.status, 200, "{}", pair.body);
    let mut control = open_channel(&rig, "/canoa/portal/ws/control", &cookie, &origin).unwrap();
    assert_eq!(exchange(&mut control, b"hi"), "canoatesta/control:hi");
    let mut entry = open_channel(&rig, "/canoa/portal/ws/entry/x", &cookie, &origin).unwrap();
    assert_eq!(exchange(&mut entry, b"hi"), "canoatesta/entry:hi");
    assert_eq!(
        open_channel(&rig, "/portal/ws/control", &cookie, &origin).err(),
        Some(404)
    );
}

#[test]
fn every_page_of_a_signed_in_person_is_under_the_prefix() {
    let rig = prefixed();
    let cookie = session(&rig);
    let mut seen = 0;
    for path in [
        "/canoa/portal/certificate",
        "/canoa/portal/factor",
        "/canoa/portal/passkeys",
        "/canoa/portal/account",
    ] {
        let answer = call(rig.addr, "GET", path, &[("Cookie", cookie.clone())], "");
        assert!(
            matches!(answer.status, 200 | 404 | 503),
            "{path}: {}",
            answer.status
        );
        if answer.status == 200
            && answer
                .header("content-type")
                .is_some_and(|kind| kind.starts_with("text/html"))
        {
            assert_mounted(&answer.body, path);
            seen += 1;
        }
        // The same page without the prefix is not one of this portal's.
        let bare = path.replacen("/canoa", "", 1);
        assert_eq!(
            call(rig.addr, "GET", &bare, &[("Cookie", cookie.clone())], "").status,
            404,
            "{bare}"
        );
    }
    assert!(seen >= 1, "at least the certificate page answers");
    let account = call(
        rig.addr,
        "GET",
        "/canoa/portal/account",
        &[("Cookie", cookie)],
        "",
    );
    assert_eq!(account.status, 200);
    assert!(!account.body.contains("\"/portal/"), "{}", account.body);
}

#[test]
fn signing_out_clears_the_cookies_at_the_prefix_and_goes_to_its_sign_in() {
    let rig = prefixed();
    let cookie = session(&rig);
    let out = call(
        rig.addr,
        "POST",
        "/canoa/portal/logout",
        &[("Cookie", cookie.clone()), ("Origin", origin(&rig))],
        "",
    );
    assert_eq!(
        (out.status, out.header("location")),
        (303, Some("/canoa/portal/login"))
    );
    let lines: Vec<&str> = out
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("set-cookie"))
        .map(|(_, value)| value.as_str())
        .collect();
    assert_eq!(lines.len(), 2, "{lines:?}");
    for line in lines {
        assert!(
            line.starts_with("__Secure-canoa")
                && line.contains("; Path=/canoa/;")
                && line.contains("Max-Age=0"),
            "{line}"
        );
    }
    let after = call(
        rig.addr,
        "GET",
        "/canoa/portal/session",
        &[("Cookie", cookie)],
        "",
    );
    assert_eq!(after.status, 401);
}

#[test]
fn a_step_of_a_sign_in_keeps_its_pending_cookie_and_page_under_the_prefix() {
    let rig = flow_rig_apart(
        &serde_json::json!({"rule": [{"allow": [["password", "totp"]]}]}),
        |settings| settings.base_path = BASE.to_owned(),
    );
    let to = To::Unix(rig.sock.clone());
    let form = |cookie: Option<&str>, body: &str| {
        let mut extra = vec![
            ("Origin", to.origin()),
            (
                "Content-Type",
                "application/x-www-form-urlencoded".to_owned(),
            ),
            ("X-Forwarded-For", "10.1.2.3".to_owned()),
        ];
        if let Some(cookie) = cookie {
            extra.push(("Cookie", cookie.to_owned()));
        }
        request(&to, "POST", "/canoa/portal/login", &extra, body)
    };
    let first = form(None, "method=password&user=alice&password=pw");
    assert_eq!(first.status, 303, "{}", first.body);
    assert_eq!(first.header("location"), Some("/canoa/portal/login"));
    let line = first.header("set-cookie").unwrap().to_owned();
    assert!(
        line.starts_with("__Secure-canoa-pending.canoa=") && line.contains("Path=/canoa/;"),
        "{line}"
    );
    let pending = first.cookie().unwrap();
    let page = request(
        &to,
        "GET",
        "/canoa/portal/login",
        &[
            ("X-Forwarded-For", "10.1.2.3".to_owned()),
            ("Cookie", pending.clone()),
        ],
        "",
    );
    assert_eq!(page.status, 200);
    assert_mounted(&page.body, "second step");
    assert!(
        page.body.contains("action=\"/canoa/portal/login\""),
        "{}",
        page.body
    );
    let methods = request(
        &to,
        "GET",
        "/canoa/portal/methods",
        &[
            ("X-Forwarded-For", "10.1.2.3".to_owned()),
            ("Cookie", pending),
        ],
        "",
    );
    assert!(methods.body.contains("\"next\""), "{}", methods.body);
}

#[test]
fn a_certificate_sign_in_under_the_prefix_opens_a_session() {
    let rig = prefixed();
    let to = To::Unix(rig.sock.clone());
    let answer = request(
        &to,
        "POST",
        "/canoa/portal/login",
        &[
            ("Origin", to.origin()),
            (
                "Content-Type",
                "application/x-www-form-urlencoded".to_owned(),
            ),
            ("X-Forwarded-For", "198.51.100.7".to_owned()),
            ("X-Client-Cert", "MIIB-test-cert".to_owned()),
        ],
        "method=certificate",
    );
    assert_eq!(
        (answer.status, answer.header("location")),
        (303, Some(BASE))
    );
    assert!(
        answer
            .header("set-cookie")
            .unwrap()
            .contains("Path=/canoa/;")
    );
}

#[test]
fn the_portal_at_the_root_answers_as_it_does_below_a_prefix_without_it() {
    let root = rig(|_| {}, 0);
    let mounted = prefixed();
    for path in ["/portal/login", "/portal/login?method=certificate"] {
        let at_root = call(root.addr, "GET", path, &[], "");
        let below = call(mounted.addr, "GET", &format!("/canoa{path}"), &[], "");
        assert_eq!((at_root.status, below.status), (200, 200));
        assert_eq!(
            below.body.replace("/canoa/", "/"),
            at_root.body,
            "{path}: the same page, but for the prefix"
        );
    }
    let answer = login(&root, "canoatesta", "pw-a", "198.51.100.7");
    assert_eq!(answer.header("location"), Some("/"));
    assert!(
        answer
            .header("set-cookie")
            .unwrap()
            .starts_with("__Host-canoa=")
    );
    assert!(answer.header("set-cookie").unwrap().contains("; Path=/;"));
}

#[test]
fn a_prefix_is_normalised_and_one_that_is_not_a_path_is_refused() {
    assert_eq!(normalise("canoa").unwrap(), "/canoa/");
    assert_eq!(normalise("/canoa").unwrap(), "/canoa/");
    assert_eq!(normalise("/a/b/").unwrap(), "/a/b/");
    assert_eq!(normalise("").unwrap(), "/");
    assert_eq!(normalise("/").unwrap(), "/");
    for bad in ["/a//b/", "/../", "/a/./", "/a b/", "/a?x/", "/é/", "/a.b/"] {
        assert!(normalise(bad).is_err(), "{bad}");
    }
    let settings = Settings::from_toml("base_path = \"canoa\"").unwrap();
    assert_eq!(settings.base_path, "/canoa/");
    assert_eq!(Settings::from_toml("").unwrap().base_path, "/");
    assert!(Settings::from_toml("base_path = \"/a b/\"").is_err());
    let unchecked = Settings {
        base_path: "canoa".to_owned(),
        ..Settings::default()
    };
    assert!(unchecked.check().is_err());
    assert_eq!(
        mount::scoped_name("__Host-canoa-pane", "a_b"),
        "__Secure-canoa-pane.a_b"
    );
}
