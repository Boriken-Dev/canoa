//! The Office add-in through the portal: its pages under their own policy, a sign-in that ends in the Office dialog, and the
//! session a task pane's frame claims with the string the dialog sends. Every other route keeps its headers.

use super::*;

/// A portal with the add-in on, and the site's icon where the add-in's icons are read from.
fn office_rig(change: impl FnOnce(&mut Settings)) -> Rig {
    let rig = rig(
        |settings| {
            settings.office = true;
            change(settings);
        },
        0,
    );
    std::fs::create_dir_all(rig.root.join("icons")).unwrap();
    std::fs::write(rig.root.join("icons").join("Icon-192.png"), "png").unwrap();
    std::fs::write(rig.root.join("main.dart.js"), "code").unwrap();
    rig
}

fn lines<'a>(answer: &'a Answer, name: &str) -> Vec<&'a str> {
    answer
        .headers
        .iter()
        .filter(|(each, _)| each.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
        .collect()
}

/// The `NAME=VALUE` of the cookie `name` an answer sets, when it sets one that holds something.
fn set(answer: &Answer, name: &str) -> Option<String> {
    lines(answer, "set-cookie")
        .into_iter()
        .filter_map(|line| line.split(';').next())
        .find(|pair| pair.starts_with(&format!("{name}=")) && !pair.ends_with('='))
        .map(str::to_owned)
}

/// The whole `Set-Cookie` line of the cookie `name`.
fn set_line<'a>(answer: &'a Answer, name: &str) -> &'a str {
    lines(answer, "set-cookie")
        .into_iter()
        .find(|line| line.starts_with(&format!("{name}=")))
        .unwrap_or_else(|| panic!("no {name} cookie in {:?}", answer.headers))
}

fn from_dialog(
    rig: &Rig,
    extra: &[(&str, String)],
    method: &str,
    path: &str,
    body: &str,
) -> Answer {
    let to = To::Unix(rig.sock.clone());
    let mut all = vec![
        ("Origin", to.origin()),
        ("Sec-Fetch-Site", "same-origin".to_owned()),
        ("X-Forwarded-For", "198.51.100.7".to_owned()),
        (
            "Content-Type",
            "application/x-www-form-urlencoded".to_owned(),
        ),
    ];
    // What the caller names replaces what a browser would send of its own.
    all.retain(|(name, _)| {
        !extra
            .iter()
            .any(|(each, _)| each.eq_ignore_ascii_case(name))
    });
    all.extend(extra.iter().map(|(name, value)| (*name, value.clone())));
    request(&to, method, path, &all, body)
}

/// The marker the dialog's first request is given.
fn marker(rig: &Rig) -> String {
    let start = from_dialog(rig, &[], "GET", OFFICE_SIGN_IN, "");
    assert_eq!(start.status, 303, "{}", start.body);
    set(&start, OFFICE_COOKIE).expect("a marker")
}

/// A sign-in in the dialog: the string its page ends with.
fn dialog_sign_in(rig: &Rig, user: &str, password: &str) -> String {
    let answer = from_dialog(
        rig,
        &[("Cookie", marker(rig))],
        "POST",
        LOGIN,
        &format!("method=password&user={user}&password={password}"),
    );
    assert_eq!(answer.status, 303, "{}", answer.body);
    let location = answer.header("location").unwrap();
    location
        .strip_prefix("/office/dialog.html#done=")
        .unwrap_or_else(|| panic!("{location}"))
        .to_owned()
}

/// What the task pane's frame sends: its own page's origin and no cookie yet.
fn claim(rig: &Rig, string: &str, extra: &[(&str, String)]) -> Answer {
    let mut all = vec![("Content-Type", "application/json".to_owned())];
    all.extend(extra.iter().map(|(name, value)| (*name, value.clone())));
    from_dialog(
        rig,
        &all,
        "POST",
        OFFICE_SESSION,
        &serde_json::json!({ "handoff": string }).to_string(),
    )
}

/// The cookie header of a frame that claimed.
fn pane(rig: &Rig, user: &str, password: &str) -> String {
    let answer = claim(rig, &dialog_sign_in(rig, user, password), &[]);
    assert_eq!(answer.status, 200, "{}", answer.body);
    set(&answer, PANE_COOKIE).expect("the pane's cookie")
}

