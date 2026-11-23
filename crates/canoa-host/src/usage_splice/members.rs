//! The processes that belong to an agent: its own and everything it started, found from a snapshot of the machine's
//! process table. An agent may sit below a launcher (`canoa launch` with a profile), so what a usage reader looks for (a listening
//! port) may be owned by a grandchild of the process the host started.
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

/// What an agent's processes use together, summed over [`members`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TreeUse {
    /// How many processes there are, the root included.
    pub(crate) processes: usize,
    /// The CPU time they have used, user and system, in milliseconds; `None` when none of them could be read.
    pub(crate) cpu_ms: Option<u64>,
    /// Their resident memory (the working set on Windows), in bytes; `None` when none of them could be read.
    pub(crate) memory_bytes: Option<u64>,
}

/// What `root` and the processes below it use now. A process that ends or cannot be opened while it is read adds nothing.
pub(crate) fn tree_use(root: u32) -> TreeUse {
    let pids = members(root);
    let (mut cpu, mut memory) = (None::<u64>, None::<u64>);
    for (cpu_ms, memory_bytes) in usage_of(&pids) {
        cpu = Some(cpu.unwrap_or(0).saturating_add(cpu_ms));
        memory = Some(memory.unwrap_or(0).saturating_add(memory_bytes));
    }
    TreeUse {
        processes: pids.len(),
        cpu_ms: cpu,
        memory_bytes: memory,
    }
}

#[cfg(windows)]
fn usage_of(pids: &[u32]) -> Vec<(u64, u64)> {
    pids.iter().filter_map(|&pid| win32::usage(pid)).collect()
}

/// CPU time from `/proc/PID/stat` (fields 14 and 15, in clock ticks) and resident memory from its field 24 (in pages).
#[cfg(target_os = "linux")]
fn usage_of(pids: &[u32]) -> Vec<(u64, u64)> {
    let ticks = rustix::param::clock_ticks_per_second().max(1);
    let page = u64::try_from(rustix::param::page_size()).unwrap_or(4096);
    pids.iter()
        .filter_map(|pid| {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            // The command name is in parentheses and may hold spaces and parentheses: the fields follow the last one.
            let (_, rest) = stat.rsplit_once(')')?;
            let fields: Vec<&str> = rest.split_whitespace().collect();
            let user: u64 = fields.get(11)?.parse().ok()?;
            let system: u64 = fields.get(12)?.parse().ok()?;
            let resident: u64 = fields.get(21)?.parse().ok()?;
            Some((
                user.saturating_add(system).saturating_mul(1000) / ticks,
                resident.saturating_mul(page),
            ))
        })
        .collect()
}

/// `ps`'s `time` (`[[DD-]HH:]MM:SS[.FF]`) and `rss` (KiB) of each process.
#[cfg(not(any(windows, target_os = "linux")))]
fn usage_of(pids: &[u32]) -> Vec<(u64, u64)> {
    let list: Vec<String> = pids.iter().map(u32::to_string).collect();
    run(Command::new("/bin/ps").args(["-o", "time=,rss=", "-p", &list.join(",")]))
        .map(|listing| ps_usage(&listing))
        .unwrap_or_default()
}

/// The rows of `ps -o time=,rss=`, each `TIME RSS`; a row that is anything else is skipped.
#[cfg(any(test, not(any(windows, target_os = "linux"))))]
fn ps_usage(listing: &str) -> Vec<(u64, u64)> {
    listing
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let time = fields.next()?;
            let kib: u64 = fields.next()?.parse().ok()?;
            Some((ps_time_ms(time)?, kib.saturating_mul(1024)))
        })
        .collect()
}

/// `[[DD-]HH:]MM:SS[.FF]` in milliseconds.
#[cfg(any(test, not(any(windows, target_os = "linux"))))]
fn ps_time_ms(time: &str) -> Option<u64> {
    let (days, clock) = match time.split_once('-') {
        Some((days, clock)) => (days.parse::<u64>().ok()?, clock),
        None => (0, time),
    };
    let (whole, fraction) = clock.split_once('.').unwrap_or((clock, ""));
    let mut seconds = 0_u64;
    for part in whole.split(':') {
        seconds = seconds.checked_mul(60)?.checked_add(part.parse().ok()?)?;
    }
    let fraction_ms = match fraction.len() {
        0 => 0,
        1 => fraction.parse::<u64>().ok()? * 100,
        2 => fraction.parse::<u64>().ok()? * 10,
        _ => fraction.get(..3)?.parse().ok()?,
    };
    let seconds = seconds.checked_add(days.checked_mul(86_400)?)?;
    seconds.checked_mul(1000)?.checked_add(fraction_ms)
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
    fn ps_times_and_sizes_are_read_in_every_form_it_writes() {
        assert_eq!(ps_time_ms("00:05"), Some(5_000));
        assert_eq!(ps_time_ms("1:02:03"), Some(3_723_000));
        assert_eq!(ps_time_ms("2-00:00:01"), Some(172_801_000));
        assert_eq!(ps_time_ms("0:01.25"), Some(1_250));
        assert_eq!(ps_time_ms("0:01.5"), Some(1_500));
        assert_eq!(ps_time_ms("x:01"), None);
        assert_eq!(
            ps_usage("  0:01.25  2048\nnot a row\n 01:00 10\n"),
            [(1_250, 2 * 1024 * 1024), (60_000, 10 * 1024)]
        );
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn this_process_uses_memory_and_time_that_are_read() {
        let used = tree_use(std::process::id());
        assert!(used.processes >= 1, "{used:?}");
        assert!(used.memory_bytes.is_some_and(|bytes| bytes > 0), "{used:?}");
        assert!(used.cpu_ms.is_some(), "{used:?}");
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
