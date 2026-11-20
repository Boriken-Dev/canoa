//! `tools/office_addin.ps1` against a scratch registry key, never the one Office reads. Needs PowerShell (the Windows one
//! or `pwsh`) and the Windows registry.

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

use canoa_office::{CLSID_ADDIN, CLSID_PANE, PANE_PROG_ID, PROG_ID};
use windows_registry::CURRENT_USER;

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/office_addin.ps1");

fn shell() -> &'static str {
    ["pwsh", "powershell"]
        .into_iter()
        .find(|name| {
            Command::new(name)
                .args(["-NoProfile", "-Command", "exit 0"])
                .status()
                .is_ok_and(|s| s.success())
        })
        .expect("the tests need PowerShell")
}

/// A key nobody reads, named by this process, under HKCU's `Software\CanoaTest`; every other key is refused.
struct Scratch {
    path: String,
    // One test at a time: removing the shared parent must not race another test's key.
    _one_at_a_time: MutexGuard<'static, ()>,
}

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

impl Scratch {
    fn new(name: &str) -> Self {
        let path = format!(r"Software\CanoaTest\{name}-{}", std::process::id());
        assert!(path.starts_with(r"Software\CanoaTest\"));
        let guard = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let scratch = Self {
            path,
            _one_at_a_time: guard,
        };
        scratch.clean();
        scratch
    }

    /// The key as the script names it.
    fn root(&self) -> String {
        format!(r"HKCU:\{}", self.path)
    }

    fn clean(&self) {
        let _ = CURRENT_USER.remove_tree(&self.path);
        // The folder above goes too once nothing else is in it.
        if let Ok(parent) = CURRENT_USER.open(r"Software\CanoaTest")
            && parent.keys().is_ok_and(|mut keys| keys.next().is_none())
            && parent.values().is_ok_and(|mut v| v.next().is_none())
        {
            drop(parent);
            let _ = CURRENT_USER.remove_tree(r"Software\CanoaTest");
        }
    }

    fn exists(&self, sub: &str) -> bool {
        CURRENT_USER.open(format!(r"{}\{sub}", self.path)).is_ok()
    }

    fn run(&self, action: &str, dll: &Path, apps: &str, root: Option<&str>) -> (bool, String) {
        let root = root.map_or_else(|| self.root(), str::to_owned);
        let done = Command::new(shell())
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-File",
                SCRIPT,
                action,
                "-Dll",
            ])
            .arg(dll)
            .args(["-Apps", apps, "-Root", &root])
            .output()
            .expect("PowerShell starts");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&done.stdout),
            String::from_utf8_lossy(&done.stderr)
        );
        (done.status.success(), text.trim().to_owned())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        self.clean();
    }
}

fn dll() -> PathBuf {
    // Any file will do: the script only checks it is one and records its path.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
}

fn guid(g: &windows_core::GUID) -> String {
    format!("{{{g:?}}}")
}

#[test]
fn the_script_names_the_ids_the_code_names() {
    let script = std::fs::read_to_string(SCRIPT).unwrap();
    for (name, value) in [
        ("progId", PROG_ID.to_owned()),
        ("clsid", guid(&CLSID_ADDIN)),
        ("paneProgId", PANE_PROG_ID.to_owned()),
        ("paneClsid", guid(&CLSID_PANE)),
    ] {
        let line = format!("${name} = '{value}'");
        assert!(script.contains(&line), "the script lacks `{line}`");
    }
}

