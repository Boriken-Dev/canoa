//! The store against real folders: what an install reuses, repairs, replaces and refuses.

use canoa_launch::{
    Entry, Error, MARKER, Manifest, Payload, differs, install, pack, prune, sha256_hex,
};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("canoa-launch-test-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// An app folder: a program, a library and a nested data file, each carrying `tag`.
fn app(root: &Path, tag: &str) -> PathBuf {
    let dir = root.join(format!("app-{tag}"));
    fs::create_dir_all(dir.join("data/assets")).unwrap();
    fs::write(dir.join("CanoaUI.exe"), format!("program {tag}")).unwrap();
    fs::write(dir.join("canoa_ffi.dll"), format!("library {tag}")).unwrap();
    fs::write(dir.join("data/assets/a.bin"), vec![7u8; 200_000]).unwrap();
    dir
}

struct Built {
    bytes: Vec<u8>,
    sha: String,
}

impl Built {
    fn new(dir: &Path, version: &str) -> Self {
        let bytes = pack(dir, version).unwrap();
        let sha = sha256_hex(&bytes);
        Self { bytes, sha }
    }
    fn payload(&self) -> Payload<'_> {
        Payload {
            bytes: &self.bytes,
            sha256: &self.sha,
        }
    }
}

fn link(at: &Path, to: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(to, at).unwrap();
    #[cfg(windows)]
    {
        // A junction needs no privilege; the symbolic link does.
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(at.components().collect::<PathBuf>())
            .arg(to.components().collect::<PathBuf>())
            .output()
            .unwrap();
        assert!(
            made.status.success(),
            "mklink /J: {}",
            String::from_utf8_lossy(&made.stdout)
        );
    }
}