#[test]
fn with_the_add_in_off_nothing_under_office_is_answered_by_it() {
    let rig = rig(|_| {}, 0);
    for path in [
        "/office/taskpane.html",
        "/office/dialog.html",
        "/office/taskpane.js",
        "/office/app/",
        "/office/icon-32.png",
        OFFICE_SIGN_IN,
        OFFICE_SESSION,
    ] {
        let answer = call(rig.addr, "GET", path, &[], "");
        assert_eq!(
            (answer.status, answer.header("location")),
            (303, Some(LOGIN)),
            "{path}: the site's, behind its sign-in"
        );
    }
    // The marker means nothing: a sign-in ends at the site, with the site's cookie.
    let answer = from_dialog(
        &rig,
        &[("Cookie", format!("{OFFICE_COOKIE}=1"))],
        "POST",
        LOGIN,
        "method=password&user=canoatesta&password=pw-a",
    );
    assert_eq!((answer.status, answer.header("location")), (303, Some("/")));
    assert!(set(&answer, COOKIE).is_some());
    assert!(
        call(rig.addr, "POST", OFFICE_SESSION, &[], "").status != 200,
        "no claim is taken"
    );
}

#[test]
fn the_pages_are_given_to_a_browser_that_has_not_signed_in_each_with_the_policy_it_needs() {
    let rig = office_rig(|_| {});
    let pane = call(rig.addr, "GET", "/office/taskpane.html", &[], "");
    assert_eq!(pane.status, 200);
    assert!(pane.body.contains("office.js"));
    let policy = pane.header("content-security-policy").unwrap();
    assert!(policy.contains("https://officeapis.public.onecdn.static.microsoft"));
    assert!(policy.contains("frame-src 'self'"));
    assert!(policy.contains("frame-ancestors https://outlook.office.com"));
    assert!(!policy.contains("frame-ancestors 'none'"));
    assert_eq!(pane.header("x-frame-options"), None, "Office frames it");
    assert_eq!(pane.header("cache-control"), Some("no-cache"));
    assert_eq!(pane.header("x-content-type-options"), Some("nosniff"));

    let dialog = call(rig.addr, "GET", "/office/dialog.html", &[], "");
    assert_eq!(dialog.status, 200);
    let policy = dialog.header("content-security-policy").unwrap();
    assert!(policy.contains("https://officeapis.public.onecdn.static.microsoft"));
    assert!(policy.contains("frame-ancestors 'none'"));
    assert_eq!(dialog.header("x-frame-options"), Some("DENY"));
    assert_eq!(
        dialog.header("cross-origin-opener-policy"),
        None,
        "the dialog keeps its link to Office"
    );

    for (path, kind) in [
        ("/office/taskpane.js", "text/javascript"),
        ("/office/relation.js", "text/javascript"),
        ("/office/dialog.js", "text/javascript"),
    ] {
        let script = call(rig.addr, "GET", path, &[], "");
        assert_eq!(script.status, 200, "{path}");
        assert!(script.header("content-type").unwrap().starts_with(kind));
        assert_eq!(script.header("x-frame-options"), Some("DENY"), "{path}");
        assert_eq!(
            script.header("cross-origin-opener-policy"),
            Some("same-origin"),
            "{path}"
        );
        assert!(
            script
                .header("content-security-policy")
                .unwrap()
                .contains("frame-ancestors 'none'"),
            "{path}"
        );
    }

    let icon = call(rig.addr, "GET", "/office/icon-32.png", &[], "");
    assert_eq!(icon.status, 200);
    assert_eq!(icon.body, "png");
    assert_eq!(
        icon.header("cross-origin-resource-policy"),
        Some("cross-origin")
    );
}

#[test]
fn every_other_route_keeps_its_headers_and_its_sign_in() {
    let rig = office_rig(|_| {});
    let login = call(rig.addr, "GET", LOGIN, &[], "");
    assert_eq!(login.header("x-frame-options"), Some("DENY"));
    assert_eq!(
        login.header("cross-origin-opener-policy"),
        Some("same-origin")
    );
    assert!(
        login
            .header("content-security-policy")
            .unwrap()
            .contains("frame-ancestors 'none'")
    );
    // Under /office/ too: a path the add-in does not name is the site's, and is behind the sign-in.
    for path in [
        "/office/other",
        "/office/app/deeper",
        "/office/taskpane.html/x",
        "/office/ICON-32.png",
    ] {
        let answer = call(rig.addr, "GET", path, &[], "");
        assert_eq!(
            (answer.status, answer.header("location")),
            (303, Some(LOGIN)),
            "{path}"
        );
    }
    let cookie = signed_in(&rig, "canoatesta");
    for path in ["/", "/main.dart.js", "/office/other"] {
        let answer = call(rig.addr, "GET", path, &[("Cookie", cookie.clone())], "");
        assert_eq!(answer.status, 200, "{path}");
        assert_eq!(answer.header("x-frame-options"), Some("DENY"), "{path}");
        assert!(
            answer
                .header("content-security-policy")
                .unwrap()
                .contains("frame-ancestors 'none'"),
            "{path}"
        );
    }
    assert_eq!(call(rig.addr, "GET", web::HOST_INDEX, &[], "").status, 401);
}

