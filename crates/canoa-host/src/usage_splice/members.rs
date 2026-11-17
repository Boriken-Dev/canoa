//! The processes that belong to an agent: its own and everything it started, found from a snapshot of the machine's
//! process table. Antigravity may sit below a launcher (`canoa launch` with a profile), so its listening port is
//! owned by a grandchild of the process the host started.
//!
//! The table is read in this process by Windows' Toolhelp snapshot and on Linux from `/proc`, and by `ps` elsewhere, which
//! runs on a worker thread bounded in time and size. A snapshot that cannot be had leaves the agent's own process alone, which is the answer for an
//! agent that is not below anything.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
#[cfg(not(any(windows, target_os = "linux")))]
use core::time::Duration;
#[cfg(not(any(windows, target_os = "linux")))]
use std::io::Read as _;
#[cfg(not(any(windows, target_os = "linux")))]
use std::thread;
#[cfg(not(any(windows, target_os = "linux")))]
use std::time::Instant;

#[cfg(not(any(windows, target_os = "linux")))]
use crate::spawn::{Command, Stdio};

/// The longest `ps` may run.
#[cfg(not(any(windows, target_os = "linux")))]
const LIMIT: Duration = Duration::from_secs(5);
/// The most of its output that is read.
#[cfg(not(any(windows, target_os = "linux")))]
const MAX_LISTING: u64 = 4 * 1024 * 1024;

/// `root` and the processes below it, by parent, plus on Unix the ones in its process group (the agent leads a group of
/// its own). Always holds `root`.
pub(crate) fn members(root: u32) -> Vec<u32> {
    snapshot().map_or_else(|| alloc::vec![root], |listing| tree(&listing, root))
}

/// The members among the rows of `listing`, each `PID PPID PGID CREATED` with `-` (or nothing) for a column the system does
/// not give; a row that is anything else is skipped. A row's parent id may name a process that has since ended and been
/// given to another, so a child counts only if it was created at or after its parent, which the root's descendants all
/// are; an unrelated process that took the id of a dead parent is older than the parent that now holds it, or younger only
/// by being unrelated, and a table without creation times takes parents at their word.
fn tree(listing: &str, root: u32) -> Vec<u32> {
    let rows: Vec<Row> = listing
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let parent = fields.next()?.parse().ok()?;
            let group = fields.next().and_then(|field| field.parse().ok());
            let created = fields.next().and_then(|field| field.parse().ok());
            Some(Row {
                pid,
                parent,
                group,
                created,
            })
        })
        .collect();
    let mut found: BTreeMap<u32, Option<u64>> = BTreeMap::new();
    found.insert(
        root,
        rows.iter()
            .find(|row| row.pid == root)
            .and_then(|row| row.created),
    );
    loop {
        let before = found.len();
        for row in &rows {
            if found.contains_key(&row.pid) {
                continue;
            }
            let younger = |parent: Option<u64>| match (row.created, parent) {
                (Some(own), Some(parent)) => own >= parent,
                _ => true,
            };
            let below = found
                .get(&row.parent)
                .is_some_and(|parent| younger(*parent));
            let grouped = row.group == Some(root) && younger(found[&root]);
            if below || grouped {
                found.insert(row.pid, row.created);
            }
        }
        if found.len() == before {
            break;
        }
    }
    found.into_keys().collect()
}

/// One row of the process table.
struct Row {
    pid: u32,
    parent: u32,
    group: Option<u32>,
    created: Option<u64>,
}

#[cfg(windows)]
#[allow(
    unsafe_code,
    reason = "the system's process table is read through its own calls"
)]
mod win32;

#[cfg(windows)]
fn snapshot() -> Option<String> {
    win32::listing()
}

/// Every process's `PID PPID PGID START` from `/proc`, where START is the start time in clock ticks since boot. A process
/// that ends while the table is read is left out.
#[cfg(target_os = "linux")]
fn snapshot() -> Option<String> {
    use core::fmt::Write as _;
    let mut listing = String::new();
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        // The command name is in parentheses and may hold spaces and parentheses: the fields follow the last one.
        let Some((_, rest)) = stat.rsplit_once(')') else {
            continue;
        };
        let fields: Vec<&str> = rest.split_whitespace().collect();
        if let (Some(parent), Some(group), Some(start)) =
            (fields.get(1), fields.get(2), fields.get(19))
        {
            let _ = writeln!(listing, "{pid} {parent} {group} {start}");
        }
    }
    Some(listing)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn snapshot() -> Option<String> {
    run(Command::new("/bin/ps").args(["-A", "-o", "pid=,ppid=,pgid="]))
}

