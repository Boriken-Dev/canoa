use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::files::workspace::{Machine, Opener, Os, Plan};
use super::files::{Files, UploadLimits};
use super::*;

/// A computer that starts nothing: it records what it was asked to start.
struct StandIn {
    os: Os,
    vars: BTreeMap<String, String>,
    files: BTreeSet<String>,
    registry: BTreeMap<String, String>,
    started: Mutex<Vec<Plan>>,
}

impl StandIn {
    fn new(os: Os) -> Self {
        Self {
            os,
            vars: BTreeMap::new(),
            files: BTreeSet::new(),
            registry: BTreeMap::new(),
            started: Mutex::new(Vec::new()),
        }
    }
}

impl Machine for StandIn {
    fn os(&self) -> Os {
        self.os
    }
    fn var(&self, name: &str) -> Option<String> {
        self.vars.get(name).cloned()
    }
    fn is_file(&self, path: &str) -> bool {
        self.files.contains(path)
    }
    fn registry_default(&self, key: &str) -> Option<String> {
        self.registry.get(key).cloned()
    }
    fn bundle_path(&self, _: &str) -> Option<String> {
        None
    }
    fn start(&self, plan: &Plan) -> Result<(), String> {
        self.started.lock().expect("lock").push(plan.clone());
        Ok(())
    }
}

/// The same stand-in, shared so a test reads what it started after the control owns it.
struct Shared(Arc<StandIn>);

impl Machine for Shared {
    fn os(&self) -> Os {
        self.0.os()
    }
    fn var(&self, name: &str) -> Option<String> {
        self.0.var(name)
    }
    fn is_file(&self, path: &str) -> bool {
        self.0.is_file(path)
    }
    fn registry_default(&self, key: &str) -> Option<String> {
        self.0.registry_default(key)
    }
    fn bundle_path(&self, id: &str) -> Option<String> {
        self.0.bundle_path(id)
    }
    fn start(&self, plan: &Plan) -> Result<(), String> {
        self.0.start(plan)
    }
}

