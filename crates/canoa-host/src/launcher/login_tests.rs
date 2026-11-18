use std::net::TcpListener;

use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("canoa-login-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a folder");
    dir
}

fn command(config: &str) -> Vec<String> {
    vec![
        "/opt/canoa app/canoa".to_owned(),
        "office-launcher".to_owned(),
        "--config".to_owned(),
        config.to_owned(),
    ]
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("a listing")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn a_windows_command_line_quotes_what_needs_it_and_reads_back() {
    assert_eq!(quote_windows("canoa.exe"), "canoa.exe");
    assert_eq!(quote_windows(""), "\"\"");
    assert_eq!(
        quote_windows(r"C:\Program Files\Canoa\canoa.exe"),
        r#""C:\Program Files\Canoa\canoa.exe""#
    );
    assert_eq!(quote_windows(r"a b\"), r#""a b\\""#);
    assert_eq!(quote_windows(r#"say "hi""#), r#""say \"hi\"""#);
}

#[test]
fn a_desktop_entrys_exec_quotes_percent_and_the_shell_characters_and_escapes_its_backslash() {
    let line = exec_line(&[
        "/opt/canoa app/canoa".to_owned(),
        "100%".to_owned(),
        "a$b".to_owned(),
        r"c\d".to_owned(),
    ]);
    assert_eq!(line, r#""/opt/canoa app/canoa" 100%% "a\\$b" "c\\\\d""#);
    assert_eq!(exec_line(&["plain".to_owned()]), "plain");
}

#[test]
fn a_property_list_holds_the_arguments_and_they_come_back_whole() {
    let arguments = vec![
        "/Applications/Canoa & Co/canoa".to_owned(),
        "office-launcher".to_owned(),
        "--config".to_owned(),
        "/Users/me/<x>.cfg".to_owned(),
    ];
    let text = plist(&arguments);
    assert!(text.contains("<key>RunAtLoad</key>\n  <true/>"));
    assert!(text.contains(LABEL));
    assert!(!text.contains("<x>"), "markup is escaped");
    assert_eq!(plist_arguments(&text), arguments);
}

#[test]
fn the_command_runs_this_program_with_the_config_it_was_given_made_absolute() {
    let none = command_of(Path::new("/bin/canoa"), &[], None, false);
    assert_eq!(none, vec!["/bin/canoa", "office-launcher"]);
    let blank = command_of(Path::new("/bin/canoa"), &[], Some(""), false);
    assert_eq!(blank, none, "an empty name names nothing");

    let leading = vec!["--canoa-child".to_owned()];
    let named = command_of(Path::new("/bin/canoa"), &leading, Some("canoa.cfg"), false);
    assert_eq!(
        &named[..4],
        ["/bin/canoa", "--canoa-child", "office-launcher", "--config"]
    );
    let absolute = std::path::absolute("canoa.cfg").expect("absolute");
    assert_eq!(named[4], absolute.display().to_string());

    let hidden = command_of(Path::new("/bin/canoa"), &leading, Some("canoa.cfg"), true);
    assert_eq!(hidden[..hidden.len() - 1], named, "only --detach is added");
    assert_eq!(hidden.last().map(String::as_str), Some("--detach"));

    let list = config_value("+a.cfg");
    assert!(list.starts_with('+'));
    assert!(Path::new(&list[1..]).is_absolute());
}

#[test]
fn installing_writes_one_file_that_names_the_config_and_again_replaces_it() {
    for place in [
        |dir: &Path| Place::Autostart(dir.to_owned()),
        |dir: &Path| Place::LaunchAgents(dir.to_owned()),
    ] {
        let dir = scratch("file");
        let place = place(&dir.join("entries"));
        assert_eq!(place.registered().expect("read"), None);

        place
            .install(&command("/home/me/canoa.cfg"))
            .expect("installed");
        let once = std::fs::read_to_string(place.file().expect("a file")).expect("written");
        assert!(once.contains("/home/me/canoa.cfg"), "{once}");
        assert_eq!(files_in(&dir.join("entries")).len(), 1);
        assert_eq!(
            place.registered().expect("read"),
            Some(place.text_of(&command("/home/me/canoa.cfg")))
        );

        place
            .install(&command("/home/me/canoa.cfg"))
            .expect("installed again");
        assert_eq!(
            std::fs::read_to_string(place.file().expect("a file")).expect("written"),
            once,
            "the same install writes the same entry"
        );
        place
            .install(&command("/home/me/other.cfg"))
            .expect("replaced");
        assert_eq!(
            files_in(&dir.join("entries")).len(),
            1,
            "one entry, whatever the config"
        );
        assert!(
            std::fs::read_to_string(place.file().expect("a file"))
                .expect("written")
                .contains("other.cfg")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn removing_deletes_the_entry_and_only_it() {
    for place in [
        |dir: &Path| Place::Autostart(dir.to_owned()),
        |dir: &Path| Place::LaunchAgents(dir.to_owned()),
    ] {
        let dir = scratch("remove");
        let entries = dir.join("entries");
        std::fs::create_dir_all(&entries).expect("a folder");
        std::fs::write(entries.join("other.desktop"), "someone else's").expect("a neighbour");
        let place = place(&entries);
        assert!(!place.remove().expect("nothing to remove"));

        place.install(&command("/c.cfg")).expect("installed");
        assert_eq!(files_in(&entries).len(), 2);
        assert!(place.remove().expect("removed"));
        assert_eq!(files_in(&entries), vec!["other.desktop"]);
        assert_eq!(place.registered().expect("read"), None);
        assert!(
            !place.remove().expect("removed twice"),
            "removing again is no error"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn status_says_whether_it_is_registered_and_whether_a_launcher_answers() {
    let dir = scratch("status");
    let place = Place::Autostart(dir.join("entries"));
    let wanted = command("/c.cfg");

    let held = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = held.local_addr().expect("its address");
    let free = {
        let other = TcpListener::bind("127.0.0.1:0").expect("a port");
        other.local_addr().expect("its address")
    };

    let nothing = status(&place, &wanted, Some(free), false).expect("status");
    assert_eq!(nothing.holds, None);
    assert!(!nothing.answers && !nothing.launcher_answers());
    assert!(nothing.describe(false).contains("registered: no"));

    place.install(&wanted).expect("installed");
    let launcher = status(&place, &wanted, Some(address), false).expect("status");
    assert!(launcher.holds.is_some() && launcher.current);
    assert!(launcher.answers && launcher.launcher_answers());
    let said = launcher.describe(false);
    assert!(said.contains("registered: yes"), "{said}");
    assert!(said.contains("a launcher answers: yes"), "{said}");

    // A host holds the home and the address: what answers is the host, not the launcher.
    let host = status(&place, &wanted, Some(address), true).expect("status");
    assert!(host.answers && !host.launcher_answers());
    let value: serde_json::Value =
        serde_json::from_str(&host.describe(true)).expect("one JSON object");
    assert_eq!(value["registered"], true);
    assert_eq!(value["launcher_answers"], false);

    // An entry written for another config is registered and not current.
    let stale = status(&place, &command("/other.cfg"), Some(address), false).expect("status");
    assert!(stale.holds.is_some() && !stale.current);
    assert!(stale.describe(false).contains("run it again"));

    // No add-in address in canoa.cfg: nothing to look at.
    let none = status(&place, &wanted, None, false).expect("status");
    assert!(!none.answers && none.describe(false).contains("address: none"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_run_value_starts_the_launcher_detached_and_an_entry_without_that_is_not_current() {
    let place = Place::RunKey(r"Software\Canoa\Test".to_owned());
    assert!(place.needs_detach(), "a console program started at sign-in");
    assert!(!Place::Autostart(PathBuf::from("a")).needs_detach());
    assert!(!Place::LaunchAgents(PathBuf::from("a")).needs_detach());

    let program = Path::new(r"C:\Program Files\Canoa\canoa.exe");
    let config = Some(r"C:\Users\me\canoa.cfg");
    let wanted = command_of(program, &[], config, true);
    assert_eq!(
        place.text_of(&wanted),
        r#""C:\Program Files\Canoa\canoa.exe" office-launcher --config C:\Users\me\canoa.cfg --detach"#
    );

    // What `reg query` lists is what `status` compares with what `--install` would write now.
    let listing = |written: &str| {
        format!("\r\nHKEY_CURRENT_USER\\Run\r\n    {VALUE}    REG_SZ    {written}\r\n")
    };
    let held = value_in(&listing(&place.text_of(&wanted)), VALUE);
    assert_eq!(held, Some(place.text_of(&wanted)));
    let before = place.text_of(&command_of(program, &[], config, false));
    assert_ne!(
        value_in(&listing(&before), VALUE),
        Some(place.text_of(&wanted)),
        "an entry written without --detach is not current: --install replaces it"
    );
}

/// A key of its own under `HKCU\Software\CanoaTest`, deleted when the test ends, so the person's Run key is never written.
#[cfg(windows)]
struct ScratchKey(String);

#[cfg(windows)]
impl ScratchKey {
    fn new(name: &str) -> Self {
        let key = format!(
            r"Software\CanoaTest\office-launcher-{name}-{}",
            std::process::id()
        );
        let made = Self(key);
        made.delete();
        made
    }

    fn delete(&self) {
        let _ = run_reg(&["delete", &format!(r"HKCU\{}", self.0), "/f"]);
    }
}

#[cfg(windows)]
impl Drop for ScratchKey {
    fn drop(&mut self) {
        self.delete();
    }
}

#[cfg(windows)]
#[test]
fn the_run_key_gets_one_value_that_names_the_config_and_removing_touches_only_it() {
    let scratch_key = ScratchKey::new("run");
    let place = Place::RunKey(scratch_key.0.clone());
    let program = r"C:\Program Files\Canoa App\canoa.exe";
    let wanted = vec![
        program.to_owned(),
        "office-launcher".to_owned(),
        "--config".to_owned(),
        r"C:\Users\me\My Config\canoa.cfg".to_owned(),
    ];
    assert_eq!(place.registered().expect("read"), None, "no key yet");
    assert!(!place.remove().expect("nothing to remove"));

    // Another program's value in the same key stays through all of it.
    reg(&[
        "add",
        &format!(r"HKCU\{}", scratch_key.0),
        "/v",
        "Neighbour",
        "/t",
        "REG_SZ",
        "/d",
        "keep",
        "/f",
    ])
    .expect("a neighbour");

    place.install(&wanted).expect("installed");
    place.install(&wanted).expect("installed again");
    let held = place.registered().expect("read").expect("registered");
    assert_eq!(held, place.text_of(&wanted));
    assert_eq!(
        held,
        r#""C:\Program Files\Canoa App\canoa.exe" office-launcher --config "C:\Users\me\My Config\canoa.cfg""#
    );
    let listing = reg_query(&format!(r"HKCU\{}", scratch_key.0))
        .expect("query")
        .expect("a key");
    assert_eq!(
        listing.matches("REG_SZ").count(),
        2,
        "the entry and the neighbour: {listing}"
    );

    assert!(place.remove().expect("removed"));
    assert_eq!(place.registered().expect("read"), None);
    let listing = reg_query(&format!(r"HKCU\{}", scratch_key.0))
        .expect("query")
        .expect("a key");
    assert!(
        listing.contains("Neighbour") && listing.contains("keep"),
        "{listing}"
    );
    assert!(!place.remove().expect("removed twice"));
}

#[cfg(windows)]
#[test]
fn the_real_run_key_is_the_one_windows_reads_at_sign_in() {
    // Only its address: nothing here writes it.
    let place = Place::current().expect("a place");
    assert_eq!(
        place.location(),
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run\CanoaOfficeLauncher"
    );
}