#[test]
fn add_writes_the_per_user_keys_and_remove_takes_exactly_those() {
    let scratch = Scratch::new("office-add");
    let dll = dll();
    // A neighbour the script must never touch: another add-in of the same application, and another class.
    let neighbour_addin = format!(r"{}\Microsoft\Office\Word\Addins\other.addin", scratch.path);
    let neighbour_class = format!(
        r"{}\Classes\CLSID\{{00000000-0000-0000-0000-000000000001}}",
        scratch.path
    );
    CURRENT_USER.create(&neighbour_addin).unwrap();
    CURRENT_USER.create(&neighbour_class).unwrap();

    for _ in 0..2 {
        let (ok, out) = scratch.run("add", &dll, "Word,Outlook", None);
        assert!(ok, "{out}");
    }

    let clsid = guid(&CLSID_ADDIN);
    let pane = guid(&CLSID_PANE);
    let root = &scratch.path;
    let path = std::fs::canonicalize(&dll).unwrap();
    let server = CURRENT_USER
        .open(format!(r"{root}\Classes\CLSID\{clsid}\InprocServer32"))
        .unwrap();
    // `canonicalize` gives the `\\?\` form, which the script's `Resolve-Path` does not.
    assert_eq!(
        server.get_string("").unwrap(),
        path.to_string_lossy().trim_start_matches(r"\\?\")
    );
    assert_eq!(server.get_string("ThreadingModel").unwrap(), "Apartment");
    assert_eq!(
        CURRENT_USER
            .open(format!(r"{root}\Classes\{PROG_ID}\CLSID"))
            .unwrap()
            .get_string("")
            .unwrap(),
        clsid
    );
    assert_eq!(
        CURRENT_USER
            .open(format!(r"{root}\Classes\{PANE_PROG_ID}\CLSID"))
            .unwrap()
            .get_string("")
            .unwrap(),
        pane
    );
    assert!(scratch.exists(&format!(r"Classes\CLSID\{pane}\Control")));
    assert_eq!(
        CURRENT_USER
            .open(format!(r"{root}\Classes\CLSID\{pane}\MiscStatus\1"))
            .unwrap()
            .get_string("")
            .unwrap(),
        "131473"
    );
    for app in ["Word", "Outlook"] {
        let key = CURRENT_USER
            .open(format!(r"{root}\Microsoft\Office\{app}\Addins\{PROG_ID}"))
            .unwrap();
        assert_eq!(key.get_u32("LoadBehavior").unwrap(), 3);
        assert_eq!(key.get_string("FriendlyName").unwrap(), "Canoa");
    }
    assert!(
        !scratch.exists(r"Microsoft\Office\Excel"),
        "an application not asked for gets nothing"
    );

    let (ok, status) = scratch.run("status", &dll, "Word,Outlook", None);
    assert!(ok && !status.contains("absent"), "{status}");

    for _ in 0..2 {
        let (ok, out) = scratch.run("remove", &dll, "Word,Outlook", None);
        assert!(ok, "{out}");
    }
    for key in [
        format!(r"Classes\{PROG_ID}"),
        format!(r"Classes\{PANE_PROG_ID}"),
        format!(r"Classes\CLSID\{clsid}"),
        format!(r"Classes\CLSID\{pane}"),
        format!(r"Microsoft\Office\Word\Addins\{PROG_ID}"),
        format!(r"Microsoft\Office\Outlook\Addins\{PROG_ID}"),
    ] {
        assert!(!scratch.exists(&key), "{key} is still there");
    }
    assert!(
        scratch.exists(r"Microsoft\Office\Word\Addins\other.addin"),
        "a neighbour add-in was removed"
    );
    assert!(
        scratch.exists(r"Classes\CLSID\{00000000-0000-0000-0000-000000000001}"),
        "a neighbour class was removed"
    );
}

#[test]
fn a_root_outside_hkcu_is_refused_and_nothing_is_written() {
    let scratch = Scratch::new("office-refuse");
    for root in [
        r"HKLM:\Software\CanoaTest\office-refuse",
        r"HKCR:\CanoaTest",
        r"Software\CanoaTest\no-drive",
    ] {
        let (ok, out) = scratch.run("add", &dll(), "Word", Some(root));
        assert!(!ok, "{root} was accepted: {out}");
        assert!(out.contains("HKCU"), "{out}");
    }
    assert!(!scratch.exists(""));
}

#[test]
fn a_dll_that_is_not_a_file_is_refused() {
    let scratch = Scratch::new("office-nodll");
    let (ok, out) = scratch.run(
        "add",
        Path::new(r"C:\no\such\canoa_office.dll"),
        "Word",
        None,
    );
    assert!(!ok && out.contains("not a file"), "{out}");
    assert!(!scratch.exists(""));
}