#[test]
fn a_first_start_unpacks_every_file_and_a_second_reuses_it() {
    let root = scratch("first");
    let built = Built::new(&app(&root, "1"), "1.0.0");
    let base = root.join("base");
    let first = install(&base, &built.payload()).unwrap();
    assert!(!first.reused);
    assert_eq!(first.dir, base.join("1.0.0"));
    assert_eq!(
        fs::read_to_string(first.dir.join("CanoaUI.exe")).unwrap(),
        "program 1"
    );
    assert_eq!(
        fs::read(first.dir.join("data/assets/a.bin")).unwrap().len(),
        200_000
    );
    assert_eq!(
        fs::read_to_string(first.dir.join(MARKER)).unwrap(),
        first.manifest.text()
    );

    let stamp = fs::metadata(first.dir.join("CanoaUI.exe"))
        .unwrap()
        .modified()
        .unwrap();
    sleep(Duration::from_millis(30));
    let second = install(&base, &built.payload()).unwrap();
    assert!(second.reused);
    assert_eq!(
        fs::metadata(second.dir.join("CanoaUI.exe"))
            .unwrap()
            .modified()
            .unwrap(),
        stamp
    );
    assert!(differs(&second.dir, &second.manifest).unwrap().is_none());
    // No temporary sibling is left behind.
    let names: Vec<_> = fs::read_dir(&base)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn packing_the_same_folder_twice_gives_the_same_bytes() {
    let root = scratch("determinism");
    let dir = app(&root, "1");
    assert_eq!(pack(&dir, "1.0.0").unwrap(), pack(&dir, "1.0.0").unwrap());
    assert_ne!(pack(&dir, "1.0.0").unwrap(), pack(&dir, "1.0.1").unwrap());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_changed_file_is_replaced_by_a_fresh_unpack() {
    let root = scratch("tamper");
    let built = Built::new(&app(&root, "1"), "1.0.0");
    let base = root.join("base");
    let first = install(&base, &built.payload()).unwrap();
    fs::write(first.dir.join("canoa_ffi.dll"), "library evil").unwrap();
    assert!(
        differs(&first.dir, &first.manifest)
            .unwrap()
            .unwrap()
            .contains("canoa_ffi.dll")
    );
    let again = install(&base, &built.payload()).unwrap();
    assert!(!again.reused);
    assert_eq!(
        fs::read_to_string(again.dir.join("canoa_ffi.dll")).unwrap(),
        "library 1"
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_same_size_change_is_caught_by_the_digest() {
    let root = scratch("samesize");
    let built = Built::new(&app(&root, "1"), "1.0.0");
    let base = root.join("base");
    let first = install(&base, &built.payload()).unwrap();
    let path = first.dir.join("data/assets/a.bin");
    let mut bytes = fs::read(&path).unwrap();
    bytes[100_000] ^= 1;
    fs::write(&path, bytes).unwrap();
    assert!(differs(&first.dir, &first.manifest).unwrap().is_some());
    assert!(!install(&base, &built.payload()).unwrap().reused);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_missing_file_and_a_planted_file_are_both_repaired() {
    let root = scratch("plant");
    let built = Built::new(&app(&root, "1"), "1.0.0");
    let base = root.join("base");
    let first = install(&base, &built.payload()).unwrap();

    fs::remove_file(first.dir.join("canoa_ffi.dll")).unwrap();
    assert!(!install(&base, &built.payload()).unwrap().reused);
    assert!(first.dir.join("canoa_ffi.dll").is_file());

    // A library the app would load beside its own, which the manifest does not list.
    fs::write(first.dir.join("planted.dll"), "evil").unwrap();
    fs::create_dir_all(first.dir.join("data/more")).unwrap();
    fs::write(first.dir.join("data/more/x.dll"), "evil").unwrap();
    assert!(
        differs(&first.dir, &first.manifest)
            .unwrap()
            .unwrap()
            .contains("not in the manifest")
    );
    assert!(!install(&base, &built.payload()).unwrap().reused);
    assert!(!first.dir.join("planted.dll").exists());
    assert!(!first.dir.join("data/more").exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_link_inside_the_folder_is_caught() {
    let root = scratch("innerlink");
    let built = Built::new(&app(&root, "1"), "1.0.0");
    let base = root.join("base");
    let first = install(&base, &built.payload()).unwrap();
    let elsewhere = root.join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    link(&first.dir.join("data/hop"), &elsewhere);
    assert!(
        differs(&first.dir, &first.manifest)
            .unwrap()
            .unwrap()
            .contains("link")
    );
    assert!(!install(&base, &built.payload()).unwrap().reused);
    assert!(!first.dir.join("data/hop").exists());
    assert!(elsewhere.is_dir(), "the link's target survives the repair");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_new_version_unpacks_beside_the_old_and_only_two_are_kept() {
    let root = scratch("versions");
    let base = root.join("base");
    let mut dirs = Vec::new();
    for v in ["1.0.0", "1.1.0", "1.2.0"] {
        let built = Built::new(&app(&root, v), v);
        let installed = install(&base, &built.payload()).unwrap();
        assert!(!installed.reused);
        dirs.push(installed.dir.clone());
        let gone = prune(&base, v);
        // The third install pushes the first out; before that nothing is removed.
        assert_eq!(
            gone,
            if v == "1.2.0" {
                vec!["1.0.0".to_owned()]
            } else {
                vec![]
            },
            "{v}"
        );
        sleep(Duration::from_millis(40));
    }
    assert!(!dirs[0].exists());
    assert!(dirs[1].join("CanoaUI.exe").is_file() && dirs[2].join("CanoaUI.exe").is_file());
    assert_eq!(
        fs::read_to_string(dirs[1].join("CanoaUI.exe")).unwrap(),
        "program 1.1.0"
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_rebuilt_same_version_replaces_the_folder_in_place() {
    let root = scratch("rebuild");
    let base = root.join("base");
    let one = Built::new(&app(&root, "a"), "2.0.0");
    let two = Built::new(&app(&root, "b"), "2.0.0");
    install(&base, &one.payload()).unwrap();
    let installed = install(&base, &two.payload()).unwrap();
    assert!(!installed.reused);
    assert_eq!(
        fs::read_to_string(installed.dir.join("CanoaUI.exe")).unwrap(),
        "program b"
    );
    assert_eq!(
        fs::read_dir(&base).unwrap().count(),
        1,
        "the replaced folder is gone"
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn prune_touches_only_what_the_launcher_made() {
    let root = scratch("prune");
    let base = root.join("base");
    let built = Built::new(&app(&root, "1"), "3.0.0");
    install(&base, &built.payload()).unwrap();
    // Folders and files that merely sit beside: no manifest copy, a manifest copy naming another folder, a plain file.
    fs::create_dir_all(base.join("1.0.0")).unwrap();
    fs::write(base.join("1.0.0/precious.txt"), "mine").unwrap();
    fs::create_dir_all(base.join("2.0.0")).unwrap();
    fs::write(
        base.join("2.0.0").join(MARKER),
        built.payload().manifest().unwrap().text(),
    )
    .unwrap();
    fs::write(base.join("notes.txt"), "mine").unwrap();
    // A half-removed folder of the launcher's own naming.
    fs::create_dir_all(base.join(".gone-1-2-3")).unwrap();
    assert!(prune(&base, "3.0.0").is_empty());
    assert!(
        base.join("1.0.0/precious.txt").is_file()
            && base.join("2.0.0").is_dir()
            && base.join("notes.txt").is_file()
    );
    assert!(!base.join(".gone-1-2-3").exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_link_where_the_version_folder_belongs_is_refused_and_not_followed() {
    let root = scratch("verlink");
    let built = Built::new(&app(&root, "1"), "1.0.0");
    let base = root.join("base");
    fs::create_dir_all(&base).unwrap();
    let target = root.join("victim");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("keep.txt"), "keep").unwrap();
    link(&base.join("1.0.0"), &target);
    assert!(matches!(
        install(&base, &built.payload()),
        Err(Error::Refused(_))
    ));
    assert_eq!(fs::read_to_string(target.join("keep.txt")).unwrap(), "keep");
    assert_eq!(
        fs::read_dir(&target).unwrap().count(),
        1,
        "nothing was written through the link"
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_link_as_the_base_folder_is_refused() {
    let root = scratch("baselink");
    let built = Built::new(&app(&root, "1"), "1.0.0");
    let real = root.join("real");
    fs::create_dir(&real).unwrap();
    let base = root.join("base");
    link(&base, &real);
    assert!(matches!(
        install(&base, &built.payload()),
        Err(Error::Refused(_))
    ));
    assert_eq!(fs::read_dir(&real).unwrap().count(), 0);
    fs::remove_dir_all(&root).unwrap();
}

#[cfg(unix)]
#[test]
fn a_folder_others_can_write_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let root = scratch("others");
    let built = Built::new(&app(&root, "1"), "1.0.0");
    let base = root.join("base");
    let first = install(&base, &built.payload()).unwrap();
    fs::set_permissions(&first.dir, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(matches!(
        install(&base, &built.payload()),
        Err(Error::Refused(_))
    ));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn only_a_listed_file_is_a_program() {
    let root = scratch("program");
    let built = Built::new(&app(&root, "1"), "1.0.0");
    let installed = install(&root.join("base"), &built.payload()).unwrap();
    assert!(installed.program("CanoaUI.exe").is_ok());
    for name in ["cmd.exe", "planted.exe", "../CanoaUI.exe", "canoaui.exe"] {
        assert!(
            matches!(installed.program(name), Err(Error::Refused(_))),
            "{name}"
        );
    }
    fs::remove_dir_all(&root).unwrap();
}

// --- payloads the launcher must not trust ---

/// A payload archive of `files` (name, body) under a manifest line list `lines`, as if a hostile builder made it.
fn forged(version: &str, lines: &str, files: &[(&str, &[u8])]) -> Built {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file(MARKER, options).unwrap();
    zip.write_all(format!("canoa-launch-manifest 1\nversion {version}\n{lines}").as_bytes())
        .unwrap();
    for (name, body) in files {
        zip.start_file(*name, options).unwrap();
        zip.write_all(body).unwrap();
    }
    let bytes = zip.finish().unwrap().into_inner();
    let sha = sha256_hex(&bytes);
    Built { bytes, sha }
}

fn line(path: &str, body: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(body).into();
    let entry = Entry {
        path: path.into(),
        size: body.len() as u64,
        sha256: digest,
    };
    let manifest = Manifest {
        version: "x".into(),
        entries: vec![entry],
    };
    manifest.text().lines().nth(2).unwrap().to_owned() + "\n"
}

#[test]
fn a_payload_that_names_a_path_outside_its_folder_unpacks_nothing() {
    let root = scratch("traversal");
    let base = root.join("base");
    for evil in [
        "../evil.dll",
        "a/../../evil.dll",
        "C:/evil.dll",
        "/evil.dll",
        "x:stream",
    ] {
        let built = forged("1.0.0", &line(evil, b"x"), &[(evil, b"x")]);
        assert!(
            matches!(install(&base, &built.payload()), Err(Error::Payload(_))),
            "{evil}"
        );
    }
    assert!(!root.join("evil.dll").exists());
    assert!(!base.exists() || fs::read_dir(&base).unwrap().count() == 0);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_file_whose_content_does_not_match_the_manifest_leaves_nothing_behind() {
    let root = scratch("badcontent");
    let base = root.join("base");
    let built = forged(
        "1.0.0",
        &line("CanoaUI.exe", b"good"),
        &[("CanoaUI.exe", b"evil")],
    );
    assert!(matches!(
        install(&base, &built.payload()),
        Err(Error::Payload(_))
    ));
    assert_eq!(
        fs::read_dir(&base).unwrap().count(),
        0,
        "no version folder and no temporary one"
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn an_archive_with_a_file_the_manifest_does_not_list_is_refused() {
    let root = scratch("extra");
    let base = root.join("base");
    let built = forged(
        "1.0.0",
        &line("CanoaUI.exe", b"ok"),
        &[("CanoaUI.exe", b"ok"), ("extra.dll", b"evil")],
    );
    assert!(matches!(
        install(&base, &built.payload()),
        Err(Error::Payload(_))
    ));
    assert_eq!(fs::read_dir(&base).unwrap().count(), 0);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn an_archive_that_does_not_match_its_recorded_digest_or_is_empty_is_refused() {
    let root = scratch("digest");
    let mut built = Built::new(&app(&root, "1"), "1.0.0");
    let good_sha = built.sha.clone();
    let last = built.bytes.len() / 2;
    built.bytes[last] ^= 0xff;
    assert!(matches!(
        install(&root.join("base"), &built.payload()),
        Err(Error::Payload(_))
    ));
    assert!(matches!(
        install(
            &root.join("base"),
            &Payload {
                bytes: &[],
                sha256: ""
            }
        ),
        Err(Error::Payload(_))
    ));
    assert!(matches!(
        install(
            &root.join("base"),
            &Payload {
                bytes: b"not a zip",
                sha256: &sha256_hex(b"not a zip")
            }
        ),
        Err(Error::Payload(_))
    ));
    assert_eq!(good_sha.len(), 64);
    assert!(!root.join("base").join("1.0.0").exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_payload_cannot_take_the_launchers_own_file_name() {
    let root = scratch("marker");
    let built = forged(
        "1.0.0",
        &line("Canoa-Launch.manifest", b"x"),
        &[("Canoa-Launch.manifest", b"x")],
    );
    assert!(matches!(
        install(&root.join("base"), &built.payload()),
        Err(Error::Payload(_))
    ));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn packing_refuses_a_link_and_a_bad_version() {
    let root = scratch("packlink");
    let dir = app(&root, "1");
    let elsewhere = root.join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    link(&dir.join("hop"), &elsewhere);
    assert!(matches!(pack(&dir, "1.0.0"), Err(Error::Payload(_))));
    assert!(matches!(pack(&dir, "../1"), Err(Error::Payload(_))));
    fs::remove_dir_all(&root).unwrap();
}