#[test]
fn the_app_page_is_for_a_signed_in_frame_alone_and_may_be_framed_by_office() {
    let rig = office_rig(|settings| {
        settings.office_frames = vec!["https://*.officeapps.live.com".to_owned()];
    });
    assert_eq!(call(rig.addr, "GET", "/office/app/", &[], "").status, 401);
    assert_eq!(
        call(
            rig.addr,
            "GET",
            "/office/app/",
            &[("Cookie", format!("{PANE_COOKIE}=nope"))],
            ""
        )
        .status,
        401
    );
    let cookie = pane(&rig, "canoatesta", "pw-a");
    let app = call(
        rig.addr,
        "GET",
        "/office/app/?mode=chat",
        &[("Cookie", cookie.clone())],
        "",
    );
    assert_eq!(app.status, 200);
    assert!(app.body.contains("the app"));
    let policy = app.header("content-security-policy").unwrap();
    assert!(policy.contains("frame-ancestors 'self' https://outlook.office.com"));
    assert!(
        policy.contains("https://*.officeapps.live.com"),
        "what the portal's settings add"
    );
    assert!(
        policy.contains("script-src 'self' 'wasm-unsafe-eval'"),
        "the app's own policy otherwise"
    );
    assert!(
        !policy.contains("officeapis"),
        "Office.js is the pane's, not the app's"
    );
    assert_eq!(app.header("x-frame-options"), None);
    // The rest of the site, which the frame's page loads, is given to the pane's cookie and keeps its headers.
    let script = call(rig.addr, "GET", "/main.dart.js", &[("Cookie", cookie)], "");
    assert_eq!(script.status, 200);
    assert_eq!(script.header("x-frame-options"), Some("DENY"));
}

#[test]
fn a_sign_in_ends_in_the_dialog_with_a_string_and_opens_no_session_there() {
    let rig = office_rig(|_| {});
    // Only the site's own page starts it: another site cannot make a person's next sign-in end in a dialog.
    for site in ["cross-site", "same-site", "none"] {
        let start = from_dialog(
            &rig,
            &[("Sec-Fetch-Site", site.to_owned())],
            "GET",
            OFFICE_SIGN_IN,
            "",
        );
        assert_eq!(start.status, 403, "{site}");
        assert_eq!(set(&start, OFFICE_COOKIE), None, "{site}");
    }
    let start = from_dialog(&rig, &[], "GET", OFFICE_SIGN_IN, "");
    assert_eq!((start.status, start.header("location")), (303, Some(LOGIN)));
    let line = set_line(&start, OFFICE_COOKIE);
    for attribute in [
        "HttpOnly",
        "Secure",
        "SameSite=Strict",
        "Path=/",
        "Max-Age=",
    ] {
        assert!(line.contains(attribute), "{attribute} in {line}");
    }
    assert!(!line.contains("Domain"));
    let marker = set(&start, OFFICE_COOKIE).unwrap();

    // The sign-in page opened in the dialog does not cut the dialog's link to Office; the same page for anyone else does.
    let to = To::Unix(rig.sock.clone());
    let with = |cookie: Option<&str>| {
        let mut extra = vec![("X-Forwarded-For", "198.51.100.7".to_owned())];
        if let Some(cookie) = cookie {
            extra.push(("Cookie", cookie.to_owned()));
        }
        request(&to, "GET", LOGIN, &extra, "")
    };
    assert_eq!(
        with(Some(&marker)).header("cross-origin-opener-policy"),
        None
    );
    assert_eq!(with(Some(&marker)).header("x-frame-options"), Some("DENY"));
    assert_eq!(
        with(None).header("cross-origin-opener-policy"),
        Some("same-origin")
    );

    // A wrong password is the sign-in page again, as ever, and leaves nothing waiting.
    let wrong = from_dialog(
        &rig,
        &[("Cookie", marker.clone())],
        "POST",
        LOGIN,
        "method=password&user=canoatesta&password=wrong",
    );
    assert_ne!(wrong.status, 303);
    assert!(
        !wrong
            .headers
            .iter()
            .any(|(_, value)| value.contains("#done="))
    );

    let right = from_dialog(
        &rig,
        &[("Cookie", marker)],
        "POST",
        LOGIN,
        "method=password&user=canoatesta&password=pw-a",
    );
    assert_eq!(right.status, 303);
    let location = right.header("location").unwrap();
    let string = location
        .strip_prefix("/office/dialog.html#done=")
        .expect(location);
    assert!(
        string.len() >= 43
            && string
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    );
    assert_eq!(
        set(&right, COOKIE),
        None,
        "no site cookie: the dialog is not where the chat runs"
    );
    assert_eq!(set(&right, PANE_COOKIE), None);
    assert!(
        lines(&right, "set-cookie")
            .iter()
            .any(|line| line.starts_with(&format!("{OFFICE_COOKIE}=;"))
                && line.contains("Max-Age=0")),
        "the marker is spent"
    );
    assert_eq!(right.header("cross-origin-opener-policy"), None);
    assert!(!rig.inner.sessions.alive(1), "no session was made");
}

