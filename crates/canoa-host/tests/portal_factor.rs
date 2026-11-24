//! `canoa portal factor`: an administrator sets up, lists and resets one-time codes in the files the portal reads, and no
//! answer but the one that makes a setup holds a link or a code.
#![cfg(all(unix, feature = "portal"))]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::path::PathBuf;
use std::process::{Command, Output};

/// The library cargo built beside the tests for the one-time code provider; its file name carries a hash.
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
            name.starts_with("libcoqui_access_totp_plugin-") && name.ends_with(".so")
        })
        .max_by_key(|file| file.metadata().and_then(|meta| meta.modified()).ok())
        .map(|file| file.path())
        .expect("the one-time code plugin is built beside the tests")
}

/// The name of the account this process runs as.
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

fn portal_toml(with_codes: bool) -> (Folder, PathBuf) {
    static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "canoa-factor-cli-{}-{}",
        std::process::id(),
        COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let codes = if with_codes {
        format!(
            ", {{ provider = \"coqui_access_totp\", library = \"{}\", config = {{ source = {{ file = {{ path = \"{}\", format = \"toml\" }} }} }} }}",
            plugin().display(),
            dir.join("totp.toml").display()
        )
    } else {
        String::new()
    };
    let file = dir.join("portal.toml");
    std::fs::write(
        &file,
        format!(
            "listen = \"127.0.0.1:0\"\nsite = \"{}\"\nlibraries_any_owner = true\nmin_uid = {}\nauthenticators = [{{ provider = \"coqui_access_system\" }}{codes}]\n",
            dir.display(),
            my_uid().max(1)
        ),
    )
    .unwrap();
    (Folder(dir), file)
}

fn factor(config: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_canoa"))
        .args(["portal", "--config"])
        .arg(config)
        .arg("factor")
        .args(args)
        .output()
        .expect("canoa runs")
}

