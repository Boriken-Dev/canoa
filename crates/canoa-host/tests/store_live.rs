//! Each built-in store against its real publisher: plan, install into a throwaway programs folder, read the version back,
//! check that the real home, the real profile files, the real `PATH` directories and (on Windows) the real `HKCU` keys did not
//! change, then remove it. It downloads hundreds of megabytes and touches the network, so it runs only when asked:
//!
//! `CANOA_STORE_LIVE=claude,codex,grok,agy,pi cargo test -p canoa-host --test store_live -- --ignored --nocapture`
//!
//! It spends nothing: no program is logged in or given a model turn, and each is started once with its version flag.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use canoa::store::{Action, ProgramStore, StoreSettings};
use canoa_host::store::footprint::{Footprint, Watch};
use canoa_host::store::programs::{Event, Options, Programs, Sink};
use canoa_host::store::registry::built_in;
use canoa_host::store::shelf::Shelf;

struct Print;

impl Sink for Print {
    fn event(&self, event: Event) {
        match event {
            Event::Say(line) => println!("    | {line}"),
            Event::Progress(p) => println!("    # {} {}/{:?}", p.label, p.done, p.total),
            _ => {}
        }
    }

    fn ask(&self, _form: coqui_acp_api::LoginForm) -> Result<coqui_acp_api::LoginAnswer, String> {
        Err("no one to ask".to_owned())
    }
}

fn throwaway() -> PathBuf {
    let mut nonce = [0u8; 4];
    getrandom::fill(&mut nonce).expect("random");
    let hex: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
    let dir = std::env::temp_dir().join(format!("canoa-live-{hex}"));
    fs::create_dir_all(&dir).expect("a directory");
    dir
}

#[test]
#[ignore = "downloads the real programs"]
fn each_store_installs_its_real_program_into_a_throwaway_folder_and_changes_nothing_else() {
    let wanted = std::env::var("CANOA_STORE_LIVE").unwrap_or_default();
    if wanted.is_empty() {
        println!("CANOA_STORE_LIVE names none: nothing to do");
        return;
    }
    let home = PathBuf::from(
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).expect("a home"),
    );
    let mut options = Options::default();
    // A host that runs as root installs only when told it may: a probe machine without another user says so here.
    options.allow_elevated = std::env::var_os("CANOA_STORE_LIVE_ELEVATED").is_some();
    // Grok's publisher gives nothing to check its binary against: the plan says so, and this run takes it.
    options.allow_unverified = true;
    let path_dirs: Vec<PathBuf> = canoa_host::store::footprint::user_path_dirs(
        &std::ffi::OsString::from(options.environment.get("PATH").unwrap_or("")),
    );
    let root = throwaway();
    // A probe starts the installed program: its own state goes to a throwaway home, not the real one.
    for name in [
        "HOME",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "CODEX_HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
    ] {
        options
            .environment
            .insert(name, root.join("probe-home").to_string_lossy().into_owned());
    }
    fs::create_dir_all(root.join("probe-home")).expect("probe home");
    let watch = |skip: &PathBuf| {
        Watch::new(&home)
            .path_dirs(path_dirs.clone())
            .skip(skip)
            .registry(true)
    };
    let before = Footprint::take(&watch(&root));
    let programs = Programs::new(
        Shelf::unchecked(root.join("programs")),
        built_in().expect("the built-in stores"),
    )
    .options(options);
    let mut failed = Vec::new();
    for id in wanted.split(',').filter(|s| !s.is_empty()) {
        let provider = format!("{id}_store");
        println!("== {provider}");
        let settings = StoreSettings::default();
        let store = programs
            .store(&provider, &settings)
            .expect("the store is registered");
        let latest = programs.latest(&*store, None).expect("latest");
        println!(
            "  latest: {:?} {:?}",
            latest.version, latest.unknown_because
        );
        let plan = match programs.plan(&provider, &*store, Action::Install, None, None) {
            Ok(plan) => plan,
            Err(error) => {
                failed.push(format!("{provider}: plan: {error}"));
                continue;
            }
        };
        println!(
            "  plan: {:?} {} download(s), changes {:?}, notices {:?}",
            plan.version,
            plan.downloads.len(),
            plan.changes,
            plan.notices
        );
        match programs.apply(
            &provider,
            &settings,
            &plan,
            Arc::new(Print),
            Arc::new(AtomicBool::new(false)),
        ) {
            Ok(Some(applied)) => {
                println!(
                    "  installed {} at {}",
                    applied.current.version, applied.current.launch.path
                );
                let report = programs.report(&*store, true, None).expect("report");
                let first = &report.copies[0];
                println!(
                    "  program(): managed={} version={:?} line={:?}",
                    first.managed, first.version, first.version_line
                );
                if first.version != plan.version {
                    failed.push(format!(
                        "{provider}: probe read {:?}, planned {:?}",
                        first.version, plan.version
                    ));
                }
                programs.remove(&*store).expect("remove");
            }
            Ok(None) => failed.push(format!("{provider}: nothing was applied")),
            Err(error) => failed.push(format!("{provider}: apply: {error}")),
        }
    }
    let after = Footprint::take(&watch(&root));
    // The agent running this test keeps its own state in the home: that is not an install's.
    let differences: Vec<_> = before
        .diff(&after)
        .into_iter()
        .filter(|d| !d.what.contains(".claude.json"))
        .collect();
    let _ = fs::remove_dir_all(&root);
    for item in &differences {
        println!("  CHANGED {} ({})", item.what, item.kind);
    }
    assert!(failed.is_empty(), "{failed:#?}");
    assert!(
        differences.is_empty(),
        "the real machine changed: {differences:#?}"
    );
    let _ = &before.home();
    let _: Option<&dyn ProgramStore> = None;
}
