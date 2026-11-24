//! The sign-in page's chooser: the ways in as tabs in the policy's order, the first one selected, and only the selected form in
//! the page.

use super::sign_in_policy::{form_at, proxied};
use super::*;

const CERTIFICATE_FIRST: &str = r#"
[[sign_in_policy.config.rule]]
allow = [["certificate"], ["password"]]
"#;

const PASSWORD_FIRST: &str = r#"
[[sign_in_policy.config.rule]]
allow = [["password"], ["certificate"]]
"#;

fn page(rig: &Rig, target: &str) -> String {
    let answer = request(
        &proxied(rig),
        "GET",
        target,
        &[("X-Forwarded-For", "10.1.2.3".to_owned())],
        "",
    );
    assert_eq!(answer.status, 200, "{}", answer.body);
    answer.body
}

fn under(policy: &str) -> Rig {
    rig(
        |settings| settings.sign_in_policy = Settings::from_toml(policy).unwrap().sign_in_policy,
        0,
    )
}

/// The tab that is selected, by the way it names.
fn selected(body: &str) -> String {
    let at = body
        .find("aria-selected=\"true\"")
        .unwrap_or_else(|| panic!("no tab is selected: {body}"));
    let before = &body[..at];
    let from = before.rfind("id=\"tab-").unwrap() + "id=\"tab-".len();
    before[from..].split('"').next().unwrap().to_owned()
}

#[test]
fn the_tabs_follow_the_policys_order_and_the_first_is_selected() {
    for (policy, first, second) in [
        (CERTIFICATE_FIRST, "certificate", "password"),
        (PASSWORD_FIRST, "password", "certificate"),
    ] {
        let rig = under(policy);
        let body = page(&rig, LOGIN);
        let (one, two) = (
            body.find(&format!("id=\"tab-{first}\"")).unwrap(),
            body.find(&format!("id=\"tab-{second}\"")).unwrap(),
        );
        assert!(one < two, "{first} comes first: {body}");
        assert_eq!(selected(&body), first);
        assert!(body.contains("role=\"tablist\"") && body.contains("role=\"tabpanel\""));
        // Only the selected way's form is in the page.
        assert_eq!(
            body.contains("name=\"password\""),
            first == "password",
            "{body}"
        );
        assert!(!body.contains("class=\"or\""), "{body}");
    }
}

#[test]
fn a_method_in_the_address_selects_it_and_an_unknown_one_leaves_the_first() {
    let rig = under(CERTIFICATE_FIRST);
    let chosen = page(&rig, &format!("{LOGIN}?method=password"));
    assert_eq!(selected(&chosen), "password");
    assert!(chosen.contains("name=\"password\""));
    assert!(!chosen.contains("Sign in with your certificate"));
    for word in ["nonsense", "totp", "", "cancel"] {
        let body = page(&rig, &format!("{LOGIN}?method={word}"));
        assert_eq!(selected(&body), "certificate", "{word:?}");
        assert!(body.contains("Sign in with your certificate"), "{word:?}");
    }
}

#[test]
fn a_failed_attempt_shows_the_tab_it_came_from() {
    let rig = under(CERTIFICATE_FIRST);
    let failed = form_at(
        &rig,
        Some("10.1.2.3"),
        None,
        "method=password&user=nobody&password=wrong",
    );
    assert_eq!(failed.status, 401, "{}", failed.body);
    assert!(failed.body.contains("Sign-in failed."));
    assert_eq!(selected(&failed.body), "password");
    assert!(failed.body.contains("name=\"password\""));
}

#[test]
fn one_way_in_is_its_form_alone_and_a_list_stands_in_for_the_tabs_script_side() {
    let rig = rig(
        |settings| {
            settings.sign_in_policy = Settings::from_toml(CERTIFICATE_FIRST)
                .unwrap()
                .sign_in_policy;
        },
        0,
    );
    let body = page(&rig, LOGIN);
    // The list is in the page, hidden until the script runs, with an address for each way.
    assert!(
        body.contains("<select class=\"pick\" hidden data-pick"),
        "{body}"
    );
    assert!(body.contains(&format!("<option value=\"{LOGIN}?method=password\"")));
    let script = request(&proxied(&rig), "GET", page::JS_PATH, &[], "").body;
    assert!(script.contains("ArrowRight") && script.contains("data-pick"));
    let single = rig_single();
    let body = page(&single, LOGIN);
    assert!(
        !body.contains("role=\"tablist\"") && !body.contains("<select"),
        "{body}"
    );
}

fn rig_single() -> Rig {
    under("[[sign_in_policy.config.rule]]\nallow = [[\"certificate\"]]\n")
}
