//! A source someone else named is fetched with none of the person's git settings, credentials, ssh keys or agent; one the person
//! named keeps them. The person is a temporary home with a `.gitconfig`, a credential helper that records its calls and an
//! `ssh` that records its arguments and environment: nothing here reads the real configuration, keys or agent, or the network.

use std::ffi::OsString;
use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::{Path, PathBuf};

use super::overlays::{OverlayLimits, Overlays};
use super::overlays_tests::{ask, dir, git, key, repo};
use super::*;

const BOTH: [Grant; 2] = [Grant::OverlayRead, Grant::OverlayWrite];

fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

struct Person {
    tmp: PathBuf,
    home: PathBuf,
    helper_log: PathBuf,
    ssh_args: PathBuf,
    ssh_agent: PathBuf,
    control: Control,
}

/// A host whose git runs inherit a person's home: `.gitconfig` rewrites `https://skills.example.invalid/tools.git` to `upstream`
/// and records every call of its credential helper; `ssh` is a program that records and fails; an agent socket is set.
fn person(upstream: &Path, tmp: PathBuf) -> Person {
    let home = tmp.join("person");
    fs::create_dir_all(&home).expect("home");
    let helper_log = tmp.join("helper.log");
    let ssh_args = tmp.join("ssh.args");
    let ssh_agent = tmp.join("ssh.agent");
    let standin = tmp.join("ssh_standin.sh");
    fs::write(
        &standin,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"{}\"\nprintf 'agent=%s\\n' \"${{SSH_AUTH_SOCK-unset}}\" > \"{}\"\nexit 255\n",
            slashed(&ssh_args),
            slashed(&ssh_agent)
        ),
    )
    .expect("ssh");
    fs::write(
        home.join(".gitconfig"),
        format!(
            "[url \"{}\"]\n\tinsteadOf = https://skills.example.invalid/tools.git\n[core]\n\tsshCommand = sh \"{}\"\n[credential]\n\thelper =\n\thelper = !echo called >> \"{}\"; true\n",
            slashed(upstream),
            slashed(&standin),
            slashed(&helper_log)
        ),
    )
    .expect("config");
    let vars: Vec<(&str, OsString)> = vec![
        ("HOME", home.clone().into()),
        ("USERPROFILE", home.clone().into()),
        ("XDG_CONFIG_HOME", home.join("xdg").into()),
        ("SSH_AUTH_SOCK", "/person/agent.sock".into()),
        (
            "CANOA_TEST_SSH",
            format!("sh \"{}\"", slashed(&standin)).into(),
        ),
    ];
    let overlays = tmp.join("cfg").join("overlays");
    let control = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_overlays(Overlays::new(Some(overlays), OverlayLimits::default()).with_person_env(&vars));
    Person {
        tmp,
        home,
        helper_log,
        ssh_args,
        ssh_agent,
        control,
    }
}

fn upstream(tmp: &Path) -> PathBuf {
    repo(
        tmp,
        "tools",
        &[
            ("OVERLAY.md", "x\n"),
            (
                "SKILL.md",
                "---\nname: tools\ndescription: Tools.\n---\nbody\n",
            ),
        ],
    )
}

/// A server that answers every request with a demand for credentials, and its address.
fn demanding_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("binds");
    let address = listener.local_addr().expect("address");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut seen = Vec::new();
            let mut chunk = [0_u8; 1024];
            while !seen.windows(4).any(|window| window == b"\r\n\r\n") {
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => seen.extend_from_slice(&chunk[..n]),
                }
            }
            let _ = stream.write_all(
                b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"x\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    format!("http://{address}/tools.git")
}

fn skill(p: &Person, repo: &str) -> Value {
    skill_at(p, repo, "main")
}

fn skill_at(p: &Person, repo: &str, reference: &str) -> Value {
    ask(
        &p.control,
        &key("k", &BOTH),
        "skill/fetch",
        json!({"repo": repo, "ref": reference}),
    )
}

fn overlay(p: &Person, repo: &str) -> Value {
    overlay_at(p, repo, "main")
}

fn overlay_at(p: &Person, repo: &str, reference: &str) -> Value {
    ask(
        &p.control,
        &key("k", &BOTH),
        "overlay/check",
        json!({"name": "o", "repo": repo, "ref": reference, "path": ""}),
    )
}

const REDIRECTED: &str = "https://skills.example.invalid/tools.git";

#[test]
fn a_skill_fetch_ignores_the_persons_url_rewrites_and_the_person_s_own_source_keeps_them() {
    let tmp = dir("isolated-rewrite");
    let up = upstream(&tmp);
    let p = person(&up, tmp);
    let fetched = skill(&p, REDIRECTED);
    assert!(
        fetched["error"].is_object(),
        "a rewrite in the person's .gitconfig reached a skill's fetch: {fetched}"
    );
    // The person's own source, fetched as they would: their configuration applies.
    let own = overlay(&p, REDIRECTED);
    assert!(own["result"]["token"].is_string(), "{own}");
    let _ = fs::remove_dir_all(p.tmp);
}

#[test]
fn a_skill_fetch_never_calls_the_persons_credential_helper_and_the_persons_own_source_does() {
    let tmp = dir("isolated-helper");
    let up = upstream(&tmp);
    let p = person(&up, tmp);
    let server = demanding_server();
    // A plaintext transport names a whole commit id.
    let id = git(&up, &["rev-parse", "HEAD"]);
    let fetched = skill_at(&p, &server, &id);
    assert!(fetched["error"].is_object(), "{fetched}");
    assert!(
        !p.helper_log.exists(),
        "the credential helper ran for a skill's fetch: {}",
        fs::read_to_string(&p.helper_log).unwrap_or_default()
    );
    let own = overlay_at(&p, &server, &id);
    assert!(own["error"].is_object(), "{own}");
    assert!(
        p.helper_log.exists(),
        "the person's own source did not use their credential helper: {own}"
    );
    let _ = fs::remove_dir_all(p.tmp);
}

