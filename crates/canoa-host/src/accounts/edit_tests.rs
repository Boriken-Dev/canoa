//! What the writers of canoa.cfg (`accounts add|remove`, an entry's offers, `pools/set`, `mail add|remove`) do to the person's own
//! text: one table changes, and the rest of the file stays as they wrote it.

use std::fs;
use std::path::PathBuf;

#[cfg(unix)]
use super::commands::append_table;
use super::commands::{remove_entry, remove_table, set_offered, set_pool, write_cfg};
use crate::config::PoolConfig;

fn dir(name: &str) -> PathBuf {
    static NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("canoa-edit-{name}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("directory");
    dir
}

#[cfg(unix)]
fn table(text: &str) -> toml::Table {
    toml::from_str(text).expect("a table")
}

const PERSON: &str = "\
# my canoa.cfg: edited by hand
[desktop]
start_hidden = true # on purpose

# the Claude entry, with a note on its header
[host.agents.Claude]   # claude code
harness = \"claude\"
title = \"Mine\"

[host.accounts.work] # the work login
kind = \"api\"
id = \"id-work\"

[host.accounts.spare]
kind = \"api\"
id = \"id-spare\"

[host.pools.team]
standard = \"anthropic-api\"
accounts = [\"work\"]

[usage]
warn_permille = 700
";

fn person(name: &str) -> (PathBuf, PathBuf) {
    let dir = dir(name);
    let path = dir.join("canoa.cfg");
    fs::write(&path, PERSON).expect("written");
    (dir, path)
}

fn only_changed(before: &str, after: &str, from: &str, to: &str) {
    assert_eq!(after, before.replacen(from, to, 1), "{after}");
}

/// A header with a comment after it, or a hand-made layout, is the table the writers mean: before, a table spelled that way was not
/// found, so a second was appended and the file refused.
#[test]
fn an_offer_edits_the_entry_whatever_its_header_looks_like() {
    let (dir, path) = person("offer");
    set_offered(&path, "Claude", "claude", &["work".to_owned()], &[]).expect("offered");
    let written = fs::read_to_string(&path).expect("read");
    assert_eq!(
        written.matches("[host.agents.Claude]").count(),
        1,
        "{written}"
    );
    only_changed(
        PERSON,
        &written,
        "title = \"Mine\"\n",
        "title = \"Mine\"\naccounts = [\"work\"]\n",
    );
    set_offered(&path, "Claude", "claude", &[], &[]).expect("withdrawn");
    assert_eq!(fs::read_to_string(&path).expect("read"), PERSON);
    let _ = fs::remove_dir_all(dir);
}

/// A pool changed is the table where the person put it, not one moved to the end of the file.
#[test]
fn a_pool_changed_stays_where_it_is_with_the_text_around_it() {
    let (dir, path) = person("pool");
    let pool = PoolConfig {
        standard: "anthropic-api".to_owned(),
        accounts: vec!["work".to_owned(), "spare".to_owned()],
    };
    set_pool(&path, "team", &pool).expect("set");
    let written = fs::read_to_string(&path).expect("read");
    only_changed(
        PERSON,
        &written,
        "accounts = [\"work\"]\n",
        "accounts = [\"work\", \"spare\"]\n",
    );
    // A new pool goes after the others and leaves everything the person wrote.
    set_pool(&path, "other", &pool).expect("made");
    let made = fs::read_to_string(&path).expect("read");
    assert!(made.starts_with(&written), "{made}");
    assert!(
        made.contains("[host.pools.other]\nstandard = \"anthropic-api\""),
        "{made}"
    );
    let _ = fs::remove_dir_all(dir);
}

/// A table taken out takes its blank line and nothing else, whatever the header carries.
#[test]
fn a_table_removed_leaves_the_rest_of_the_text_as_written() {
    let (dir, path) = person("remove");
    remove_table(&path, &["host", "accounts"], "spare", "canoa accounts add").expect("removed");
    let written = fs::read_to_string(&path).expect("read");
    assert_eq!(
        written,
        PERSON.replacen(
            "\n[host.accounts.spare]\nkind = \"api\"\nid = \"id-spare\"\n",
            "",
            1
        ),
        "{written}"
    );
    assert!(remove_table(&path, &["host", "accounts"], "spare", "canoa accounts add").is_err());
    // The header of the table in front of it carries a comment, and is found all the same.
    remove_table(&path, &["host", "pools"], "team", "pools/set").expect("a pool");
    assert!(!fs::read_to_string(&path).expect("read").contains("team"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn an_entry_with_tables_of_its_own_is_not_dropped_and_a_plain_one_is() {
    let dir = dir("entry");
    let path = dir.join("canoa.cfg");
    fs::write(
        &path,
        "[host.agents.A] # plain\nharness = \"claude\"\n\n[host.agents.B]\nharness = \"py_harness\"\nsource = \"s.dll\"\n\n[host.agents.B.source_settings]\nx = 1\n",
    )
    .expect("written");
    assert!(
        remove_entry(&path, "B").is_err(),
        "it holds a table of its own"
    );
    remove_entry(&path, "A").expect("a plain entry goes");
    let written = fs::read_to_string(&path).expect("read");
    assert!(
        !written.contains("[host.agents.A]") && written.contains("[host.agents.B.source_settings]"),
        "{written}"
    );
    let _ = fs::remove_dir_all(dir);
}

/// canoa.cfg written through the host's one writer is whole and leaves nothing beside it.
#[test]
fn a_canoa_cfg_is_written_whole_with_nothing_left_beside_it() {
    let dir = dir("whole");
    let path = dir.join("canoa.cfg");
    fs::write(&path, "[desktop]\nstart_hidden = true\n").expect("written");
    write_cfg(&path, "[desktop]\nstart_hidden = false\n").expect("written");
    assert_eq!(
        fs::read_to_string(&path).expect("read"),
        "[desktop]\nstart_hidden = false\n"
    );
    assert!(
        write_cfg(&path, "[desktop\n").is_err(),
        "a file the host would refuse is not written"
    );
    assert_eq!(
        fs::read_to_string(&path).expect("read"),
        "[desktop]\nstart_hidden = false\n"
    );
    let names: Vec<_> = fs::read_dir(&dir)
        .expect("dir")
        .flatten()
        .map(|entry| entry.file_name())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    let _ = fs::remove_dir_all(dir);
}

/// `accounts add` on a canoa.cfg that is a link writes through it: the link stays and its target changes.
#[cfg(unix)]
#[test]
fn a_canoa_cfg_that_is_a_link_is_written_through_by_the_accounts_commands() {
    let dir = dir("link");
    let target = dir.join("real.cfg");
    fs::write(&target, "[desktop]\nstart_hidden = true\n").expect("written");
    let link = dir.join("canoa.cfg");
    std::os::unix::fs::symlink(&target, &link).expect("a link");
    append_table(
        &link,
        &["host", "accounts"],
        "work",
        &table("kind = \"api\"\nid = \"i\"\n"),
    )
    .expect("added");
    assert!(
        fs::symlink_metadata(&link)
            .expect("meta")
            .file_type()
            .is_symlink()
    );
    assert!(
        fs::read_to_string(&target)
            .expect("read")
            .contains("[host.accounts.work]")
    );
    let _ = fs::remove_dir_all(dir);
}