fn dir(name: &str) -> PathBuf {
    static NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("canoa-open-{name}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("directory");
    fs::canonicalize(&dir)
        .map(|real| {
            let text = real.to_string_lossy().into_owned();
            PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
        })
        .expect("resolves")
}

fn ask(control: &Control, caller: &Caller, method: &str, params: Value) -> Value {
    let line =
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .expect("encodes");
    serde_json::from_slice(&control.handle(caller, &line).expect("answered")).expect("JSON")
}

fn key(grants: &[Grant]) -> Caller {
    Caller::Peer {
        id: "sha256:aa".to_owned(),
        grants: grants.iter().copied().collect(),
    }
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn control_on(machine: &Arc<StandIn>, home: &Path) -> Control {
    Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_files(
        Files::new(
            Some(home.to_path_buf()),
            Vec::new(),
            None,
            None,
            UploadLimits::default(),
        )
        .with_opener(Opener::on(Arc::new(Shared(machine.clone())))),
    )
}

fn linux_with_code() -> Arc<StandIn> {
    let mut machine = StandIn::new(Os::Linux);
    machine
        .vars
        .insert("PATH".to_owned(), "/usr/bin:/opt/code/bin".to_owned());
    machine.files.insert("/opt/code/bin/code".to_owned());
    Arc::new(machine)
}

#[test]
fn both_methods_need_the_workspace_grant() {
    let tmp = dir("grant");
    let machine = linux_with_code();
    let control = control_on(&machine, &tmp);
    let without = key(&[Grant::Folders, Grant::Files, Grant::StoreRead]);
    for (method, params) in [
        ("workspace/openers", json!({})),
        (
            "workspace/open",
            json!({"path": text(&tmp), "opener": "file_manager"}),
        ),
    ] {
        let said = ask(&control, &without, method, params);
        assert_eq!(said["error"]["code"], -32001, "{method}: {said}");
    }
    assert!(machine.started.lock().expect("lock").is_empty());
    assert_eq!(Grant::named("workspace.open"), Some(Grant::WorkspaceOpen));
}

#[test]
fn the_openers_are_what_this_computer_has() {
    let tmp = dir("openers");
    let granted = key(&[Grant::WorkspaceOpen]);
    let found = ask(
        &control_on(&linux_with_code(), &tmp),
        &granted,
        "workspace/openers",
        json!({}),
    );
    let ids: Vec<&str> = found["result"]["openers"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|opener| opener["id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids, ["vscode", "file_manager", "reveal"], "{found}");
    assert_eq!(found["result"]["os"], "linux");

    let bare = Arc::new(StandIn::new(Os::Linux));
    let found = ask(
        &control_on(&bare, &tmp),
        &granted,
        "workspace/openers",
        json!({}),
    );
    assert_eq!(
        found["result"]["openers"][0]["id"], "file_manager",
        "without VS Code it is not offered: {found}"
    );
}

#[test]
fn a_folder_opens_in_the_program_chosen_and_nothing_real_starts() {
    let tmp = dir("start");
    let machine = linux_with_code();
    let control = control_on(&machine, &tmp);
    let granted = key(&[Grant::WorkspaceOpen]);
    for opener in ["vscode", "file_manager"] {
        let said = ask(
            &control,
            &granted,
            "workspace/open",
            json!({"path": text(&tmp), "opener": opener}),
        );
        assert_eq!(said["result"]["started"], true, "{opener}: {said}");
    }
    let file = tmp.join("a.txt");
    fs::write(&file, "x").expect("file");
    let said = ask(
        &control,
        &granted,
        "workspace/open",
        json!({"path": text(&file), "opener": "reveal"}),
    );
    assert_eq!(said["result"]["started"], true, "{said}");
    let started = machine.started.lock().expect("lock").clone();
    let folder = text(&tmp);
    let plan = |program: &str| Plan {
        program: program.to_owned(),
        args: vec![folder.clone()],
    };
    assert_eq!(started.len(), 3);
    assert_eq!(started[..2], [plan("/opt/code/bin/code"), plan("xdg-open")]);
    // On this stand-in's system a file is shown by opening the folder that holds it.
    assert_eq!(started[2].program, "xdg-open");
}

#[test]
fn a_request_that_cannot_be_done_says_why_and_starts_nothing() {
    let tmp = dir("refuse");
    let machine = Arc::new(StandIn::new(Os::Linux));
    let control = control_on(&machine, &tmp);
    let granted = key(&[Grant::WorkspaceOpen]);
    let file = tmp.join("a.txt");
    fs::write(&file, "x").expect("file");
    for (params, code) in [
        (json!({"path": text(&tmp), "opener": "vscode"}), -32002),
        (
            json!({"path": text(&file), "opener": "file_manager"}),
            -32003,
        ),
        (json!({"path": text(&tmp), "opener": "rm"}), -32602),
        (json!({"path": text(&tmp)}), -32602),
        (json!({"opener": "file_manager"}), -32602),
        (
            json!({"path": text(&tmp.join("missing")), "opener": "file_manager"}),
            -32003,
        ),
    ] {
        let said = ask(&control, &granted, "workspace/open", params.clone());
        assert_eq!(said["error"]["code"], code, "{params}: {said}");
    }
    let said = ask(
        &control,
        &granted,
        "workspace/open",
        json!({"path": "relative", "opener": "file_manager"}),
    );
    assert!(said.get("error").is_some(), "{said}");
    assert!(machine.started.lock().expect("lock").is_empty());
}

#[test]
fn a_key_opens_only_the_places_it_may_browse() {
    let home = dir("home");
    let outside = dir("outside");
    let machine = linux_with_code();
    let control = control_on(&machine, &home);
    let granted = key(&[Grant::WorkspaceOpen]);
    let said = ask(
        &control,
        &granted,
        "workspace/open",
        json!({"path": text(&outside), "opener": "file_manager"}),
    );
    assert!(said.get("error").is_some(), "{said}");
    assert!(machine.started.lock().expect("lock").is_empty());
    let said = ask(
        &control,
        &Caller::Owner,
        "workspace/open",
        json!({"path": text(&outside), "opener": "file_manager"}),
    );
    assert_eq!(said["result"]["started"], true, "{said}");
}

#[test]
fn windows_starts_code_exe_and_selects_in_explorer_with_the_path_as_one_argument() {
    let mut machine = StandIn::new(Os::Windows);
    machine
        .vars
        .insert("SystemRoot".to_owned(), r"C:\Windows".to_owned());
    machine.registry.insert(
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\App Paths\Code.exe".to_owned(),
        r"C:\Apps\Code.exe".to_owned(),
    );
    machine.files.insert(r"C:\Apps\Code.exe".to_owned());
    let opener = Opener::on(Arc::new(machine));
    assert_eq!(
        opener.vs_code(r"C:\work\a b").expect("found"),
        Plan {
            program: r"C:\Apps\Code.exe".to_owned(),
            args: vec![r"C:\work\a b".to_owned()]
        }
    );
    assert_eq!(
        opener.file_manager(r"C:\work\a b", true),
        Plan {
            program: r"C:\Windows\explorer.exe".to_owned(),
            args: vec!["/select,".to_owned(), r"C:\work\a b".to_owned()]
        }
    );
}

#[test]
fn the_computers_own_starter_runs_a_program_detached_and_names_one_it_cannot_start() {
    // The one test that starts a real process: this test program itself, asked only to list its tests.
    let me = std::env::current_exe().expect("this program");
    let real = Opener::default();
    real.start(&Plan {
        program: text(&me),
        args: vec!["--list".to_owned()],
    })
    .expect("starts");
    let why = real
        .start(&Plan {
            program: text(&me.with_file_name("no-such-program-here")),
            args: Vec::new(),
        })
        .expect_err("is not there");
    assert!(why.contains("could not start"), "{why}");
}

#[cfg(windows)]
#[test]
fn open_in_takes_a_folder_on_a_share_written_plain() {
    let local = dir("share");
    let text_local = text(&local);
    let (drive, rest) = text_local.split_at(2);
    let share = PathBuf::from(format!(r"\\localhost\{}${rest}", &drive[..1]));
    let machine = Arc::new(StandIn::new(Os::Windows));
    let control = control_on(&machine, &std::env::temp_dir());
    for written in [text(&share), format!(r"\\?\UNC\{}", &text(&share)[2..])] {
        let said = ask(
            &control,
            &Caller::Owner,
            "workspace/open",
            json!({"path": written, "opener": "file_manager"}),
        );
        assert_eq!(said["result"]["started"], true, "{said}");
    }
    let started = machine.started.lock().expect("lock");
    assert_eq!(started.len(), 2);
    for plan in started.iter() {
        assert_eq!(plan.args, vec![text(&share)], "{plan:?}");
    }
    drop(started);
    let _ = fs::remove_dir_all(local);
}
