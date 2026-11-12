//! Small rules the host keeps for the stores: what looks like a credential, what a line may carry to the person, which
//! paths and names an archive or a fetch may use, and what the guarded spawner lets a store start.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use canoa::store::{Change, Check, FetchRequest, InstallTools, PATH, UnpackRequest};
use canoa_host::store::exec::{GuardedSpawner, Run, clean, is_credential_name};
use canoa_host::store::footprint::{Footprint, Watch, uncovered};
use canoa_host::store::tools::{HttpTransport, LocalTools};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_harness::LocalSpawner;

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let mut nonce = [0u8; 4];
        getrandom::fill(&mut nonce).expect("random");
        let hex: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let path = std::env::temp_dir().join(format!("canoa-security-{name}-{hex}"));
        fs::create_dir_all(&path).expect("a directory");
        Self(path)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn names_that_carry_a_secret_are_credentials() {
    for name in [
        "ANTHROPIC_API_KEY",
        "GITHUB_TOKEN",
        "NPMTOKEN",
        "GH_PAT",
        "SSH_AUTH_SOCK",
        "DOTAGENTS_SECRET_THING",
        "dotagents_x",
        "OPENAIAPIKEY",
        "GOOGLE_APPLICATION_CREDENTIALS",
        "AWS_SHARED_CREDENTIALS",
        "AWS_SECRET_ACCESS_KEY_ID_PASSWORD",
    ] {
        assert!(is_credential_name(name), "{name}");
    }
    for name in ["PATH", "HOME", "TEMP", "SystemRoot", "PATHEXT", "LANG"] {
        assert!(!is_credential_name(name), "{name}");
    }
}

#[test]
fn a_line_the_person_reads_loses_every_character_that_can_hide_or_reorder_text() {
    let hidden = [
        '\u{2028}',
        '\u{2029}',
        '\u{061C}',
        '\u{200B}',
        '\u{200D}',
        '\u{2060}',
        '\u{FEFF}',
        '\u{202E}',
        '\u{1b}',
        '\u{ad}',
        '\u{180e}',
        '\u{2061}',
        '\u{2064}',
        '\u{fe0f}',
        '\u{e0041}',
        '\u{e0100}',
    ];
    let line: String = "ok".chars().chain(hidden).chain("done".chars()).collect();
    let cleaned = clean(&line);
    for c in hidden {
        assert!(!cleaned.contains(c), "{c:?} survived in {cleaned:?}");
    }
    assert!(cleaned.starts_with("ok") && cleaned.ends_with("done"));
}

#[test]
fn comparing_a_path_with_a_declared_one_never_splits_a_character() {
    let dir = Dir::new("boundary");
    let home = dir.0.join("home");
    fs::create_dir_all(&home).expect("home");
    let watch = || Watch::new(&home);
    let before = Footprint::take(&watch());
    fs::write(home.join("a\u{e9}"), b"x").expect("a file");
    let diff = before.diff(&Footprint::take(&watch()));
    let declared = Change::new(
        PATH,
        home.join("ab").to_string_lossy(),
        "a different name",
        false,
    );
    // A prefix that ends inside the two bytes of the changed name's second letter must be an ordinary no.
    let left = uncovered(&diff, &[declared]);
    assert_eq!(left.len(), diff.len());
}

fn tools(dir: &Dir) -> LocalTools {
    LocalTools::new(Arc::new(HttpTransport::default()), vec![]).granted(&dir.0)
}

#[test]
fn superscript_device_names_are_refused_like_the_plain_ones() {
    let dir = Dir::new("devices");
    for name in ["COM\u{b9}", "LPT\u{b2}.txt", "com\u{b3}", "COM1", "nul.txt"] {
        let request = FetchRequest::new(
            "https://x.example/y",
            name,
            Check::Unverified { why: "t".into() },
            10,
        );
        let error = tools(&dir).fetch(request).expect_err(name);
        assert!(
            error.message().contains("leaves its folder"),
            "{name}: {}",
            error.message()
        );
    }
}

fn tar_of(entries: &[(String, bool)]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut builder = tar::Builder::new(gz);
    for (path, dir) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(if *dir {
            tar::EntryType::Directory
        } else {
            tar::EntryType::Regular
        });
        header.set_size(0);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, path, std::io::empty())
            .expect("entry");
    }
    builder.into_inner().expect("tar").finish().expect("gz")
}

