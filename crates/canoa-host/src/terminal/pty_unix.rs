//! A pseudo-terminal and the shell on it, on Unix.
//!
//! This is `unsafe` for one call: the closure a child runs between `fork` and `exec`, which makes it the leader of a session of its own
//! and the terminal its controlling one, so that Ctrl-C reaches what the person is running and not the host.
#![allow(unsafe_code, reason = "the child's setup between fork and exec")]

use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read as _, Write as _};
use std::os::fd::{BorrowedFd, OwnedFd};
use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::process::{Pid, Signal, kill_process_group};
use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
use rustix::termios::{Winsize, tcsetwinsize};

use super::Size;

/// The shell's end of the terminal: what the person types goes in, and what the shell prints comes out.
pub(super) struct Pty {
    pub(super) input: Input,
    pub(super) output: Output,
    pub(super) process: Process,
}

/// What the person types, written when the shell takes it and never waited on once the terminal is over.
pub(super) struct Input {
    master: File,
    wake: OwnedFd,
}

/// What the shell prints, read when there is some and until the terminal is over.
pub(super) struct Output {
    master: File,
    wake: OwnedFd,
}

/// The shell and everything it started.
pub(super) struct Process {
    child: Mutex<Child>,
    /// The shell's process id, which is also its process group's and its session's.
    group: Option<Pid>,
    master: OwnedFd,
    /// Written to wake the reader once the terminal is over, so that it takes what is left and ends.
    wake: OwnedFd,
    ended: AtomicBool,
}

/// Starts `program` on a new terminal of `size`, in `cwd`.
pub(super) fn open(program: &OsStr, cwd: &Path, size: Size) -> io::Result<Pty> {
    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC)?;
    grantpt(&master)?;
    unlockpt(&master)?;
    let name = ptsname(&master, Vec::new())?;
    let slave: OwnedFd = rustix::fs::open(
        name.as_c_str(),
        rustix::fs::OFlags::RDWR | rustix::fs::OFlags::NOCTTY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?;
    set_size(&master, size)?;
    let (wake_read, wake_write) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)?;
    let mut command = Command::new(program);
    command
        .current_dir(cwd)
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave));
    // SAFETY: the closure makes only the two system calls below, which allocate nothing and take no lock, as a closure between
    // fork and exec must.
    unsafe {
        command.pre_exec(|| {
            rustix::process::setsid()?;
            // Standard input is the terminal's slave side by now.
            rustix::process::ioctl_tiocsctty(BorrowedFd::borrow_raw(0))?;
            Ok(())
        });
    }
    let child = command.spawn()?;
    let group = i32::try_from(child.id()).ok().and_then(Pid::from_raw);
    // Reads and writes wait in `poll`, on the terminal and on the signal that it is over, and never in the call itself.
    let flags = rustix::fs::fcntl_getfl(&master)?;
    rustix::fs::fcntl_setfl(&master, flags | rustix::fs::OFlags::NONBLOCK)?;
    let reader = File::from(master.try_clone()?);
    let writer = File::from(master.try_clone()?);
    Ok(Pty {
        input: Input {
            master: writer,
            wake: wake_read.try_clone()?,
        },
        output: Output {
            master: reader,
            wake: wake_read,
        },
        process: Process {
            child: Mutex::new(child),
            group,
            master,
            wake: wake_write,
            ended: AtomicBool::new(false),
        },
    })
}

fn set_size(master: &OwnedFd, size: Size) -> io::Result<()> {
    tcsetwinsize(
        master,
        Winsize {
            ws_row: size.rows,
            ws_col: size.cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        },
    )?;
    Ok(())
}

