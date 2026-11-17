//! A program the host starts finds a runtime the Store installed on its `PATH`, after the person's own folders.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::path::PathBuf;

use canoa_host::spawn::Command;
use canoa_host::store::shelf::{MadeBy, Shelf, record_tree};

/// The test runs itself as the program it starts, which prints its own `PATH`.
const CHILD: &str = "CANOA_TEST_PRINT_PATH";

#[test]
fn a_started_program_has_the_installed_runtime_after_the_persons_path() {
    if std::env::var_os(CHILD).is_some() {
        println!(
            "PATH<<{}>>",
            std::env::var("PATH").unwrap_or_else(|_| "none".to_owned())
        );
        return;
    }
    let root = std::env::temp_dir().join(format!("canoa-runtime-path-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a folder");
    // The programs folder is read from the environment, which is this test's alone: it is the file's only test.
    let variable = if cfg!(windows) {
        "LOCALAPPDATA"
    } else {
        "XDG_DATA_HOME"
    };
    // SAFETY: no other thread of this process reads the environment while the test runs.
    unsafe { std::env::set_var(variable, &root) };
    let shelf = Shelf::unchecked(Shelf::default_root().expect("a programs folder"));
    let program = shelf.program("node").expect("a name");
    let locked = program.lock().expect("locked");
    let staging = locked.stage("1.0.0").expect("staged");
    let binary = staging.join("node");
    std::fs::write(&binary, b"node").expect("written");
    let launch = canoa::store::Launch::new(binary.to_string_lossy().into_owned());
    let mut files = Vec::new();
    record_tree(&staging, &mut files).expect("recorded");
    locked
        .commit(
            &staging,
            "1.0.0",
            &launch,
            files,
            MadeBy {
                provider: "node_store".to_owned(),
                version: "0".to_owned(),
            },
        )
        .expect("committed");
    drop(locked);

    let own = std::env::var_os("PATH").unwrap_or_default();
    let run = |clear: bool| {
        let mut command = Command::new(std::env::current_exe().expect("this test"));
        command
            .args([
                "--exact",
                "a_started_program_has_the_installed_runtime_after_the_persons_path",
                "--nocapture",
            ])
            .env(CHILD, "1");
        if clear {
            command.env_clear().env(CHILD, "1");
        }
        let output = command.output().expect("it runs");
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    let printed = run(false);
    let path = printed
        .split("PATH<<")
        .nth(1)
        .and_then(|rest| rest.split(">>").next())
        .unwrap_or_else(|| panic!("no PATH printed: {printed}"));
    let folders: Vec<PathBuf> = std::env::split_paths(path).collect();
    let mine: Vec<PathBuf> = std::env::split_paths(&own).collect();
    let runtime = binary.parent().expect("a folder").to_path_buf();
    assert_eq!(
        folders.last(),
        Some(&runtime),
        "the runtime's folder is last: {path}"
    );
    assert_eq!(
        &folders[..mine.len()],
        &mine[..],
        "the person's own folders come first, as they were"
    );
    // A program started with nothing in its environment is given nothing.
    assert!(run(true).contains("PATH<<none>>"), "{}", run(true));
    let _ = std::fs::remove_dir_all(&root);
}