#[test]
fn the_string_is_claimed_once_by_a_frame_of_the_sites_own_page_and_sets_a_cookie_a_third_party_frame_carries()
 {
    let rig = office_rig(|_| {});
    let string = dialog_sign_in(&rig, "canoatesta", "pw-a");
    // Not from another site, not from a page that names none, not unless it is JSON with the string in it.
    for site in ["cross-site", "same-site"] {
        let answer = claim(&rig, &string, &[("Sec-Fetch-Site", site.to_owned())]);
        assert_eq!(answer.status, 403, "{site}");
    }
    assert_eq!(
        claim(
            &rig,
            &string,
            &[("Origin", "http://evil.example".to_owned())]
        )
        .status,
        403
    );
    assert_eq!(
        claim(&rig, &string, &[("Origin", "null".to_owned())]).status,
        403
    );
    let plain = from_dialog(&rig, &[], "POST", OFFICE_SESSION, "handoff=x");
    assert_eq!(plain.status, 400);
    let nothing = from_dialog(&rig, &[], "POST", OFFICE_SESSION, "{}");
    assert_eq!(nothing.status, 400);
    assert_eq!(claim(&rig, "not-a-string-the-portal-made", &[]).status, 401);

    // None of those spent it.
    let answer = claim(&rig, &string, &[]);
    assert_eq!(answer.status, 200, "{}", answer.body);
    assert!(answer.body.contains("canoatesta"));
    assert_eq!(answer.header("cache-control"), Some("no-store"));
    let line = set_line(&answer, PANE_COOKIE);
    for attribute in [
        "HttpOnly",
        "Secure",
        "SameSite=None",
        "Partitioned",
        "Path=/",
        "Max-Age=",
    ] {
        assert!(line.contains(attribute), "{attribute} in {line}");
    }
    assert!(!line.contains("Domain"));
    assert!(line.starts_with("__Host-canoa-pane="));
    let token = set(&answer, PANE_COOKIE)
        .unwrap()
        .split_once('=')
        .unwrap()
        .1
        .to_owned();
    assert!(!answer.body.contains(&token) && !string.contains(&token) && token != string);
    assert_eq!(claim(&rig, &string, &[]).status, 401, "good for one claim");
}

#[test]
fn a_claimed_session_is_the_signed_in_person_to_every_route_a_session_opens() {
    let rig = office_rig(|_| {});
    let cookie = pane(&rig, "canoatesta", "pw-a");
    let state = call(
        rig.addr,
        "GET",
        OFFICE_SESSION,
        &[("Cookie", cookie.clone())],
        "",
    );
    assert_eq!(state.status, 200);
    assert!(
        state
            .header("content-type")
            .unwrap()
            .starts_with("application/json")
    );
    assert!(state.body.contains("canoatesta") && state.body.contains("true"));
    let session = call(
        rig.addr,
        "GET",
        "/portal/session",
        &[("Cookie", cookie.clone())],
        "",
    );
    assert_eq!(session.status, 200);
    let index = call(
        rig.addr,
        "GET",
        web::HOST_INDEX,
        &[("Cookie", cookie.clone())],
        "",
    );
    assert_eq!(index.status, 200, "{}", index.body);
    assert!(index.body.contains("canoatesta"));
    assert_eq!(rig.hosts.starts.load(Ordering::SeqCst), 1);

    // Without the cookie the same questions are answered as for anyone not signed in, in JSON the pane reads.
    let none = call(rig.addr, "GET", OFFICE_SESSION, &[], "");
    assert_eq!(none.status, 401);
    assert!(
        none.header("content-type")
            .unwrap()
            .starts_with("application/json")
    );
    assert!(none.body.contains("false"));

    // A pane that signs out ends the session and clears both cookies.
    let out = call(
        rig.addr,
        "POST",
        "/portal/logout",
        &[("Cookie", cookie.clone()), ("Origin", origin(&rig))],
        "",
    );
    assert_eq!(out.status, 303);
    assert!(lines(&out, "set-cookie").iter().any(|line| {
        line.starts_with(&format!("{PANE_COOKIE}=;"))
            && line.contains("Partitioned")
            && line.contains("Max-Age=0")
    }));
    assert_eq!(
        call(rig.addr, "GET", OFFICE_SESSION, &[("Cookie", cookie)], "").status,
        401
    );
}