/// What `command` writes to its standard output, if it ended well within [`LIMIT`].
#[cfg(not(any(windows, target_os = "linux")))]
fn run(command: &mut Command) -> Option<String> {
    let mut child = command
        .stdin(Stdio::Null)
        .stdout(Stdio::Piped)
        .stderr(Stdio::Null)
        .spawn()
        .ok()?;
    let output = child.stdout.take()?;
    let reader = thread::spawn(move || {
        let mut text = Vec::new();
        let _ = output.take(MAX_LISTING).read_to_end(&mut text);
        text
    });
    let end = Instant::now() + LIMIT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(None) if Instant::now() < end => thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    String::from_utf8(reader.join().ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::time::Duration;

    use crate::spawn::{Command, Stdio};

    #[test]
    fn the_members_are_the_root_and_what_is_below_it() {
        let listing = "1 0\n10 1\n20 10\n30 20\n40 1\n50 40\n";
        assert_eq!(tree(listing, 10), [10, 20, 30]);
        assert_eq!(tree(listing, 40), [40, 50]);
        assert_eq!(tree(listing, 99), [99]);
    }

    #[test]
    fn a_process_group_member_is_one_whatever_its_parent() {
        let listing = "10 1 10\n20 1 10\n30 20 10\n40 1 40\n";
        assert_eq!(tree(listing, 10), [10, 20, 30]);
    }

    #[test]
    fn rows_that_are_not_numbers_are_skipped() {
        let listing = "PID PPID\n10 1\n\nnot a row\n20 10\n";
        assert_eq!(tree(listing, 10), [10, 20]);
    }

    #[test]
    fn an_orphan_of_a_dead_parent_whose_id_was_reused_is_not_a_member() {
        // The agent is 10, started at 500. 20 is its child (600) and 30 that child's (700). 60 was started at 100 by an
        // earlier process that had id 10 and ended; its parent id now names the agent. 70 is started later by 60.
        let listing = "\
10 1 - 500\n\
20 10 - 600\n\
30 20 - 700\n\
60 10 - 100\n\
70 60 - 800\n";
        assert_eq!(tree(listing, 10), [10, 20, 30]);
        // Without creation times a parent is taken at its word.
        assert_eq!(tree("10 1\n60 10\n", 10), [10, 60]);
    }

    #[test]
    fn the_process_table_shows_a_child_below_this_process() {
        let mut command = if cfg!(windows) {
            let mut command = Command::new("ping");
            command.args(["-n", "30", "127.0.0.1"]);
            command
        } else {
            let mut command = Command::new("sleep");
            command.arg("30");
            command
        };
        let mut child = command
            .stdout(Stdio::Null)
            .spawn()
            .expect("a child that waits");
        // A loaded machine may not show the child at the first read.
        let until = std::time::Instant::now() + Duration::from_secs(30);
        let found = loop {
            let found = members(std::process::id());
            if found.contains(&child.id()) || std::time::Instant::now() >= until {
                break found;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let _ = child.kill();
        let _ = child.wait();
        assert!(found.contains(&child.id()), "{found:?}");
    }

    #[test]
    fn a_grandchild_is_a_member_too() {
        let mut command = if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.args(["/C", "ping -n 30 127.0.0.1"]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 30; true"]);
            command
        };
        let mut child = command
            .stdout(Stdio::Null)
            .spawn()
            .expect("a child that starts one");
        let own = std::process::id();
        let until = std::time::Instant::now() + Duration::from_secs(10);
        // The grandchild appears once the shell has started it.
        let found = loop {
            let found = members(own);
            if found.len() >= 3 || std::time::Instant::now() >= until {
                break found;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let _ = child.kill();
        let _ = child.wait();
        assert!(
            found.contains(&own) && found.contains(&child.id()),
            "{found:?}"
        );
        assert!(found.len() >= 3, "{found:?}");
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn the_process_table_is_read_without_starting_a_program() {
        // A program started for the table costs hundreds of milliseconds; reading it in process costs a few.
        let started = std::time::Instant::now();
        for _ in 0..5 {
            assert!(members(std::process::id()).contains(&std::process::id()));
        }
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "five reads took {:?}",
            started.elapsed()
        );
    }
}
