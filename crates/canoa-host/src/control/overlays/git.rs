//! Git subprocess isolation, fetching, tree listing and blob reading for the overlay store.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, processes and clocks are std's"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use canoa::network::{GitTls, Network};
use coqui_aos::{EXCLUDED_COMPONENTS, TreeEntry, digest_entries};

use crate::profile::full_commit_id;
use crate::spawn::{Command, Stdio};

use super::updates::Follows;
use super::{Listing, OverlayLimits, Overlays, Trust, refused, unique};

/// The git settings every isolated run is given: no credential helper, no redirect followed, every object received checked.
const ISOLATED_SETTINGS: [&str; 3] = [
    "credential.helper=",
    "http.followRedirects=false",
    "transfer.fsckObjects=true",
];

/// The transports an isolated run may use.
const ISOLATED_PROTOCOLS: [&str; 5] = ["https", "http", "git", "ssh", "file"];

/// What an isolated run's environment keeps of the person's: where git finds its programs, and the proxy the network goes through.
const KEPT_VARIABLES: [&str; 15] = [
    "PATH",
    "SystemRoot",
    "windir",
    "TEMP",
    "TMP",
    "TMPDIR",
    "LANG",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "all_proxy",
];

/// The most entries a source's tree may list, excluded ones included.
const MAX_ENTRIES: usize = 16384;
/// The most bytes one fetch may write into the cache before it is ended.
const MAX_FETCH_BYTES: u64 = 256 * 1024 * 1024;
/// How long one git command may run before it is ended.
const GIT_TIME: Duration = Duration::from_secs(5 * 60);
/// The overlay caches, and the catalogs, this host keeps at once.
const MAX_CACHES: usize = 64;
/// What the overlays folder may hold in all, installed overlays and fetched history together.
const MAX_FOLDER_BYTES: u64 = 1024 * 1024 * 1024;

/// The ssh command of an isolated run: none of the person's configuration, keys or agent. OpenSSH takes its home from the
/// password database, not from `HOME`, so its configuration and identity files are replaced by `empty`, a file with nothing in
/// it, and only that identity may be tried.
fn isolated_ssh(program: &str, empty: &Path) -> String {
    let empty = empty.display().to_string().replace('\\', "/");
    format!(
        "{program} -F \"{empty}\" -o BatchMode=yes -o IdentitiesOnly=yes -o IdentityAgent=none -o IdentityFile=\"{empty}\" \
         -o PasswordAuthentication=no -o KbdInteractiveAuthentication=no"
    )
}

impl Overlays {
    /// The program an isolated run starts for ssh.
    fn ssh_program(&self) -> String {
        #[cfg(test)]
        if let Some((_, value)) = self.person.iter().find(|(key, _)| key == "CANOA_TEST_SSH") {
            return value.to_string_lossy().into_owned();
        }
        "ssh".to_owned()
    }

    /// A variable of the environment git runs inherit.
    fn inherited(&self, name: &str) -> Option<OsString> {
        #[cfg(test)]
        if let Some((_, value)) = self.person.iter().find(|(key, _)| key == name) {
            return (!value.is_empty()).then(|| value.clone());
        }
        std::env::var_os(name)
    }

    /// The commit `reference` names at `repo` now, from one `ls-remote` and no fetch; a whole commit id is itself.
    pub(crate) fn remote_revision(
        &self,
        repo: &str,
        reference: &str,
    ) -> Result<String, (i32, String)> {
        self.remote_ref(Trust::Own, repo, reference)
            .map(|(_, commit)| commit)
    }

    /// What `reference` is at `repo` now and the commit it names, from one `ls-remote` run under `trust` and no fetch; a whole
    /// commit id is itself and names no listing.
    pub(super) fn remote_ref(
        &self,
        trust: Trust,
        repo: &str,
        reference: &str,
    ) -> Result<(Follows, String), (i32, String)> {
        if full_commit_id(reference) {
            return Ok((Follows::Commit, reference.to_owned()));
        }
        let listed = self.run_as(
            trust,
            &[
                "ls-remote".into(),
                "--heads".into(),
                "--tags".into(),
                "--".into(),
                repo.into(),
                format!("refs/heads/{reference}").into(),
                format!("refs/tags/{reference}").into(),
                format!("refs/tags/{reference}^{{}}").into(),
            ],
            None,
            None,
            1024 * 1024,
        )?;
        let text = String::from_utf8_lossy(&listed).into_owned();
        // A branch wins over a tag of the name, as the fetch decides; an annotated tag lists its commit on the peeled line.
        let find = |name: &str| {
            text.lines()
                .filter_map(|line| line.split_once('\t'))
                .find(|(_, listed)| *listed == name)
                .map(|(id, _)| id.to_owned())
        };
        find(&format!("refs/heads/{reference}"))
            .map(|id| (Follows::Branch, id))
            .or_else(|| {
                find(&format!("refs/tags/{reference}^{{}}"))
                    .or_else(|| find(&format!("refs/tags/{reference}")))
                    .map(|id| (Follows::Tag, id))
            })
            .ok_or_else(|| refused("The source has no branch or tag of that name"))
    }

