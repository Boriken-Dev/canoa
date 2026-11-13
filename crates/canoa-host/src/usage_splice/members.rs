//! The processes that belong to an agent: its own and everything it started, found from a snapshot of the machine's
//! process table. Antigravity may sit below a launcher (`canoa launch` with a profile), so its listening port is
//! owned by a grandchild of the process the host started.
//!
//! The host forbids `unsafe`, and the table is read by the system's own tool instead: PowerShell's CIM query on Windows,
//! `ps` elsewhere. Both run on a worker thread, bounded in time and size. A snapshot that cannot be had leaves the agent's
//! own process alone, which is the answer for an agent that is not below anything.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;
use std::io::Read as _;
use std::thread;
use std::time::Instant;

use crate::spawn::{Command, Stdio};

/// The longest the table's tool may run.
const LIMIT: Duration = Duration::from_secs(5);
/// The most of its output that is read.
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
fn snapshot() -> Option<String> {
    /// `CREATE_NO_WINDOW`: no console window flashes up for a program started from a GUI.
    const NO_WINDOW: u32 = 0x0800_0000;
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let shell = std::path::Path::new(&root)
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let mut command = Command::new(shell);
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-CimInstance -ClassName Win32_Process -Property ProcessId,ParentProcessId,CreationDate | \
             ForEach-Object { if ($_.CreationDate) { '{0} {1} - {2}' -f $_.ProcessId, $_.ParentProcessId, $_.CreationDate.ToFileTimeUtc() } }",
        ])
        .creation_flags(NO_WINDOW);
    run(&mut command)
}

#[cfg(not(windows))]
fn snapshot() -> Option<String> {
    run(Command::new("/bin/ps").args(["-A", "-o", "pid=,ppid=,pgid="]))
}

/// What `command` writes to its standard output, if it ended well within [`LIMIT`].
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
        let found = members(std::process::id());
        let _ = child.kill();
        let _ = child.wait();
        assert!(found.contains(&child.id()), "{found:?}");
    }
}