#[test]
fn a_skill_fetch_over_ssh_carries_no_agent_no_configuration_and_only_an_empty_identity() {
    let tmp = dir("isolated-ssh");
    let up = upstream(&tmp);
    let p = person(&up, tmp);
    let fetched = skill(&p, "ssh://git@skills.example.invalid/tools.git");
    assert!(fetched["error"].is_object(), "{fetched}");
    let args = fs::read_to_string(&p.ssh_args)
        .unwrap_or_else(|e| panic!("ssh was started: {e}: {fetched}"));
    for wanted in [
        "-F",
        "BatchMode=yes",
        "IdentitiesOnly=yes",
        "IdentityAgent=none",
        "PasswordAuthentication=no",
    ] {
        assert!(args.contains(wanted), "{wanted} missing from {args}");
    }
    let empty = slashed(
        &p.home
            .parent()
            .expect("tmp")
            .join("cfg/overlays/.isolated/empty.config"),
    );
    assert!(
        args.lines().any(|line| line == empty) && args.contains(&format!("IdentityFile={empty}")),
        "ssh was not given the empty file as its configuration and identity: {args}"
    );
    assert_eq!(
        fs::read_to_string(&p.ssh_agent).expect("environment"),
        "agent=unset\n",
        "the person's ssh agent reached a skill's fetch"
    );
    assert!(
        fs::read(
            p.home
                .parent()
                .expect("tmp")
                .join("cfg/overlays/.isolated/empty.config")
        )
        .expect("empty file")
        .is_empty()
    );

    // The person's own source over ssh: their agent is theirs, and no isolating option is added.
    let own = overlay(&p, "ssh://git@skills.example.invalid/tools.git");
    assert!(own["error"].is_object(), "{own}");
    assert_eq!(
        fs::read_to_string(&p.ssh_agent).expect("environment"),
        "agent=/person/agent.sock\n"
    );
    assert!(
        !fs::read_to_string(&p.ssh_args)
            .expect("arguments")
            .contains("IdentityAgent=none")
    );
    let _ = fs::remove_dir_all(p.tmp);
}

#[test]
fn a_catalog_entry_read_ahead_is_fetched_without_the_persons_settings() {
    let tmp = dir("isolated-ahead");
    let up = upstream(&tmp);
    let p = person(&up, tmp);
    let answer = ask(
        &p.control,
        &key("k", &BOTH),
        "store/read_ahead",
        json!({"entries": [
            {"id": "o", "kind": "overlay", "name": "o", "repo": REDIRECTED, "ref": "main"},
            {"id": "s", "kind": "skill", "name": "s", "repo": REDIRECTED, "ref": "main"},
        ]}),
    );
    let entries = &answer["result"]["entries"];
    for id in ["o", "s"] {
        assert!(
            entries[id]["problem"].is_string(),
            "{id} was fetched through the person's configuration: {answer}"
        );
    }
    let _ = fs::remove_dir_all(p.tmp);
}

fn overlay_from_catalog(p: &Person, repo: &str, label: &str) -> Value {
    ask(
        &p.control,
        &key("k", &BOTH),
        "overlay/check",
        json!({"name": "o", "repo": repo, "ref": "main", "path": "", "catalog": label}),
    )
}

#[test]
fn an_overlay_chosen_from_a_catalog_ignores_the_persons_url_rewrites_and_a_typed_one_keeps_them() {
    let tmp = dir("isolated-chosen");
    let up = upstream(&tmp);
    let p = person(&up, tmp);
    let chosen = overlay_from_catalog(&p, REDIRECTED, "boriken");
    assert!(
        chosen["error"].is_object(),
        "a rewrite in the person's .gitconfig reached a catalog entry's install: {chosen}"
    );
    let typed = overlay(&p, REDIRECTED);
    assert!(typed["result"]["token"].is_string(), "{typed}");
    let _ = fs::remove_dir_all(p.tmp);
}

#[test]
fn an_overlay_chosen_from_a_catalog_never_calls_the_persons_credential_helper_or_agent() {
    let tmp = dir("isolated-chosen-helper");
    let up = upstream(&tmp);
    let p = person(&up, tmp);
    let server = demanding_server();
    let id = git(&up, &["rev-parse", "HEAD"]);
    let chosen = ask(
        &p.control,
        &key("k", &BOTH),
        "overlay/check",
        json!({"name": "o", "repo": server, "ref": id, "path": "", "catalog": "mine"}),
    );
    assert!(chosen["error"].is_object(), "{chosen}");
    assert!(
        !p.helper_log.exists(),
        "the credential helper ran for a catalog entry's install"
    );
    let over_ssh = overlay_from_catalog(&p, "ssh://git@skills.example.invalid/tools.git", "mine");
    assert!(over_ssh["error"].is_object(), "{over_ssh}");
    assert_eq!(
        fs::read_to_string(&p.ssh_agent).expect("environment"),
        "agent=unset\n"
    );
    let bad = ask(
        &p.control,
        &key("k", &BOTH),
        "overlay/check",
        json!({"name": "o", "repo": REDIRECTED, "ref": "main", "catalog": "../x"}),
    );
    assert!(bad["error"].is_object(), "{bad}");
    let _ = fs::remove_dir_all(p.tmp);
}
