//! Putting a process in a job object of its own that ends everything in it when its last handle closes (Windows).

use core::time::Duration;
use std::io;
use std::os::windows::io::AsRawHandle as _;
use std::thread;
use std::time::Instant;

use super::Child;

/// How long a process that refuses to join a job is given to be seen ending: it is refused while it exits, a moment
/// before it can be waited on.
const EXIT_GRACE: Duration = Duration::from_millis(250);

/// `ERROR_ACCESS_DENIED`.
const ERROR_ACCESS_DENIED: i32 = 5;

/// A job object that ends what is in it when its last handle closes.
///
/// # Errors
///
/// The system could not make it; the error carries its own text.
pub(crate) fn create() -> io::Result<win32job::Job> {
    let mut limits = win32job::ExtendedLimitInfo::new();
    limits.limit_kill_on_job_close();
    win32job::Job::create_with_limit_info(&limits)
        .map_err(|error| refusal("make a job object", error))
}

/// Puts `child` in a job object of its own. `None` when it had already ended: there is nothing left to contain, and its
/// status is still there to read.
///
/// # Errors
///
/// The system refused the job or the assignment; the error carries its own text.
pub(crate) fn contain(child: &mut Child) -> io::Result<Option<win32job::Job>> {
    let job = create()?;
    let handle = child.as_raw_handle() as isize;
    let contained = assign(&job, handle, || {
        child.try_wait().map(|status| status.is_some())
    })?;
    Ok(contained.then_some(job))
}

/// Puts the process `handle` names in `job`: `true` when it is in, `false` when it had ended, which the system refuses with
/// "access denied" and which `exited` confirms. Any other refusal is an error that says what the system said.
pub(crate) fn assign(
    job: &win32job::Job,
    handle: isize,
    mut exited: impl FnMut() -> io::Result<bool>,
) -> io::Result<bool> {
    let Err(error) = job.assign_process(handle) else {
        return Ok(true);
    };
    let error = system_error(error);
    if error.raw_os_error() == Some(ERROR_ACCESS_DENIED) {
        let deadline = Instant::now() + EXIT_GRACE;
        loop {
            if matches!(exited(), Ok(true)) {
                return Ok(false);
            }
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }
    Err(io::Error::new(
        error.kind(),
        format!("the process could not be put in its job object: {error}"),
    ))
}

/// A job object call's refusal as the system's own error: the crate hands back an HRESULT (`0x8007` and the code), which
/// names no text, so the code is taken out of it.
fn system_error(error: win32job::JobError) -> io::Error {
    let error = io::Error::from(error);
    match error.raw_os_error() {
        #[expect(
            clippy::cast_sign_loss,
            reason = "an HRESULT is read as its bit pattern"
        )]
        Some(code) if (code as u32) >> 16 == 0x8007 => io::Error::from_raw_os_error(code & 0xFFFF),
        _ => error,
    }
}

/// A job object call's refusal as an error that carries the system's own text, which the crate's own message leaves out.
fn refusal(what: &str, error: win32job::JobError) -> io::Error {
    let error = system_error(error);
    io::Error::new(error.kind(), format!("could not {what}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spawn::{Command, Stdio};

    fn quick(script: &str) -> Child {
        Command::new("cmd.exe")
            .args(["/d", "/c", script])
            .stdin(Stdio::Null)
            .stdout(Stdio::Null)
            .stderr(Stdio::Null)
            .spawn()
            .expect("cmd.exe starts")
    }

    #[test]
    fn a_process_that_ended_before_it_could_join_its_job_is_nothing_to_contain() {
        let mut child = quick("exit 3");
        let ended = child.wait().expect("it ends");
        let job = contain(&mut child).expect("an ended process is not an error");
        assert!(job.is_none());
        assert_eq!(ended.code(), Some(3), "its status is there to read");
    }

    #[test]
    fn a_running_process_joins_its_job_and_the_system_s_refusal_is_reported_as_it_said_it() {
        let mut child = quick("ping -n 30 127.0.0.1 >nul");
        let job = contain(&mut child).expect("it joins");
        assert!(job.is_some(), "it is contained");
        // A handle that names nothing: the system's refusal, with its code and text.
        let broken = create().expect("a job");
        let error = assign(&broken, 0, || Ok(false)).expect_err("no process has handle 0");
        let text = error.to_string();
        assert!(text.contains("job object"), "{text}");
        assert!(
            text.contains("os error"),
            "the system's own error is named: {text}"
        );
        drop(job);
        let _ = child.wait();
    }

    /// Programs that end the moment they start, started through the host's own spawner over and over while the machine is busy.
    #[test]
    fn a_program_that_ends_at_once_is_always_contained_or_gone() {
        let stop = alloc::sync::Arc::new(core::sync::atomic::AtomicBool::new(false));
        let burners: Vec<_> = (0..8)
            .map(|_| {
                let stop = alloc::sync::Arc::clone(&stop);
                thread::spawn(move || {
                    let mut n = 0_u64;
                    while !stop.load(core::sync::atomic::Ordering::Relaxed) {
                        n = core::hint::black_box(n.wrapping_mul(31).wrapping_add(7));
                    }
                })
            })
            .collect();
        let starters: Vec<_> = (0..4)
            .map(|_| {
                thread::spawn(|| {
                    for _ in 0..500 {
                        let mut child = quick("exit 0");
                        contain(&mut child)
                            .unwrap_or_else(|error| panic!("a quick program was refused: {error}"));
                        let _ = child.wait();
                    }
                })
            })
            .collect();
        let failures = starters
            .into_iter()
            .filter_map(|starter| starter.join().err())
            .count();
        stop.store(true, core::sync::atomic::Ordering::Relaxed);
        for burner in burners {
            let _ = burner.join();
        }
        assert_eq!(failures, 0, "2000 starts");
    }
}
