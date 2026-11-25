//! The names of accounts and pools are keys of canoa.cfg: any text a person can read, written quoted where a bare key could not hold
//! it.

use std::fs;
use std::path::{Path, PathBuf};

use super::commands::{append_table, remove_table};
use crate::config::Config;

fn file(path: &std::path::Path) -> crate::config_store::FileStore {
    crate::config_store::FileStore::at(path)
}

fn dir(name: &str) -> PathBuf {
    static NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("canoa-names-{name}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("directory");
    dir
}

fn table(text: &str) -> toml::Table {
    toml::from_str(text).expect("a table")
}

/// A name is whatever the person writes: it is quoted where a bare key could not hold it, and found again by the same name.
#[test]
fn an_account_called_anything_a_person_can_read_is_written_found_and_removed() {
    let dir = dir("names");
    let path = dir.join("canoa.cfg");
    fs::write(&path, "# mine\n[desktop]\nstart_hidden = true\n").expect("written");
    let names = [
        "work",
        "my work",
        "work.2",
        "café",
        "日本語",
        "a \"quoted\" one",
        "a:b",
    ];
    for name in names {
        append_table(
            &file(&path),
            &["host", "accounts"],
            name,
            &table(&format!("kind = \"api\"\nid = \"id-{}\"\n", name.len())),
        )
        .expect(name);
    }
    let written = fs::read_to_string(&path).expect("read");
    assert!(
        written.starts_with("# mine\n[desktop]\nstart_hidden = true\n"),
        "{written}"
    );
    let config = Config::from_str_named(&written, "canoa.cfg", Path::new(".")).expect("reads");
    for name in names {
        assert!(config.accounts.contains_key(name), "{name}: {written}");
    }
    assert!(
        append_table(
            &file(&path),
            &["host", "accounts"],
            "café",
            &table("harness = \"claude\"\n")
        )
        .is_err(),
        "a name taken is not written over"
    );
    for name in names {
        remove_table(
            &file(&path),
            &["host", "accounts"],
            name,
            "canoa accounts add",
        )
        .expect(name);
    }
    let after = fs::read_to_string(&path).expect("read");
    assert_eq!(
        after.trim_end(),
        "# mine\n[desktop]\nstart_hidden = true",
        "{after}"
    );
    let _ = fs::remove_dir_all(dir);
}

/// A name for a pool or an account may not hold a control character, which no one can read; the names that stay addresses stay
/// as they were.
#[test]
fn names_that_are_keys_take_any_text_and_names_that_are_addresses_do_not() {
    for name in ["work", "my work", "café", "a.b"] {
        let text = format!(
            "[host.accounts.\"{name}\"]\nkind = \"api\"\nid = \"i\"\n\n[host.pools.\"pool {name}\"]\nstandard = \"anthropic-api\"\naccounts = [\"{name}\"]\n"
        );
        Config::from_str_named(&text, "canoa.cfg", Path::new(".")).expect(name);
    }
    let control = "[host.accounts.\"a\\u0001b\"]\nkind = \"api\"\nid = \"i\"\n";
    assert!(Config::from_str_named(control, "canoa.cfg", Path::new(".")).is_err());
    let entry = "[host.agents.\"my agent\"]\nharness = \"claude\"\n";
    assert!(Config::from_str_named(entry, "canoa.cfg", Path::new(".")).is_err());
}
