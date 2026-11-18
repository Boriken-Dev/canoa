//! What the add-in on the person's own computer needs of the host's web server, beyond its pages: a secret only this account
//! can read and the page that offers to start a host that is not running.
//!
//! Every route here answers only a request that is from this computer, names it by a loopback name, comes from a page of this very
//! origin and, with `[host.web] office_secret`, carries the home's secret. A program on this computer can send any header, so the
//! checks keep out pages of other origins and other computers; the secret is what keeps out another account on a shared one.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks, threads, files and the clock are std's"
)]

use alloc::string::String;
use std::io::{self, Write as _};
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard, PoisonError};

/// The header the page sends its secret in.
pub(crate) const KEY_HEADER: &str = "x-canoa-office-key";
/// Where the page that is shown while no host runs asks for one to be started.
pub(crate) const LAUNCH_PATH: &str = "/office/launch";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The secret of this home's add-in: 32 random bytes in hexadecimal, in a file in the home, which the manifest rendered for this
/// computer carries and only this account can read.
pub(crate) struct Secret(String);

impl Secret {
    /// The secret in `dir`'s `secret` file, made when there is none (or the file holds something else).
    pub(crate) fn load(dir: &Path) -> io::Result<Self> {
        let file = dir.join("secret");
        let usable =
            |text: &str| text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit());
        match std::fs::read_to_string(&file) {
            Ok(text) if usable(text.trim()) => return Ok(Self(text.trim().to_ascii_lowercase())),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        std::fs::create_dir_all(dir)?;
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
        let text: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut written = options.open(&file)?;
        written.write_all(text.as_bytes())?;
        written.write_all(b"\n")?;
        Ok(Self(text))
    }

    /// Whether `given` is the secret, compared in time that does not say how much of it matched.
    pub(crate) fn permits(&self, given: Option<&str>) -> bool {
        let Some(given) = given else {
            return false;
        };
        let (own, other) = (self.0.as_bytes(), given.trim().as_bytes());
        let mut difference = u8::from(own.len() != other.len());
        for (index, byte) in own.iter().enumerate() {
            difference |= byte ^ other.get(index).copied().unwrap_or(0);
        }
        difference == 0
    }
}

/// What the page shown while no host runs asks for: a host started. The request is carried to whatever waits for it.
pub(crate) struct Launcher {
    asked: Mutex<mpsc::Sender<()>>,
}

impl Launcher {
    /// A launcher that tells `asked` each time the page asks.
    pub(crate) fn new(asked: mpsc::Sender<()>) -> Self {
        Self {
            asked: Mutex::new(asked),
        }
    }

    /// Whether the request could be passed on.
    pub(crate) fn ask(&self) -> bool {
        lock(&self.asked).send(()).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_secret_is_made_once_and_read_again_and_only_itself_is_permitted() {
        let dir = std::env::temp_dir().join(format!("canoa-office-secret-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = Secret::load(&dir).expect("made");
        assert_eq!(first.0.len(), 64);
        let again = Secret::load(&dir).expect("read");
        assert_eq!(first.0, again.0, "the second load reads the file");
        assert!(first.permits(Some(&first.0)));
        assert!(
            first.permits(Some(&format!("{}\n", first.0))),
            "a trailing newline is not a difference"
        );
        assert!(!first.permits(None));
        assert!(!first.permits(Some("")));
        assert!(!first.permits(Some(&first.0[..63])));
        assert!(!first.permits(Some(&format!("{}0", first.0))));
        let mut other = first.0.clone();
        other.replace_range(0..1, if first.0.starts_with('a') { "b" } else { "a" });
        assert!(!first.permits(Some(&other)));
        std::fs::write(dir.join("secret"), "not a key").expect("written");
        assert_eq!(
            Secret::load(&dir).expect("made again").0.len(),
            64,
            "a file holding something else is replaced"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
