//! A home a host runs on keeps its method: `id --method` and `pair --method` naming another are refused unless `--force`,
//! which logs it as weakened. The same method, and a home no host runs on, are never refused.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::path::Path;
use std::process::{Command, Output};

use canoa_host::Home;
use coqui_agentic_peer::auth::Method;
use support::{Host, Scratch, example, host_binary, quoted};

fn agent_entry() -> String {
    format!(
        "[host.agents.fixture]\nprogram = {}\n",
        quoted(&example("stdio_agent", false).to_string_lossy())
    )
}

fn other() -> Method {
    Method::ALL
        .into_iter()
        .find(|method| *method != support::method())
        .expect("another method")
}

/// `canoa <args>` on `home`, to its end.
fn canoa(home: &Path, args: &[&str]) -> Output {
    Command::new(host_binary())
        .args(args)
        .env("COQUI_AGENTIC_PEER_HOME", home)
        .output()
        .expect("the command runs")
}

fn recorded(home: &Path) -> String {
    std::fs::read_to_string(home.join("method"))
        .expect("the home records its method")
        .trim()
        .to_owned()
}

fn said(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A home holding an identity of this run's method, as a host that has run there leaves it.
fn home_with_method(scratch: &Scratch) -> std::path::PathBuf {
    let home = scratch.0.join("home");
    let made = canoa(&home, &["id", "--method", support::method().name()]);
    assert!(made.status.success(), "{made:?}");
    home
}

#[test]
fn a_running_standalone_host_refuses_another_method_for_id() {
    let host = Host::start(&agent_entry(), 1);
    let holder = Home::at(host.home()).holder().expect("it says who holds");
    let refused = canoa(&host.home(), &["id", "--method", other().name()]);
    assert_eq!(refused.status.code(), Some(3), "{refused:?}");
    assert!(refused.stdout.is_empty(), "{refused:?}");
    let said = said(&refused);
    assert!(
        said.contains(&format!("canoa (process {})", holder.pid)),
        "{said}"
    );
    assert!(
        said.contains("already-paired clients would be refused after its next restart"),
        "{said}"
    );
    assert!(said.contains("--force"), "{said}");
    assert_eq!(recorded(&host.home()), support::method().name());
    let ours = host.command(&["id"]);
    assert_eq!(
        String::from_utf8_lossy(&ours.stdout).trim(),
        host.peer.to_string()
    );
}

#[test]
fn force_changes_the_method_of_a_running_host_and_logs_it_as_weakened() {
    let host = Host::start(&agent_entry(), 1);
    let forced = canoa(&host.home(), &["id", "--force", "--method", other().name()]);
    assert!(forced.status.success(), "{forced:?}");
    let said = said(&forced);
    assert!(said.starts_with("weakened: "), "{said}");
    assert!(said.contains("already-paired clients"), "{said}");
    assert_eq!(recorded(&host.home()), other().name());
}

#[test]
fn the_method_a_home_already_has_is_never_refused() {
    let host = Host::start(&agent_entry(), 1);
    let same = canoa(&host.home(), &["id", "--method", support::method().name()]);
    assert!(same.status.success(), "{same:?}");
    assert!(said(&same).is_empty(), "{same:?}");
    assert_eq!(
        String::from_utf8_lossy(&same.stdout).trim(),
        host.peer.to_string()
    );
    // Naming none keeps the recorded one too.
    let none = canoa(&host.home(), &["id"]);
    assert!(none.status.success(), "{none:?}");
}

#[test]
fn with_no_host_running_the_method_changes_freely() {
    let scratch = Scratch::new();
    let home = home_with_method(&scratch);
    let changed = canoa(&home, &["id", "--method", other().name()]);
    assert!(changed.status.success(), "{changed:?}");
    assert!(said(&changed).is_empty(), "{changed:?}");
    assert_eq!(recorded(&home), other().name());
}

#[test]
fn an_embedded_host_holding_the_home_refuses_another_method_too() {
    let scratch = Scratch::new();
    let home = home_with_method(&scratch);
    let lock = Home::at(&home).lock("CanoaUI").expect("a free home locks");
    let refused = canoa(&home, &["id", "--method", other().name()]);
    assert_eq!(refused.status.code(), Some(3), "{refused:?}");
    let said_by = said(&refused);
    assert!(
        said_by.contains(&format!("CanoaUI (process {})", std::process::id())),
        "{said_by}"
    );
    assert_eq!(recorded(&home), support::method().name());
    let forced = canoa(&home, &["id", "--force", "--method", other().name()]);
    assert!(forced.status.success(), "{forced:?}");
    assert!(said(&forced).starts_with("weakened: "), "{forced:?}");
    assert_eq!(recorded(&home), other().name());
    drop(lock);
}

#[test]
fn pair_is_guarded_the_same_way() {
    let scratch = Scratch::new();
    let home = home_with_method(&scratch);
    let lock = Home::at(&home).lock("CanoaUI").expect("a free home locks");
    let pair = |extra: &[&str]| {
        let mut args = vec!["pair", "--timeout-seconds", "1", "--method", other().name()];
        args.extend_from_slice(extra);
        canoa(&home, &args)
    };
    let refused = pair(&[]);
    assert_eq!(refused.status.code(), Some(3), "{refused:?}");
    assert!(
        said(&refused).contains("already-paired clients would be refused after its next restart"),
        "{refused:?}"
    );
    assert_eq!(recorded(&home), support::method().name());
    let forced = pair(&["--force"]);
    assert!(said(&forced).starts_with("weakened: "), "{forced:?}");
    assert_eq!(recorded(&home), other().name());
    drop(lock);
}
