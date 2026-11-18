use super::*;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("canoa-needs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a folder");
    dir
}

/// An empty program called `name` in `dir`: found by name, never run.
fn program(dir: &Path, name: &str) {
    let file = dir.join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    });
    std::fs::write(&file, "").expect("a file");
    #[cfg(unix)]
    std::fs::set_permissions(&file, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("executable");
}

/// What a stand-in says of itself: its folder's name, after a `v`, is its version.
fn said(program: &Path) -> Option<String> {
    let folder = program.parent()?.file_name()?.to_str()?;
    folder.strip_prefix('v').map(str::to_owned)
}

fn finder(dirs: &[&Path]) -> Finder {
    let path = std::env::join_paths(dirs).expect("a path");
    let lookup = move |name: &str| (name == "PATH").then(|| path.to_string_lossy().into_owned());
    Finder::new(&lookup, &[]).with_version(said)
}

#[test]
fn a_version_is_compared_as_numbers_and_read_from_what_a_program_says() {
    assert!(at_least("3.12.4", "3.11"));
    assert!(at_least("3.11", "3.11.0"));
    assert!(at_least("3.10.0", "3.9"));
    assert!(!at_least("3.9.18", "3.10"));
    assert!(!at_least("20.1", "20.1.1"));
    for (answer, version) in [
        ("Python 3.12.1\n", Some("3.12.1")),
        ("v22.4.0\n", Some("22.4.0")),
        ("cargo 1.95.0 (abc 2026-01-01)", Some("1.95.0")),
        ("Python 3.11", Some("3.11")),
        ("no number here", None),
        ("", None),
    ] {
        assert_eq!(version_in(answer).as_deref(), version, "{answer:?}");
    }
}

#[test]
fn a_program_that_is_new_enough_is_met_and_one_that_is_not_is_offered_from_the_store() {
    let root = temp("versions");
    let (old, new, none) = (
        root.join("v3.8.1"),
        root.join("v3.12.4"),
        root.join("empty"),
    );
    for dir in [&old, &new, &none] {
        std::fs::create_dir_all(dir).expect("a folder");
        program(dir, "python");
        program(dir, "python3");
    }
    let python = |minimum: &str| Required {
        python: Some(Some(minimum.to_owned())),
        ..Required::default()
    };
    // The first Python on the path is the one that counts: an older one in front hides a newer one behind it.
    let shadowed = check(&python("3.11"), &finder(&[&old, &new]), "linux");
    assert!(!shadowed[0].met, "{shadowed:?}");
    assert!(shadowed[0].how.contains("3.8.1"), "{}", shadowed[0].how);
    let met = check(&python("3.11"), &finder(&[&new, &old]), "linux");
    assert_eq!(met.len(), 1);
    assert!(met[0].met, "{met:?}");
    assert_eq!(met[0].key, "runtime.python");
    assert!(met[0].how.contains("3.12.4"), "{}", met[0].how);
    assert!(met[0].how.contains("v3.12.4"), "{}", met[0].how);
    assert_eq!(met[0].offer, None);

    let older = check(&python("3.11"), &finder(&[&old]), "linux");
    assert!(!older[0].met);
    assert_eq!(older[0].offer, Some("python_store"));
    assert!(older[0].how.contains("3.8.1"), "{}", older[0].how);
    assert!(older[0].how.contains("3.11 or later"), "{}", older[0].how);
    assert!(older[0].how.contains("Store"), "{}", older[0].how);

    let unsaid = check(&python("3.11"), &finder(&[&none]), "linux");
    assert!(!unsaid[0].met);
    assert!(
        unsaid[0].how.contains("did not say its version"),
        "{}",
        unsaid[0].how
    );

    let absent = check(&python("3.11"), &finder(&[&root]), "linux");
    assert!(!absent[0].met);
    assert!(
        absent[0].how.contains("not on the PATH"),
        "{}",
        absent[0].how
    );
    assert_eq!(absent[0].offer, Some("python_store"));

    // With no version asked for, nothing is run: the program is only found.
    let any = Required {
        python: Some(None),
        ..Required::default()
    };
    assert!(check(&any, &finder(&[&none]), "linux")[0].met);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn commands_and_interpreters_are_found_on_the_path_and_the_stores_runtimes_come_after_it() {
    let root = temp("commands");
    let (own, runtimes) = (root.join("own"), root.join("runtimes"));
    for dir in [&own, &runtimes] {
        std::fs::create_dir_all(dir).expect("a folder");
    }
    program(&own, "tool");
    program(&runtimes, "node");
    let path = own.to_string_lossy().into_owned();
    let lookup = move |name: &str| (name == "PATH").then(|| path.clone());
    let found = Finder::new(&lookup, std::slice::from_ref(&runtimes));
    let required = Required {
        node: Some(None),
        interpreters: vec!["sh".to_owned()],
        commands: vec![
            "tool".to_owned(),
            "canoa-test-absent".to_owned(),
            "npm".to_owned(),
        ],
        ..Required::default()
    };
    let lines = check(&required, &found, "linux");
    let keys: Vec<&str> = lines.iter().map(|line| line.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "runtime.node",
            "runtime.interpreter: sh",
            "runtime.commands: tool",
            "runtime.commands: canoa-test-absent",
            "runtime.commands: npm",
        ]
    );
    let met: Vec<bool> = lines.iter().map(|line| line.met).collect();
    // The runtime's Node is found after the person's own folder; the test's path holds no shell and no npm.
    assert_eq!(met, [true, false, true, false, false]);
    assert_eq!(lines[1].offer, runtime_for("sh"));
    assert_eq!(lines[3].offer, None, "{}", lines[3].how);
    assert_eq!(lines[4].offer, Some("node_store"));
    assert!(lines[4].how.contains("Node.js"), "{}", lines[4].how);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_runtime_list_that_leaves_this_os_out_is_reported() {
    let required = Required {
        other_os: Some(vec!["windows".to_owned(), "macos".to_owned()]),
        ..Required::default()
    };
    let lines = check(&required, &finder(&[]), "linux");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].key, "runtime.os");
    assert!(!lines[0].met);
    assert_eq!(
        lines[0].how,
        "it names windows, macos; this computer is linux"
    );
}

#[test]
fn a_real_program_is_asked_its_version_and_a_missing_one_has_none() {
    let Some(cargo) = std::env::var_os("CARGO") else {
        return;
    };
    let version = version_of(Path::new(&cargo)).expect("cargo says its version");
    assert!(at_least(&version, "1.0"), "{version}");
    assert_eq!(
        version_of(Path::new("no-such-folder/no-such-program")),
        None
    );
}

#[test]
fn the_stores_commands_are_the_ones_its_recipes_list() {
    use crate::store::runtimes::{NodeRecipe, PythonRecipe};
    use canoa::store::recipe::Recipe as _;
    for (commands, id) in [
        (NodeRecipe.info().commands, "node_store"),
        (PythonRecipe.info().commands, "python_store"),
    ] {
        for command in commands {
            assert_eq!(runtime_for(&command), Some(id), "{command}");
        }
    }
}