#[test]
fn a_claimed_session_is_asked_about_again_like_any_other() {
    let rig = office_rig(|_| {});
    let cookie = pane(&rig, "canoatesta", "pw-a");
    assert_eq!(
        call(
            rig.addr,
            "GET",
            OFFICE_SESSION,
            &[("Cookie", cookie.clone())],
            ""
        )
        .status,
        200
    );
    rig.accounts
        .gone
        .lock()
        .unwrap()
        .insert("canoatesta".to_owned());
    assert_eq!(
        call(
            rig.addr,
            "GET",
            OFFICE_SESSION,
            &[("Cookie", cookie.clone())],
            ""
        )
        .status,
        401,
        "an account that is gone ends its sessions"
    );
    assert_eq!(
        call(rig.addr, "GET", web::HOST_INDEX, &[("Cookie", cookie)], "").status,
        401
    );
    // A claim for an account that went between the dialog and the pane opens nothing.
    let string = {
        rig.accounts.gone.lock().unwrap().clear();
        dialog_sign_in(&rig, "canoatesta", "pw-a")
    };
    rig.accounts
        .gone
        .lock()
        .unwrap()
        .insert("canoatesta".to_owned());
    assert_eq!(claim(&rig, &string, &[]).status, 401);
}

#[test]
fn a_string_the_pane_does_not_claim_in_time_is_good_for_nothing() {
    let rig = office_rig(|settings| settings.office_handoff = Duration::from_millis(150));
    let string = dialog_sign_in(&rig, "canoatesta", "pw-a");
    thread::sleep(Duration::from_millis(300));
    assert_eq!(claim(&rig, &string, &[]).status, 401);
}

#[test]
fn one_account_does_not_claim_another_accounts_string_and_a_string_names_one_sign_in() {
    let rig = office_rig(|_| {});
    let a = dialog_sign_in(&rig, "canoatesta", "pw-a");
    let b = dialog_sign_in(&rig, "canoatestb", "pw-b");
    assert_ne!(a, b);
    let first = claim(&rig, &b, &[]);
    let cookie = set(&first, PANE_COOKIE).unwrap();
    let who = call(rig.addr, "GET", OFFICE_SESSION, &[("Cookie", cookie)], "");
    assert!(who.body.contains("canoatestb") && !who.body.contains("canoatesta"));
}

#[test]
fn the_site_cookie_still_wins_and_the_two_never_make_each_other_ambiguous() {
    let rig = office_rig(|_| {});
    let site = signed_in(&rig, "canoatesta");
    let pane = pane(&rig, "canoatestb", "pw-b");
    let both = format!("{site}; {pane}");
    let answer = call(rig.addr, "GET", "/portal/session", &[("Cookie", both)], "");
    assert_eq!(answer.status, 200);
    assert!(answer.body.contains("canoatesta"), "{}", answer.body);
}

#[test]
fn the_settings_name_the_add_in_and_refuse_an_origin_that_is_not_one() {
    let on = Settings::from_toml(
        "office = true\noffice_frames = [\"https://*.officeapps.live.com\"]\noffice_handoff_seconds = 30",
    )
    .unwrap();
    assert!(on.office);
    assert_eq!(on.office_frames, ["https://*.officeapps.live.com"]);
    assert_eq!(on.office_handoff, Duration::from_secs(30));
    assert!(!Settings::default().office);
    for bad in [
        "http://x.example",
        "https://*.com",
        "https://x.example; frame-src *",
        "x.example",
        "https://",
    ] {
        let text = format!("office = true\noffice_frames = [\"{bad}\"]");
        let why = Settings::from_toml(&text).unwrap_err();
        assert!(why.starts_with("office_frames"), "{bad}: {why}");
    }
    assert!(Settings::from_toml("office_handoff_seconds = 0").is_err());
}