    fn git(&self) -> Result<&Path, (i32, String)> {
        self.git
            .get_or_init(find_git)
            .as_ref()
            .map(PathBuf::as_path)
            .map_err(|why| refused(why.clone()))
    }

    pub(super) fn init_cache(&self, cache: &Path) -> Result<(), (i32, String)> {
        if cache.exists() {
            return Ok(());
        }
        let kept = cache
            .parent()
            .and_then(|dir| fs::read_dir(dir).ok())
            .map_or(0, |reader| reader.count());
        if kept >= MAX_CACHES || folder_size(self.root()?) > MAX_FOLDER_BYTES {
            return Err(refused(
                "this host holds as many fetched sources as it keeps: remove some",
            ));
        }
        fs::create_dir_all(cache)
            .map_err(|error| refused(format!("the cache could not be made: {error}")))?;
        self.run(
            &[
                "init".into(),
                "--quiet".into(),
                "--bare".into(),
                "--template=".into(),
                cache.into(),
            ],
            None,
            None,
            1024 * 1024,
        )
        .map(drop)
    }

    /// Fetches only what `reference` names, one commit deep, and returns its commit. A whole commit id is that object and
    /// nothing else: a branch or tag carrying the same name never stands in for it.
    pub(super) fn fetch(
        &self,
        trust: Trust,
        cache: &Path,
        repo: &str,
        reference: &str,
    ) -> Result<String, (i32, String)> {
        let commit = self.fetch_into(trust, cache, repo, reference)?;
        // What the fetch wrote counts against the folder's cap at once: past it the history goes and nothing more is fetched.
        if folder_size(self.root()?) > MAX_FOLDER_BYTES {
            let _ = fs::remove_dir_all(cache);
            return Err(refused(
                "the fetch took the overlays folder past what this host keeps: remove some",
            ));
        }
        Ok(commit)
    }

    fn fetch_into(
        &self,
        trust: Trust,
        cache: &Path,
        repo: &str,
        reference: &str,
    ) -> Result<String, (i32, String)> {
        if folder_size(self.root()?) > MAX_FOLDER_BYTES {
            return Err(refused(
                "the overlays folder holds as much as this host keeps: remove some",
            ));
        }
        let dir = format!("--git-dir={}", cache.display());
        let fetch = |wants: &[&str]| -> Result<Vec<u8>, (i32, String)> {
            let mut args: Vec<OsString> = vec![
                dir.clone().into(),
                "fetch".into(),
                "--quiet".into(),
                "--no-tags".into(),
                "--no-recurse-submodules".into(),
                "--depth=1".into(),
                "--".into(),
                repo.into(),
            ];
            args.extend(wants.iter().map(OsString::from));
            self.run_as(trust, &args, None, Some(cache), 1024 * 1024)
        };
        if full_commit_id(reference) {
            if fetch(&[reference]).is_err() {
                // A server that refuses a want by id still serves its branches and tags, one of which may hold the commit.
                fetch(&[
                    "+refs/heads/*:refs/canoa/heads/*",
                    "+refs/tags/*:refs/canoa/tags/*",
                ])?;
            }
            let found = self.run(
                &[
                    dir.clone().into(),
                    "rev-parse".into(),
                    "--verify".into(),
                    "--quiet".into(),
                    "--end-of-options".into(),
                    format!("{reference}^{{commit}}").into(),
                ],
                None,
                None,
                4096,
            );
            return match found {
                Ok(out) if String::from_utf8_lossy(&out).trim() == reference => {
                    Ok(reference.to_owned())
                }
                _ => Err(refused("The source has no commit of that id")),
            };
        }
        let heads = format!("refs/heads/{reference}");
        let tags = format!("refs/tags/{reference}");
        let listed = self.run_as(
            trust,
            &[
                format!("--git-dir={}", cache.display()).into(),
                "ls-remote".into(),
                "--heads".into(),
                "--tags".into(),
                "--".into(),
                repo.into(),
                heads.clone().into(),
                tags.clone().into(),
            ],
            None,
            None,
            1024 * 1024,
        )?;
        let text = String::from_utf8_lossy(&listed).into_owned();
        let names: BTreeSet<&str> = text
            .lines()
            .filter_map(|line| line.rsplit('\t').next())
            .collect();
        let remote = [heads, tags]
            .into_iter()
            .find(|name| names.contains(name.as_str()))
            .ok_or_else(|| refused("The source has no branch or tag of that name"))?;
        let local = format!("refs/canoa/{}", &remote["refs/".len()..]);
        fetch(&[&format!("+{remote}:{local}")])?;
        let found = self.run(
            &[
                dir.into(),
                "rev-parse".into(),
                "--verify".into(),
                "--end-of-options".into(),
                format!("{local}^{{commit}}").into(),
            ],
            None,
            None,
            4096,
        )?;
        Ok(String::from_utf8_lossy(&found).trim().to_owned())
    }