fn unpack(
    dir: &Dir,
    bytes: &[u8],
    max_files: u64,
) -> Result<canoa::store::Unpacked, coqui_acp_api::guatiao::library::ProviderError> {
    fs::write(dir.0.join("a.tar.gz"), bytes).expect("archive");
    tools(dir).unpack(UnpackRequest::new(
        "a.tar.gz",
        "out",
        "tar.gz",
        1 << 20,
        max_files,
    ))
}

#[test]
fn directories_count_against_the_file_limit() {
    let dir = Dir::new("manydirs");
    let entries: Vec<(String, bool)> = (0..50).map(|n| (format!("d{n}/"), true)).collect();
    assert!(
        unpack(&dir, &tar_of(&entries), 10).is_err(),
        "fifty directories passed a limit of ten"
    );
}

#[test]
fn an_archive_nested_past_the_depth_cap_is_refused() {
    let dir = Dir::new("deep");
    let deep = (0..200)
        .map(|n| format!("d{n}"))
        .collect::<Vec<_>>()
        .join("/");
    let entries = vec![(format!("{deep}/f"), false)];
    assert!(unpack(&dir, &tar_of(&entries), 1000).is_err());
}

#[test]
fn a_working_directory_that_climbs_out_of_the_staging_folder_is_refused() {
    let dir = Dir::new("cwd");
    let staging = dir.0.join("staging");
    fs::create_dir_all(&staging).expect("staging");
    let spawner = GuardedSpawner::new(LocalSpawner::new(), staging.to_string_lossy());
    let log = spawner.log();
    let program = if cfg!(windows) {
        r"C:\Windows\System32\cmd.exe"
    } else {
        "/bin/sh"
    };
    let climbing = staging.join("..").join("outside");
    let spec = coqui_acp_api::SpawnSpec::new(program).cwd(climbing.to_string_lossy());
    let _ = spawner.spawn(spec);
    assert!(
        log.violations()
            .iter()
            .any(|v| v.contains("working directory")),
        "violations: {:?}, spawned: {:?}",
        log.violations(),
        log.spawned()
    );
    let _ = std::io::stdout().flush();
}

#[test]
fn an_install_is_not_given_npm_node_or_ssh_agent_variables_of_the_person() {
    use coqui_acp_api::Environment;
    let dir = Dir::new("env");
    let spawner = LocalSpawner::new().into_object();
    let mut env = Environment::new();
    for (name, value) in [
        ("npm_config_registry", "https://elsewhere.example/"),
        ("NPM_CONFIG_USERCONFIG", "/somewhere/.npmrc"),
        ("NODE_OPTIONS", "--require /somewhere/x.js"),
        ("SSH_AUTH_SOCK", "/tmp/agent.sock"),
        ("KEPT_VARIABLE", "kept"),
    ] {
        env.insert(name, value);
    }
    if let Ok(root) = std::env::var("SystemRoot") {
        env.insert("SystemRoot", root);
    }
    let (program, args): (String, Vec<&str>) = if cfg!(windows) {
        (
            format!(
                "{}\\System32\\cmd.exe",
                std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into())
            ),
            vec!["/c", "set"],
        )
    } else {
        ("/usr/bin/env".to_owned(), vec![])
    };
    let run = Run::new(
        &spawner,
        dir.0.to_string_lossy(),
        env,
        std::time::Instant::now() + std::time::Duration::from_secs(20),
    );
    let output = run
        .exec(&program, &args, &[("npm_config_userconfig", "/ours/npmrc")])
        .expect("runs");
    let text = output.text.to_ascii_lowercase();
    assert!(text.contains("kept_variable=kept"), "{text}");
    assert!(
        text.contains("npm_config_userconfig=/ours/npmrc"),
        "the store's own setting is passed"
    );
    for gone in ["elsewhere.example", "somewhere", "agent.sock"] {
        assert!(!text.contains(gone), "{gone} was passed on: {text}");
    }
}

#[test]
fn a_script_named_with_a_trailing_space_or_dot_is_still_a_script() {
    let dir = Dir::new("scripts");
    let spawner = GuardedSpawner::new(LocalSpawner::new(), dir.0.to_string_lossy());
    let log = spawner.log();
    for name in ["run.cmd ", "run.bat.", "run.ps1 .", "RUN.CMD"] {
        let program = dir.0.join(name).to_string_lossy().into_owned();
        let _ = spawner.spawn(coqui_acp_api::SpawnSpec::new(program).cwd(dir.0.to_string_lossy()));
    }
    assert_eq!(log.violations().len(), 4, "{:?}", log.violations());
}
