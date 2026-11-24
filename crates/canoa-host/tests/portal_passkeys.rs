//! `canoa portal passkeys`: an administrator lists and removes passkeys in the files the portal reads, and no listing carries a key.
#![cfg(all(unix, feature = "portal"))]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

// The software passkey device the portal's own tests sign with.
#[allow(
    dead_code,
    reason = "the portal's own tests use more of the device than this one"
)]
#[path = "../src/portal/device.rs"]
mod device;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use canoa_host::portal::{Access, Accounts as _, Settings};

/// The library cargo built beside the tests for the passkey provider; its file name carries a hash.
fn plugin() -> PathBuf {
    let deps = PathBuf::from(env!("CARGO_BIN_EXE_canoa"))
        .parent()
        .unwrap()
        .join("deps");
    std::fs::read_dir(deps)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|file| {
            let name = file.file_name().to_string_lossy().into_owned();
            name.starts_with("libcoqui_access_passkey_plugin-") && name.ends_with(".so")
        })
        .max_by_key(|file| file.metadata().and_then(|meta| meta.modified()).ok())
        .map(|file| file.path())
        .expect("the passkey plugin is built beside the tests")
}

fn me() -> String {
    let output = Command::new("id").arg("-un").output().expect("id");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn my_uid() -> u32 {
    let output = Command::new("id").arg("-u").output().expect("id");
    String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

struct Folder(PathBuf);

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn portal_toml(with_passkeys: bool) -> (Folder, PathBuf) {
    static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "canoa-passkeys-cli-{}-{}",
        std::process::id(),
        COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let passkeys = if with_passkeys {
        format!(
            ", {{ provider = \"coqui_access_passkey\", library = \"{}\", config = {{ rp_id = \"{}\", origins = [\"{}\"], source = {{ file = {{ path = \"{}\", format = \"toml\" }} }} }} }}",
            plugin().display(),
            device::RP_ID,
            device::ORIGIN,
            dir.join("passkeys.toml").display()
        )
    } else {
        String::new()
    };
    let file = dir.join("portal.toml");
    std::fs::write(
        &file,
        format!(
            "listen = \"127.0.0.1:0\"\nsite = \"{}\"\nlibraries_any_owner = true\nmin_uid = {}\nauthenticators = [{{ provider = \"coqui_access_system\" }}{passkeys}]\n",
            dir.display(),
            my_uid().max(1)
        ),
    )
    .unwrap();
    (Folder(dir), file)
}

fn passkeys(config: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_canoa"))
        .args(["portal", "--config"])
        .arg(config)
        .arg("passkeys")
        .args(args)
        .output()
        .expect("canoa runs")
}

fn lines(output: &Output) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Adds a passkey to `name` as the portal's page does: the provider's options, the device's answer, the confirmation.
fn add(config: &Path, name: &str, label: &str) -> String {
    let settings = Settings::from_toml(&std::fs::read_to_string(config).unwrap()).unwrap();
    let access = Access::new(&settings).unwrap();
    let mut device = device::Device::new();
    let options = access.begin_passkey(name, my_uid()).unwrap();
    let answer = device.register(device::ORIGIN, &options).unwrap();
    access
        .confirm_passkey(name, my_uid(), &answer, label)
        .unwrap()
}

#[test]
fn the_help_of_every_passkey_command_prints() {
    let (_folder, config) = portal_toml(true);
    for args in [&["--help"][..], &["list", "--help"], &["remove", "--help"]] {
        let output = passkeys(&config, args);
        assert!(output.status.success(), "{args:?}");
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(
            text.contains("Usage:") && text.contains("passkey"),
            "{text}"
        );
    }
}

#[test]
fn an_administrator_lists_and_removes_passkeys_and_no_listing_carries_a_key() {
    let (_folder, config) = portal_toml(true);
    let name = me();
    let empty = passkeys(&config, &["list"]);
    assert!(empty.status.success());
    assert!(lines(&empty).is_empty());
    let id = add(&config, &name, "desk");
    let listed = passkeys(&config, &["list"]);
    assert!(listed.status.success());
    let each = lines(&listed);
    assert_eq!(each.len(), 1);
    assert_eq!(each[0]["user"], name.as_str());
    assert_eq!(each[0]["id"], id.as_str());
    assert_eq!(each[0]["label"], "desk");
    assert_eq!(each[0]["disabled"], false);
    assert!(each[0]["added"].as_i64().is_some());
    assert!(each[0]["last_used"].is_null());
    let text = String::from_utf8_lossy(&listed.stdout).to_lowercase();
    assert!(
        !text.contains("public") && !text.contains("\"key\""),
        "{text}"
    );
    // One that is not the account's is no removal, and the account keeps what it has.
    let wrong = passkeys(&config, &["remove", &name, "not-an-id"]);
    assert!(!wrong.status.success());
    assert!(String::from_utf8_lossy(&wrong.stderr).contains("no passkey with that id"));
    assert_eq!(lines(&passkeys(&config, &["list"])).len(), 1);
    // An id may start with a hyphen: it reaches the lookup, which finds none, and is not read as an option.
    let hyphen = passkeys(&config, &["remove", &name, "-O_fixed-id"]);
    let said = String::from_utf8_lossy(&hyphen.stderr).into_owned();
    assert!(said.contains("no passkey with that id"), "{said}");
    assert_eq!(lines(&passkeys(&config, &["list"])).len(), 1);
    let removed = passkeys(&config, &["remove", &name, &id]);
    assert!(
        removed.status.success(),
        "{}",
        String::from_utf8_lossy(&removed.stderr)
    );
    assert_eq!(lines(&removed)[0]["removed"]["passkeys"], 1);
    assert!(lines(&passkeys(&config, &["list"])).is_empty());
    // Every passkey of an account at once.
    add(&config, &name, "one");
    add(&config, &name, "two");
    let all = passkeys(&config, &["remove", &name]);
    assert_eq!(lines(&all)[0]["removed"]["passkeys"], 2);
    let none = passkeys(&config, &["remove", &name]);
    assert!(!none.status.success());
    assert!(String::from_utf8_lossy(&none.stderr).contains("has no passkeys"));
}

#[test]
fn a_portal_with_no_passkey_provider_has_nothing_to_list_or_remove() {
    let (_folder, config) = portal_toml(false);
    for args in [&["list"][..], &["remove", "alice"]] {
        let output = passkeys(&config, args);
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("keeps no passkeys"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