/// The code an authenticator app shows now for the secret in a setup link.
fn totp_in(link: &str) -> String {
    use hmac::{Hmac, KeyInit, Mac as _};
    let secret = link
        .split(['?', '&'])
        .find_map(|part| part.strip_prefix("secret="))
        .unwrap();
    let key = data_encoding::BASE32_NOPAD
        .decode(secret.as_bytes())
        .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut mac = <Hmac<sha1::Sha1> as KeyInit>::new_from_slice(&key).unwrap();
    mac.update(&(now / 30).to_be_bytes());
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

/// Sets the account up as an administrator does: the setup link, then its first code.
fn enrolled(config: &std::path::Path, name: &str) {
    let begun = factor(config, &["enrol", name]);
    let uri = lines(&begun)[0]["enrol"]["uri"]
        .as_str()
        .unwrap()
        .to_owned();
    enrolled_with(config, name, &uri);
}

/// Confirms the setup the link belongs to with its first code.
fn enrolled_with(config: &std::path::Path, name: &str, uri: &str) {
    let done = factor(config, &["enrol", name, "--code", &totp_in(uri)]);
    assert!(
        done.status.success(),
        "{}",
        String::from_utf8_lossy(&done.stderr)
    );
}

fn lines(output: &Output) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

#[test]
fn an_administrator_sets_a_code_up_lists_it_and_resets_it() {
    let (_dir, config) = portal_toml(true);
    let name = me();
    assert!(lines(&factor(&config, &["list"])).is_empty());
    let begun = factor(&config, &["enrol", &name]);
    assert!(
        begun.status.success(),
        "{}",
        String::from_utf8_lossy(&begun.stderr)
    );
    let answer = &lines(&begun)[0]["enrol"];
    assert_eq!(answer["user"], name.as_str());
    assert!(
        answer["uri"]
            .as_str()
            .unwrap()
            .starts_with("otpauth://totp/")
    );
    assert_eq!(answer["recovery_codes"].as_array().unwrap().len(), 10);
    // Nothing is a factor yet, and the listing never shows a secret.
    let listed = factor(&config, &["list"]);
    let entry = &lines(&listed)[0];
    assert_eq!(
        (entry["enrolled"].as_bool(), entry["waiting"].as_bool()),
        (Some(false), Some(true))
    );
    assert!(!String::from_utf8_lossy(&listed.stdout).contains("secret"));
    // A wrong first code confirms nothing, and says nothing of the secret.
    let wrong = factor(&config, &["enrol", &name, "--code", "000000"]);
    assert_eq!(wrong.status.code(), Some(1));
    let said = String::from_utf8_lossy(&wrong.stderr).into_owned();
    assert!(said.contains("not accepted"), "{said}");
    assert!(!said.contains("000000") && !said.contains("secret="));
    // A code or an id that starts with a hyphen is a value, not an option.
    let dashed = factor(&config, &["enrol", &name, "--code", "-123456"]);
    let said = String::from_utf8_lossy(&dashed.stderr).into_owned();
    assert_eq!(dashed.status.code(), Some(1), "{said}");
    assert!(said.contains("not accepted"), "{said}");
    let dashed = factor(&config, &["reset", "--id", "-junk"]);
    let said = String::from_utf8_lossy(&dashed.stderr).into_owned();
    assert!(!said.contains("unexpected argument"), "{said}");
    // A name the portal does not sign in is a mistake in the invocation.
    assert_eq!(
        factor(&config, &["enrol", "nobodyatall"]).status.code(),
        Some(2)
    );
    assert_eq!(factor(&config, &["reset", "../x"]).status.code(), Some(2));
    let uri = answer["uri"].as_str().unwrap().to_owned();
    enrolled_with(&config, &name, &uri);
    let reset = factor(&config, &["reset", &name]);
    assert!(reset.status.success());
    assert_eq!(lines(&reset)[0]["reset"]["account"], name.as_str());
    // Nothing of the code is left, and the account is marked for an administrator.
    let after = lines(&factor(&config, &["list"]));
    assert_eq!(after.len(), 1);
    assert_eq!(
        (
            after[0]["reset"].as_bool(),
            after[0]["enrolled"].as_bool(),
            after[0]["waiting"].as_bool()
        ),
        (Some(true), Some(false), Some(false))
    );
}

#[test]
fn a_portal_that_keeps_no_codes_says_so() {
    let (_dir, config) = portal_toml(false);
    for args in [&["list"][..], &["enrol", "someone"], &["reset", "someone"]] {
        let answer = factor(&config, args);
        assert_eq!(answer.status.code(), Some(2), "{args:?}");
        assert!(String::from_utf8_lossy(&answer.stderr).contains("one-time codes"));
    }
}

#[test]
fn a_reset_marks_the_account_and_one_by_id_works_for_an_account_that_is_gone() {
    let (_dir, config) = portal_toml(true);
    let name = me();
    enrolled(&config, &name);
    let listed = lines(&factor(&config, &["list"]));
    let id = listed[0]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        id,
        format!("{}:{name}", my_uid()),
        "a record is the user id and the name"
    );
    let reset = factor(&config, &["reset", "--id", &id]);
    assert!(
        reset.status.success(),
        "{}",
        String::from_utf8_lossy(&reset.stderr)
    );
    let after = &lines(&factor(&config, &["list"]))[0];
    assert_eq!(
        (after["reset"].as_bool(), after["enrolled"].as_bool()),
        (Some(true), Some(false))
    );
    // Not an id, and not both a name and an id.
    assert_eq!(
        factor(&config, &["reset", "--id", "junk"]).status.code(),
        Some(2)
    );
    assert_eq!(
        factor(&config, &["reset", &name, "--id", &id])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(factor(&config, &["reset"]).status.code(), Some(2));
}

#[test]
fn the_command_refuses_to_run_as_an_account_that_does_not_own_the_code_files() {
    // `/tmp` belongs to root: whatever this command made there would not be the portal's account's, and the files would be
    // unreadable to it or to this account.
    let name = format!("canoa-factor-owner-{}.toml", std::process::id());
    let dir = std::env::temp_dir().join(format!("canoa-factor-owner-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let folder = Folder(dir.clone());
    let config = dir.join("portal.toml");
    std::fs::write(
        &config,
        format!(
            "listen = \"127.0.0.1:0\"\nsite = \"{}\"\nlibraries_any_owner = true\nmin_uid = {}\nauthenticators = [{{ provider = \"coqui_access_system\" }}, {{ provider = \"coqui_access_totp\", library = \"{}\", config = {{ source = {{ file = {{ path = \"/tmp/{name}\", format = \"toml\" }} }} }} }}]\n",
            dir.display(),
            my_uid().max(1),
            plugin().display()
        ),
    )
    .unwrap();
    for args in [&["list"][..], &["enrol", "someone"], &["reset", "someone"]] {
        let answer = factor(&config, args);
        assert_eq!(answer.status.code(), Some(2), "{args:?}");
        let said = String::from_utf8_lossy(&answer.stderr).into_owned();
        assert!(said.contains("belongs to user id 0"), "{said}");
    }
    let made: Vec<_> = std::fs::read_dir("/tmp")
        .unwrap()
        .filter_map(Result::ok)
        .filter(|file| file.file_name().to_string_lossy().starts_with(&name))
        .collect();
    assert!(
        made.is_empty(),
        "nothing was made in a folder that is not ours"
    );
    drop(folder);
}

#[test]
fn an_administrator_unlocks_an_account_and_the_time_is_kept_where_a_running_portal_reads_it() {
    let (dir, config) = portal_toml(true);
    let name = me();
    // An account with no code has nothing to clear.
    let none = factor(&config, &["unlock", &name]);
    assert!(
        none.status.success(),
        "{}",
        String::from_utf8_lossy(&none.stderr)
    );
    assert!(
        !std::fs::read_to_string(dir.0.join("totp.toml"))
            .unwrap_or_default()
            .contains("unlocked_unix"),
    );
    enrolled(&config, &name);
    let unlocked = factor(&config, &["unlock", &name]);
    assert!(
        unlocked.status.success(),
        "{}",
        String::from_utf8_lossy(&unlocked.stderr)
    );
    assert_eq!(lines(&unlocked)[0]["unlock"]["user"], name.as_str());
    // The provider keeps the time in its own table, which nobody else may read, and no file is made beside it for the purpose.
    let table = dir.0.join("totp.toml");
    assert!(
        std::fs::read_to_string(&table)
            .unwrap()
            .contains("unlocked_unix")
    );
    let mode =
        std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&table).unwrap().permissions());
    assert_eq!(mode & 0o077, 0);
    assert!(!dir.0.join("totp.toml.unlock").exists());
    assert!(!dir.0.join("totp.toml.reset").exists());
    assert_eq!(
        factor(&config, &["unlock", "nobodyatall"]).status.code(),
        Some(2)
    );
}