    /// Every entry under `path` at `commit`, read as git lists it and refused as soon as its files pass the limits, then
    /// digested as AOS defines: a link, a submodule, a name that is not a relative path or two names equal without ASCII case
    /// refuse the whole overlay. Entries under an excluded name are left out, never installed.
    pub(super) fn list(
        &self,
        cache: &Path,
        commit: &str,
        path: &str,
    ) -> Result<Listing, (i32, String)> {
        self.list_within(cache, commit, path, &self.limits)
    }

    /// [`Self::list`] under `limits` in place of this host's overlay limits.
    pub(super) fn list_within(
        &self,
        cache: &Path,
        commit: &str,
        path: &str,
        limits: &OverlayLimits,
    ) -> Result<Listing, (i32, String)> {
        let prefix: Vec<u8> = if path.is_empty() {
            Vec::new()
        } else {
            format!("{path}/").into_bytes()
        };
        let mut args: Vec<OsString> = vec![
            format!("--git-dir={}", cache.display()).into(),
            "ls-tree".into(),
            "-r".into(),
            "-l".into(),
            "-z".into(),
            "--full-tree".into(),
            commit.into(),
            "--".into(),
        ];
        if !path.is_empty() {
            args.push(path.into());
        }
        let out = self.run(&args, None, None, 64 * 1024 * 1024)?;
        struct Listed {
            components: Vec<Vec<u8>>,
            kind: Kind,
            blob: String,
        }
        #[derive(PartialEq, Eq, Clone, Copy)]
        enum Kind {
            File,
            Symlink,
            Special,
        }
        let excluded = |entry: &Listed| {
            entry.components.iter().any(|part| {
                EXCLUDED_COMPONENTS
                    .iter()
                    .any(|name| name.as_bytes() == part.as_slice())
            }) || (entry.kind == Kind::File
                && entry
                    .components
                    .last()
                    .is_some_and(|last| last.ends_with(b".pyc")))
        };
        let path_of = |entry: &Listed| {
            entry
                .components
                .iter()
                .map(|part| String::from_utf8_lossy(part).into_owned())
                .collect::<Vec<_>>()
                .join("/")
        };
        let mut entries: Vec<Listed> = Vec::new();
        let mut folded: BTreeMap<String, String> = BTreeMap::new();
        let (mut count, mut bytes) = (0_usize, 0_u64);
        for record in out
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty())
        {
            if entries.len() >= MAX_ENTRIES {
                return Err(refused(format!(
                    "The source holds more than {MAX_ENTRIES} entries"
                )));
            }
            let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
                continue;
            };
            let head = String::from_utf8_lossy(&record[..tab]).into_owned();
            let full = &record[tab + 1..];
            if full.len() <= prefix.len() || !full.starts_with(&prefix) {
                continue;
            }
            let relative = &full[prefix.len()..];
            let fields: Vec<&str> = head.split_whitespace().collect();
            let [mode, kind, blob, size] = fields[..] else {
                continue;
            };
            let kind = if mode == "120000" {
                Kind::Symlink
            } else if kind == "blob" {
                Kind::File
            } else {
                Kind::Special
            };
            let entry = Listed {
                components: relative
                    .split(|byte| *byte == b'/')
                    .map(<[u8]>::to_vec)
                    .collect(),
                kind,
                blob: blob.to_owned(),
            };
            let skip = excluded(&entry);
            if !skip && kind != Kind::File {
                return Err(refused(
                    "The source holds a link or a submodule, which no overlay has",
                ));
            }
            if !skip {
                // One directory on a file system that ignores case: two names it would merge refuse the overlay.
                let path = path_of(&entry);
                let names: Vec<&str> = path.split('/').collect();
                for end in 1..=names.len() {
                    let name = names[..end].join("/");
                    let first = folded
                        .entry(name.to_lowercase())
                        .or_insert_with(|| name.clone());
                    if *first != name {
                        return Err(refused(
                            "The source holds two names that differ only in case",
                        ));
                    }
                }
                count += 1;
                bytes += size.parse::<u64>().unwrap_or(0);
                if count > limits.files {
                    return Err(refused(format!(
                        "The overlay holds more than {} files",
                        limits.files
                    )));
                }
                if bytes > limits.bytes {
                    return Err(refused(format!(
                        "The overlay holds more than {} bytes",
                        limits.bytes
                    )));
                }
            }
            entries.push(entry);
        }
        let members: Vec<&Listed> = entries
            .iter()
            .filter(|entry| entry.kind == Kind::File && !excluded(entry))
            .collect();
        if members.is_empty() {
            return Err(refused("The source has no files at that path and commit"));
        }
        let ids: Vec<String> = unique(members.iter().map(|entry| &entry.blob));
        let blobs = self.blobs_within(cache, &ids, limits)?;
        let tree: Vec<(Vec<&[u8]>, TreeEntry)> = entries
            .iter()
            .map(|entry| {
                let components: Vec<&[u8]> = entry.components.iter().map(Vec::as_slice).collect();
                let tree = match entry.kind {
                    Kind::File if excluded(entry) => TreeEntry::File(Vec::new()),
                    Kind::File => {
                        TreeEntry::File(blobs.get(&entry.blob).cloned().unwrap_or_default())
                    }
                    Kind::Symlink => TreeEntry::Symlink,
                    Kind::Special => TreeEntry::Special,
                };
                (components, tree)
            })
            .collect();
        let digest = digest_entries(
            tree.iter()
                .map(|(components, entry)| (components.clone(), entry)),
        )
        .map_err(|error| refused(format!("The source cannot be an overlay ({})", error.class)))?;
        let files = members
            .iter()
            .map(|entry| (path_of(entry), entry.blob.clone()))
            .collect();
        let skipped = entries
            .iter()
            .filter(|entry| excluded(entry))
            .map(&path_of)
            .collect();
        Ok((files, skipped, digest.digest.to_string(), bytes))
    }

    /// The bytes of each blob in `ids`, read from git's object store in one `cat-file --batch`, as stored.
    pub(super) fn blobs(
        &self,
        cache: &Path,
        ids: &[String],
    ) -> Result<BTreeMap<String, Vec<u8>>, (i32, String)> {
        self.blobs_within(cache, ids, &self.limits)
    }

    /// [`Self::blobs`] with room for what `limits` allows an overlay to hold.
    pub(super) fn blobs_within(
        &self,
        cache: &Path,
        ids: &[String],
        limits: &OverlayLimits,
    ) -> Result<BTreeMap<String, Vec<u8>>, (i32, String)> {
        if ids.is_empty() {
            return Ok(BTreeMap::new());
        }
        if ids
            .iter()
            .any(|id| !id.bytes().all(|b| b.is_ascii_hexdigit()) || !matches!(id.len(), 40 | 64))
        {
            return Err(refused("git did not give an object id"));
        }
        let stdin: Vec<u8> = ids
            .iter()
            .flat_map(|id| format!("{id}\n").into_bytes())
            .collect();
        let ceiling =
            usize::try_from(limits.bytes.saturating_add(64 * 1024 * 1024)).unwrap_or(usize::MAX);
        let data = self.run(
            &[
                format!("--git-dir={}", cache.display()).into(),
                "cat-file".into(),
                "--batch".into(),
            ],
            Some(stdin),
            None,
            ceiling,
        )?;
        let mut out = BTreeMap::new();
        let mut at = 0;
        for id in ids {
            let end = data[at..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map(|offset| at + offset)
                .ok_or_else(|| refused("git cat-file answered short"))?;
            let header = String::from_utf8_lossy(&data[at..end]).into_owned();
            let fields: Vec<&str> = header.split(' ').collect();
            let [name, kind, size] = fields[..] else {
                return Err(refused("git cat-file did not give a blob it was asked for"));
            };
            if name != id || kind != "blob" {
                return Err(refused("git cat-file did not give a blob it was asked for"));
            }
            let size: usize = size
                .parse()
                .map_err(|_| refused("git cat-file answered badly"))?;
            at = end + 1;
            if at + size + 1 > data.len() {
                return Err(refused("git cat-file answered short"));
            }
            out.insert(id.clone(), data[at..at + size].to_vec());
            at += size + 1;
        }
        Ok(out)
    }

    /// `git args`, with only the transports a source may name, nothing asked of the person and nothing from the source's
    /// configuration, ended after [`GIT_TIME`]; with `guard`, also ended once that folder holds more than [`MAX_FETCH_BYTES`].
    /// What git prints is returned up to `ceiling` bytes, past which the run is ended; a failure says only that it failed.
    pub(super) fn run(
        &self,
        args: &[OsString],
        stdin: Option<Vec<u8>>,
        guard: Option<&Path>,
        ceiling: usize,
    ) -> Result<Vec<u8>, (i32, String)> {
        self.run_as(Trust::Own, args, stdin, guard, ceiling)
    }

    /// The folder an isolated run names as its home, and an empty file in it that stands for every configuration file.
    fn isolated_home(&self) -> Result<(PathBuf, PathBuf), (i32, String)> {
        let home = self.root()?.join(".isolated");
        fs::create_dir_all(&home)
            .map_err(|error| refused(format!("the fetch could not be made: {error}")))?;
        // A file, since Git for Windows cannot open the null device as a configuration file.
        let empty = home.join("empty.config");
        fs::write(&empty, b"")
            .map_err(|error| refused(format!("the fetch could not be made: {error}")))?;
        Ok((home, empty))
    }

    /// [`Self::run`] under `trust`.
    pub(super) fn run_as(
        &self,
        trust: Trust,
        args: &[OsString],
        stdin: Option<Vec<u8>>,
        guard: Option<&Path>,
        ceiling: usize,
    ) -> Result<Vec<u8>, (i32, String)> {
        let git = self.git()?;
        // Run from the overlays folder with the ceiling at its parent, so git looks no higher for a repository whose configuration
        // it would read; what names a cache takes `--git-dir`.
        let root = self.root()?;
        fs::create_dir_all(root)
            .map_err(|error| refused(format!("the overlays folder could not be made: {error}")))?;
        let no_hooks = if cfg!(windows) {
            "core.hooksPath=NUL"
        } else {
            "core.hooksPath=/dev/null"
        };
        self.git_runs
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let isolated = if trust == Trust::Isolated {
            Some(self.isolated_home()?)
        } else {
            None
        };
        let mut command = Command::new(git);
        #[cfg(test)]
        for (name, value) in &self.person {
            if value.is_empty() {
                command.env_remove(name);
            } else {
                command.env(name, value);
            }
        }
        if isolated.is_some() {
            command.env_clear();
            for name in KEPT_VARIABLES {
                if let Some(value) = self.inherited(name) {
                    command.env(name, value);
                }
            }
        }
        for (name, _) in std::env::vars_os() {
            if name
                .to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("GIT_")
            {
                command.env_remove(name);
            }
        }
        // Git reaches the network as everything else here does: the system's proxy for the remote it is asked about (as variables,
        // which a person's own `http.proxy` still overrides) and, on Windows, the operating system's TLS library, so the system's
        // certificate store and client certificates are used.
        let network = self.network.clone().unwrap_or_else(Network::current);
        if cfg!(windows) && network.git_tls() == GitTls::System {
            command.args(["-c", "http.sslBackend=schannel"]);
        }
        command.network(std::sync::Arc::clone(&network));
        let remotes: Vec<String> = args
            .iter()
            .filter_map(|arg| arg.to_str())
            .filter(|arg| arg.starts_with("https://") || arg.starts_with("http://"))
            .map(str::to_owned)
            .collect();
        if !remotes.is_empty() {
            command.reaches(remotes);
        }
        #[cfg(test)]
        for (address, repository) in &self.rewrites {
            command.args(["-c", &format!("url.{repository}.insteadOf={address}")]);
        }
        command.args(["-c", "protocol.allow=never"]);
        for scheme in ["https", "http", "ssh", "git", "file"] {
            command.args(["-c", &format!("protocol.{scheme}.allow=always")]);
        }
        if let Some((home, empty)) = &isolated {
            for setting in ISOLATED_SETTINGS {
                command.args(["-c", setting]);
            }
            command
                .env("HOME", home)
                .env("USERPROFILE", home)
                .env("XDG_CONFIG_HOME", home)
                .env("GIT_CONFIG_GLOBAL", empty)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_ALLOW_PROTOCOL", ISOLATED_PROTOCOLS.join(":"))
                .env("GIT_SSH_COMMAND", isolated_ssh(&self.ssh_program(), empty));
        }
        let mut child = command
            .args([
                "-c",
                no_hooks,
                "-c",
                "core.fsmonitor=false",
                "-c",
                "credential.interactive=never",
            ])
            .args(args)
            .current_dir(root)
            .env("GIT_CEILING_DIRECTORIES", root.parent().unwrap_or(root))
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never")
            .env("SSH_ASKPASS_REQUIRE", "never")
            .env("GIT_LITERAL_PATHSPECS", "1")
            .stdin(if stdin.is_some() {
                Stdio::Piped
            } else {
                Stdio::Null
            })
            .stdout(Stdio::Piped)
            .stderr(Stdio::Piped)
            .spawn()
            .map_err(|_| refused("git could not be started"))?;
        let writer = stdin.and_then(|bytes| {
            child.stdin.take().map(|mut pipe| {
                std::thread::spawn(move || {
                    let _ = pipe.write_all(&bytes);
                })
            })
        });
        let complaint = child.stderr.take().map(|mut pipe| {
            std::thread::spawn(move || {
                let mut said = Vec::new();
                let _ = pipe.by_ref().take(2048).read_to_end(&mut said);
                // Drained, so git never waits on a full pipe.
                let _ = std::io::copy(&mut pipe, &mut std::io::sink());
                said
            })
        });
        let mut stdout = child.stdout.take();
        let past = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let raised = std::sync::Arc::clone(&past);
        let reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut chunk = [0_u8; 64 * 1024];
            let mut over = false;
            if let Some(stdout) = stdout.as_mut() {
                loop {
                    match stdout.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if bytes.len() + n > ceiling {
                                over = true;
                                raised.store(true, std::sync::atomic::Ordering::SeqCst);
                                break;
                            }
                            bytes.extend_from_slice(&chunk[..n]);
                        }
                    }
                }
            }
            (bytes, over)
        });
        let started = Instant::now();
        let mut checked = Instant::now();
        let mut too_large = false;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {}
                Err(_) => break None,
            }
            if past.load(std::sync::atomic::Ordering::SeqCst) {
                // The reader stopped before git did: git wrote more than it may.
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            if started.elapsed() > GIT_TIME {
                let _ = child.kill();
                let _ = child.wait();
                return Err(refused("git ran too long and was ended"));
            }
            if let Some(guard) = guard
                && checked.elapsed() > Duration::from_millis(500)
            {
                checked = Instant::now();
                if folder_size(guard) > MAX_FETCH_BYTES {
                    too_large = true;
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        if let Some(writer) = writer {
            let _ = writer.join();
        }
        let (bytes, over) = reader.join().unwrap_or_default();
        let said = complaint
            .and_then(|reader| reader.join().ok())
            .map(|said| {
                String::from_utf8_lossy(&said)
                    .chars()
                    .map(|c| if c.is_control() { ' ' } else { c })
                    .take(300)
                    .collect::<String>()
            })
            .unwrap_or_default();
        if too_large {
            return Err(refused(format!(
                "The source sent more than {MAX_FETCH_BYTES} bytes: nothing was installed"
            )));
        }
        match status {
            Some(status) if status.success() && !over => Ok(bytes),
            _ if over => Err(refused("git printed more than this host reads")),
            _ => {
                let word = args
                    .iter()
                    .find(|arg| !arg.to_string_lossy().starts_with('-'))
                    .map_or_else(String::new, |arg| arg.to_string_lossy().into_owned());
                Err(refused(format!("git {word} failed: {}", said.trim())))
            }
        }
    }
}

fn folder_size(dir: &Path) -> u64 {
    let mut size = 0;
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        let Ok(reader) = fs::read_dir(&next) else {
            continue;
        };
        for entry in reader.flatten() {
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => pending.push(entry.path()),
                Ok(kind) if kind.is_file() => size += entry.metadata().map_or(0, |meta| meta.len()),
                _ => {}
            }
        }
    }
    size
}

pub(in crate::control) fn find_git() -> Result<PathBuf, String> {
    let names: &[&str] = if cfg!(windows) {
        &["git.exe", "git.cmd"]
    } else {
        &["git"]
    };
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        if !dir.is_absolute() {
            continue;
        }
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err("git is not in an absolute directory of PATH".to_owned())
}