impl Output {
    /// Reads what is there into `buf`, waiting for it, and 0 once the terminal is over and nothing is left. The wait is the
    /// system's, on the terminal and on the signal that it is over; it is not repeated.
    pub(super) fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let over = {
                let mut fds = [
                    PollFd::new(&self.master, PollFlags::IN),
                    PollFd::new(&self.wake, PollFlags::IN),
                ];
                poll(&mut fds, None).map_err(io::Error::from)?;
                fds[1].revents().intersects(PollFlags::IN)
            };
            if over {
                // What the shell wrote before it ended is still the person's to see: take it without waiting for more.
                let mut fds = [PollFd::new(&self.master, PollFlags::IN)];
                let now = Timespec {
                    tv_sec: 0,
                    tv_nsec: 0,
                };
                if poll(&mut fds, Some(&now)).map_err(io::Error::from)? == 0 {
                    return Ok(0);
                }
            }
            match self.master.read(buf) {
                Ok(n) => return Ok(n),
                // The shell's side is closed: Linux reports it as an input error, other systems as the end of the file.
                Err(error)
                    if error.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error()) =>
                {
                    return Ok(0);
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                    ) => {}
                Err(error) => return Err(error),
            }
        }
    }
}

impl Input {
    /// Writes all of `bytes`, waiting while the shell is not reading, and gives up once the terminal is over.
    pub(super) fn write_all(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            match self.master.write(bytes) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(written) => bytes = &bytes[written..],
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    let mut fds = [
                        PollFd::new(&self.master, PollFlags::OUT),
                        PollFd::new(&self.wake, PollFlags::IN),
                    ];
                    poll(&mut fds, None).map_err(io::Error::from)?;
                    if fds[1].revents().intersects(PollFlags::IN) {
                        return Err(io::Error::other("the terminal has ended"));
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

impl Process {
    /// Waits for the shell to end, ends what it left running, and tells the reader to finish. Its exit code, or 128 and the signal.
    pub(super) fn wait(&self) -> i32 {
        let status = self
            .child
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .wait();
        self.end();
        match status {
            Ok(status) => status
                .code()
                .or_else(|| status.signal().map(|signal| 128 + signal))
                .unwrap_or(-1),
            Err(_) => -1,
        }
    }

    /// Ends the shell and the process group it leads with no signal they can catch, and wakes the reader. Safe to call again.
    pub(super) fn end(&self) {
        if !self.ended.swap(true, Ordering::SeqCst)
            && let Some(group) = self.group
        {
            end_tree(group, &self.master);
        }
        // One byte, left unread, so that the reader sees it whenever it looks.
        let _ = rustix::io::write(&self.wake, &[1]);
    }

    /// Tells the shell the terminal is now `size`.
    pub(super) fn resize(&self, size: Size) -> io::Result<()> {
        set_size(&self.master, size)
    }
}

/// Ends the shell, the job the person is running in the foreground, and every process in the shell's session: a job the shell
/// put in the background has a process group of its own, which the shell's group does not reach. A process that made a session of
/// its own (a daemon) is not in it.
fn end_tree(group: Pid, master: &OwnedFd) {
    if let Ok(foreground) = rustix::termios::tcgetpgrp(master) {
        let _ = kill_process_group(foreground, Signal::KILL);
    }
    let _ = kill_process_group(group, Signal::KILL);
    #[cfg(target_os = "linux")]
    for _ in 0..2 {
        for member in session_members(group.as_raw_pid()) {
            if let Some(member) = Pid::from_raw(member) {
                let _ = rustix::process::kill_process(member, Signal::KILL);
            }
        }
    }
}

/// The processes whose session is `session`, from the process table.
#[cfg(target_os = "linux")]
fn session_members(session: i32) -> Vec<i32> {
    let Ok(table) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    table
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let pid: i32 = entry.file_name().to_str()?.parse().ok()?;
            let stat = std::fs::read_to_string(entry.path().join("stat")).ok()?;
            // `pid (name) state ppid pgrp session ...`: the name may hold spaces and parentheses, so count from its end.
            let rest = stat.get(stat.rfind(')')? + 1..)?;
            let found: i32 = rest.split_whitespace().nth(3)?.parse().ok()?;
            (found == session).then_some(pid)
        })
        .collect()
}

impl Drop for Process {
    fn drop(&mut self) {
        // A terminal dropped with its shell running ends it; a reaped shell leaves nothing to end.
        let ended = self.ended.load(Ordering::SeqCst);
        if !ended && let Some(group) = self.group {
            end_tree(group, &self.master);
        }
    }
}
