//! The control channel from this process: opened by its name over a link, the ledger answered as the owner's, kept in the
//! host's home, and ended with the host.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod support;

use linked::{Hosted, ask, entry, next};
use serde_json::json;

#[test]
fn the_ledger_is_kept_over_a_link_by_name_and_the_host_stopping_ends_the_link() {
    let host = Hosted::start(&entry("fixture", ""));
    assert!(
        !host.local.entries().contains(&"@control"),
        "it is no entry"
    );
    let link = host.local.open("@control").expect("a link");

    let (put, _) = ask(
        &link,
        1,
        "ledger/put",
        json!({"id": "local:Claude/s1", "record": {"title": "Fix the build", "where": "local"}}),
    )
    .expect("put");
    assert!(put["updated"].as_u64().is_some_and(|ms| ms > 0), "{put}");
    let (listed, _) = ask(&link, 2, "ledger/list", json!({})).expect("listed");
    assert_eq!(listed["records"][0]["id"], "local:Claude/s1");
    assert_eq!(listed["records"][0]["owner"], "local");
    assert_eq!(listed["records"][0]["record"]["title"], "Fix the build");

    // The file is the host's own, in its home.
    let file = host
        .scratch
        .0
        .join("home")
        .join("control")
        .join("ledger.json");
    assert!(file.exists(), "{}", file.display());

    // A second link sees the same ledger.
    let second = host.local.open("@control").expect("another link");
    let (again, _) = ask(&second, 1, "ledger/list", json!({})).expect("listed");
    assert_eq!(again["records"].as_array().map(Vec::len), Some(1));

    host.stop();
    assert!(next(&link).is_err(), "the link ended with the host");
    assert!(next(&second).is_err());
}
