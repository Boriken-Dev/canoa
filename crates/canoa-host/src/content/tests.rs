//! The content registry: the schemes the host ships, the table of schemes and its refusals, a scheme and a generator a library
//! adds, and the generator that runs a program.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use canoa::content::{
    CatalogGeneratorProvider, ContentHost, ContentInfo, ContentSourceProvider, EntryKind,
    GenerateRequest, GeneratorInfo, ListAnswer, ListEntry, ListRequest, Listing, OpenAnswer,
    OpenRequest, Opened, RefusalKind, ResolveAnswer, Resolved, SchemeInfo, SourceUse, Stat,
    StatAnswer, Target,
};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::Provider;
use coqui_acp_api::guatiao::library::{Object, ProviderError, Registry};

use super::Content;
use crate::store::http_tests::{Answer, Origin};
use crate::store::sources::Fetching;
use crate::store::tools::HttpTransport;

/// A scheme a library adds: `demo://NAME` is a file holding `NAME`'s text, and its listing has two entries.
#[derive(Default, Provider)]
#[provider(ContentSourceProvider, id = "demo", crate = ::coqui_acp_api::guatiao)]
struct Demo;

impl ContentSourceProvider for Demo {
    fn describe(&self) -> Result<ContentInfo, ProviderError> {
        Ok(ContentInfo::new("demo", "A demonstration").serving(
            SchemeInfo::new("demo", "Demonstration", "demo://name")
                .listing()
                .resolving(),
        ))
    }

    fn stat(&self, target: Target) -> Result<StatAnswer, ProviderError> {
        Ok(StatAnswer::found(
            Stat::file().sized(target.uri.len() as u64),
        ))
    }

    fn list(&self, _request: ListRequest) -> Result<ListAnswer, ProviderError> {
        Ok(ListAnswer::found(Listing::of(vec![
            ListEntry::new("a.txt", EntryKind::File),
            ListEntry::new("../escape", EntryKind::File),
            ListEntry::new("sub", EntryKind::Dir),
        ])))
    }

    fn open(&self, request: OpenRequest) -> Result<OpenAnswer, ProviderError> {
        let name = request
            .target
            .uri
            .strip_prefix("demo://")
            .unwrap_or_default()
            .split('?')
            .next()
            .unwrap_or_default();
        let text = if name == "models" {
            // A catalog in the format of a type that lists models.
            serde_json::json!({"format": "canoa.models/1", "revision": "demo", "items": [{
                "id": "base", "label": "base", "files": [{
                    "url": "https://hub.example/base.bin", "file": "base.bin",
                    "check": {"sha256": "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe"}}]}]})
            .to_string()
        } else if name.ends_with("/id.txt") {
            "base".to_owned()
        } else {
            format!("demo text of {name}")
        };
        Ok(OpenAnswer::found(Opened::inline(
            Stat::file(),
            text.into_bytes(),
        )))
    }

    fn resolve(&self, target: Target) -> Result<ResolveAnswer, ProviderError> {
        Ok(ResolveAnswer::found(Resolved::new(
            format!(
                "{}?ref=abc",
                target.uri.split('?').next().unwrap_or_default()
            ),
            "abc",
            true,
        )))
    }
}

/// A second library's provider that claims `demo` as well.
#[derive(Default, Provider)]
#[provider(ContentSourceProvider, id = "rival", crate = ::coqui_acp_api::guatiao)]
struct Rival;

impl ContentSourceProvider for Rival {
    fn describe(&self) -> Result<ContentInfo, ProviderError> {
        Ok(ContentInfo::new("rival", "A rival")
            .serving(SchemeInfo::new("rival", "Rival", "rival://x"))
            .serving(SchemeInfo::new("demo", "Rival of demo", "demo://x")))
    }

    fn stat(&self, _target: Target) -> Result<StatAnswer, ProviderError> {
        Err(canoa::status::unsupported("nothing"))
    }

    fn list(&self, _request: ListRequest) -> Result<ListAnswer, ProviderError> {
        Err(canoa::status::unsupported("nothing"))
    }

    fn open(&self, _request: OpenRequest) -> Result<OpenAnswer, ProviderError> {
        Err(canoa::status::unsupported("nothing"))
    }

    fn resolve(&self, _target: Target) -> Result<ResolveAnswer, ProviderError> {
        Err(canoa::status::unsupported("nothing"))
    }
}

/// A generator that reads `NAME` of its source (`<source>/id.txt`) through the host and makes a models catalog of one item.
#[derive(Default, Provider)]
#[provider(CatalogGeneratorProvider, id = "one_model", crate = ::coqui_acp_api::guatiao)]
struct OneModel;

impl CatalogGeneratorProvider for OneModel {
    fn describe(&self) -> Result<GeneratorInfo, ProviderError> {
        let mut info = GeneratorInfo::new("one_model", "One model");
        info.types = vec!["whisper".to_owned()];
        Ok(info)
    }

    fn generate(
        &self,
        request: GenerateRequest,
        _spawner: Object<dyn ProcessSpawner>,
        host: Object<dyn ContentHost>,
    ) -> Result<String, ProviderError> {
        let source = request
            .source
            .ok_or_else(|| canoa::status::bad_value("a source is needed"))?;
        let at = host.child(&source, "id.txt")?;
        let name = host.open_text(&at, 1024)?;
        let sha = "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe";
        Ok(serde_json::json!({"format": "canoa.models/1", "items": [{
            "id": name, "label": name, "files": [{
                "url": format!("https://hub.example/{name}.bin"), "file": format!("{name}.bin"),
                "check": {"sha256": sha}}]}]})
        .to_string())
    }
}

/// A generator that tries to read outside its source.
#[derive(Default, Provider)]
#[provider(CatalogGeneratorProvider, id = "wanderer", crate = ::coqui_acp_api::guatiao)]
struct Wanderer;

impl CatalogGeneratorProvider for Wanderer {
    fn describe(&self) -> Result<GeneratorInfo, ProviderError> {
        Ok(GeneratorInfo::new("wanderer", "Reads elsewhere"))
    }

    fn generate(
        &self,
        _request: GenerateRequest,
        _spawner: Object<dyn ProcessSpawner>,
        host: Object<dyn ContentHost>,
    ) -> Result<String, ProviderError> {
        host.open_text("demo://elsewhere/x", 1024)
    }
}

coqui_acp_api::guatiao::local_providers!(
    id = "canoa-standin-content",
    version = "0",
    providers = [Demo, OneModel, Wanderer]
);

mod second {
    use super::Rival;

    coqui_acp_api::guatiao::local_providers!(
        id = "canoa-standin-rival",
        version = "0",
        providers = [Rival]
    );
}

/// The stand-in library, as a registry.
pub(crate) fn demo_registry() -> Arc<Registry> {
    let mut registry = Registry::new("canoa", "0");
    let loading = registry.register_local("demo", library).expect("registers");
    assert!(loading.loaded().is_some());
    Arc::new(registry)
}

fn rival_registry() -> Arc<Registry> {
    let mut registry = Registry::new("canoa", "0");
    let loading = registry
        .register_local("rival", second::library)
        .expect("registers");
    assert!(loading.loaded().is_some());
    Arc::new(registry)
}

/// The built-in providers, and the stand-in library as an extension would add it.
pub(crate) fn with_demo(git_root: Option<&Path>) -> Content {
    let mut content = Content::built_in(Fetching::default(), git_root);
    content.add(Some("demo_ext"), &demo_registry());
    content
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("canoa-content-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a folder");
    dir
}

fn uri_of(path: &Path) -> String {
    let text = path.display().to_string().replace('\\', "/");
    if text.starts_with('/') {
        format!("file://{text}")
    } else {
        format!("file:///{text}")
    }
}

#[test]
fn a_file_is_read_by_a_path_or_a_file_uri_and_listed_and_a_catalog_may_not_name_one() {
    let dir = folder("file");
    std::fs::write(dir.join("a.toml"), "name = \"a\"\n").expect("written");
    std::fs::create_dir_all(dir.join("sub")).expect("folder");
    std::fs::write(dir.join("sub").join("b.toml"), "b").expect("written");
    let content = Content::default();
    let plain = dir.join("a.toml").display().to_string();
    assert_eq!(
        content.open_text(&plain, true, 1024).as_deref(),
        Ok("name = \"a\"\n")
    );
    let uri = uri_of(&dir.join("a.toml"));
    assert_eq!(
        content.open_text(&uri, true, 1024).as_deref(),
        Ok("name = \"a\"\n")
    );
    let stat = content.stat(&uri, true).expect("a stat");
    assert_eq!(stat.size, Some(11));
    assert!(stat.etag.is_some());
    // A listing: one level, then everything below.
    let listed = content.list(&uri_of(&dir), true, 100).expect("listed");
    let names: Vec<&str> = listed
        .entries
        .iter()
        .map(|each| each.path.as_str())
        .collect();
    assert_eq!(names, ["a.toml", "sub"]);
    // A file over what the reader takes is refused with the reason, and one that is not there is not found.
    let long = content.open_text(&plain, true, 4).expect_err("too long");
    assert_eq!(long.kind, RefusalKind::TooLarge);
    let gone = content
        .open_text(&uri_of(&dir.join("nope")), true, 4)
        .expect_err("absent");
    assert_eq!(gone.kind, RefusalKind::NotFound);
    // A URI a catalog document named never reaches this computer's files.
    let named = content
        .open_text(&uri, false, 1024)
        .expect_err("not the person's");
    assert_eq!(named.kind, RefusalKind::RefusedPlace);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_data_uri_is_text_or_base64_with_a_derived_digest() {
    let content = Content::default();
    assert_eq!(
        content
            .open_text("data:text/plain;base64,aGVsbG8=", true, 100)
            .as_deref(),
        Ok("hello")
    );
    assert_eq!(
        content.open_text("data:,a%20b", false, 100).as_deref(),
        Ok("a b")
    );
    let stat = content
        .stat("data:text/plain;base64,aGVsbG8=", true)
        .expect("stat");
    assert!(stat.immutable);
    assert_eq!(stat.digests[0].algorithm, "sha256");
    assert_eq!(
        stat.digests[0].value,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
    assert_eq!(
        stat.digests[0].provenance,
        canoa::content::Provenance::Derived
    );
    assert!(content.open_text("data:nothing", true, 100).is_err());
}

#[test]
fn a_scheme_nobody_serves_and_an_address_with_a_password_are_refused_with_the_reason() {
    let content = Content::default();
    let why = content
        .open_text("hf://owner/name", true, 10)
        .expect_err("nobody");
    assert!(why.why.contains("no provider offers hf:"), "{}", why.why);
    assert!(why.why.contains("https"), "{}", why.why);
    let why = content
        .open_text("https://user:hunter2@host/x", true, 10)
        .expect_err("a password");
    assert!(why.why.contains("holds a password"), "{}", why.why);
    assert!(!why.why.contains("hunter2"), "{}", why.why);
}

#[test]
fn a_library_adds_a_scheme_a_second_claim_is_refused_and_the_rest_of_its_library_loads() {
    let mut content = with_demo(None);
    assert_eq!(
        content.open_text("demo://x", true, 100).as_deref(),
        Ok("demo text of x")
    );
    let listed = content.list("demo://x", true, 10).expect("listed");
    // A name that is not one path component is left out by the host.
    let names: Vec<&str> = listed
        .entries
        .iter()
        .map(|each| each.path.as_str())
        .collect();
    assert_eq!(names, ["a.txt", "sub"]);
    let fixed = content.resolve("demo://x", true).expect("resolved");
    assert_eq!((fixed.revision.as_str(), fixed.immutable), ("abc", true));

    content.add(Some("rival_ext"), &rival_registry());
    let refused = content.refused();
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert_eq!(refused[0].id, "rival");
    assert!(
        refused[0].why.contains("demo is already offered by demo"),
        "{}",
        refused[0].why
    );
    // The first stays.
    assert_eq!(
        content.open_text("demo://y", true, 100).as_deref(),
        Ok("demo text of y")
    );
    let rows = content.schemes();
    let demo: Vec<_> = rows.iter().filter(|row| row.scheme == "demo").collect();
    assert_eq!(demo.len(), 1);
    assert_eq!(demo[0].extension.as_deref(), Some("demo_ext"));
    for built_in in [
        "file",
        "https",
        "http",
        "data",
        "git+https",
        "git+ssh",
        "git+file",
    ] {
        assert!(rows.iter().any(|row| row.scheme == built_in), "{built_in}");
    }
}

#[test]
fn a_provider_the_config_turns_off_is_listed_and_refuses_its_schemes() {
    let content = with_demo(None);
    let _off = crate::disabled::naming(crate::disabled::Disabled::new([(
        "content_source".to_owned(),
        vec!["demo".to_owned()],
    )]));
    let why = content.open_text("demo://x", true, 100).expect_err("off");
    assert!(why.why.contains("turns off"), "{}", why.why);
    assert!(
        content
            .schemes()
            .iter()
            .any(|row| row.scheme == "demo" && !row.on)
    );
    // The built-in ones still read.
    assert!(content.open_text("data:,x", true, 10).is_ok());
}

#[test]
fn a_generator_reads_through_the_host_only_under_its_source_and_makes_the_catalog() {
    let content = with_demo(None);
    let request = GenerateRequest::new("whisper", "mine", Some("demo://root".to_owned()), "{}");
    let text = content.generate("one_model", request).expect("generated");
    let made: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    // `demo://root/id.txt` is what the generator read, through the host, under its source.
    assert_eq!(made["items"][0]["id"], "base");

    // A place the entry's source does not name is refused, with the place named.
    let wandering = GenerateRequest::new("whisper", "mine", Some("demo://root".to_owned()), "{}");
    let why = content
        .generate("wanderer", wandering)
        .expect_err("elsewhere");
    assert!(why.contains("not a place this generator reads"), "{why}");

    // A generator for another type, one that does not exist, and a missing source are each refused.
    let other = GenerateRequest::new("piper", "mine", Some("demo://root".to_owned()), "{}");
    assert!(
        content
            .generate("one_model", other)
            .expect_err("another type")
            .contains("does not serve")
    );
    let none = GenerateRequest::new("whisper", "mine", None, "{}");
    assert!(
        content
            .generate("one_model", none)
            .expect_err("no source")
            .contains("reads a source")
    );
    let unknown = GenerateRequest::new("whisper", "mine", None, "{}");
    let why = content.generate("nobody", unknown).expect_err("unknown");
    assert!(why.contains("no catalog generator"), "{why}");
    assert!(why.contains("command"), "{why}");
}

#[test]
fn the_command_generator_answers_with_what_the_program_prints_and_says_why_it_failed() {
    let content = Content::default();
    let said = |fields: serde_json::Value| {
        content.generate(
            "command",
            GenerateRequest::new("overlays", "made", None, fields.to_string()),
        )
    };
    let printed = said(serde_json::json!({"command": ["git", "--version"]})).expect("printed");
    assert!(printed.starts_with("git version"), "{printed}");
    let failed =
        said(serde_json::json!({"command": ["git", "--no-such-flag"]})).expect_err("fails");
    assert!(failed.contains("exit code"), "{failed}");
    let none = said(serde_json::json!({"command": []})).expect_err("no program");
    assert!(none.contains("a program"), "{none}");
    let missing =
        said(serde_json::json!({"command": ["no-such-program-canoa"]})).expect_err("not found");
    assert!(missing.contains("could not start"), "{missing}");
    // It reads no source.
    let with_source = content
        .generate(
            "command",
            GenerateRequest::new(
                "overlays",
                "made",
                Some("https://x/".to_owned()),
                "{\"command\": [\"git\"]}",
            ),
        )
        .expect_err("a source");
    assert!(with_source.contains("reads no source"), "{with_source}");
    assert_eq!(
        content
            .generators()
            .find(|(info, _)| info.id == "command")
            .map(|(info, _)| info.source),
        Some(SourceUse::Absent)
    );
}

fn plain_fetching() -> Fetching {
    let network = Arc::new(canoa::network::Network::with(
        crate::network::tests::direct(),
        coqui_syshttp::tls::TlsBuilder::default(),
    ));
    Fetching::through(
        Arc::new(HttpTransport::on(network)),
        Duration::from_secs(5),
        Duration::from_secs(5),
    )
}

#[test]
fn an_address_is_read_with_its_etag_revalidated_so_an_unchanged_document_costs_no_body() {
    let origin = Origin::start(|asked| {
        if asked.if_none_match.as_deref() == Some("\"v1\"") {
            Answer {
                status: 304,
                etag: Some("\"v1\"".to_owned()),
                body: Vec::new(),
                delay: Duration::ZERO,
            }
        } else {
            Answer {
                status: 200,
                etag: Some("\"v1\"".to_owned()),
                body: b"catalog text".to_vec(),
                delay: Duration::ZERO,
            }
        }
    });
    let content = Content::built_in(plain_fetching(), None);
    let url = format!("{}/c.json", origin.url);
    assert_eq!(
        content.open_text(&url, true, 1024).as_deref(),
        Ok("catalog text")
    );
    assert_eq!(
        content.open_text(&url, true, 1024).as_deref(),
        Ok("catalog text")
    );
    let seen = origin.seen.lock().expect("lock");
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1].if_none_match.as_deref(), Some("\"v1\""));
    assert_eq!(
        origin.bytes.load(std::sync::atomic::Ordering::SeqCst),
        "catalog text".len(),
        "the body came once"
    );
}

#[test]
fn a_redirect_off_the_addresss_own_origin_is_refused() {
    let elsewhere = Origin::start(|_| Answer {
        status: 200,
        etag: None,
        body: b"secret".to_vec(),
        delay: Duration::ZERO,
    });
    let target = elsewhere.url.clone();
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let url = format!(
        "http://127.0.0.1:{}/c.json",
        listener.local_addr().expect("addr").port()
    );
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer);
            let reply = format!(
                "HTTP/1.1 302 Found\r\nLocation: {target}/c.json\r\nContent-Length: 0\r\n\r\n"
            );
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    let content = Content::built_in(plain_fetching(), None);
    let refused = content
        .open_text(&url, true, 1024)
        .expect_err("off the origin");
    assert_eq!(refused.kind, RefusalKind::RefusedPlace, "{}", refused.why);
    assert_eq!(
        elsewhere.requests.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the other origin was never asked"
    );
}

#[test]
fn an_address_that_answers_with_an_error_or_too_much_says_which() {
    let origin = Origin::start(|asked| {
        if asked.path == "/missing" {
            Answer {
                status: 404,
                etag: None,
                body: Vec::new(),
                delay: Duration::ZERO,
            }
        } else {
            Answer {
                status: 200,
                etag: None,
                body: vec![b'x'; 2048],
                delay: Duration::ZERO,
            }
        }
    });
    let content = Content::built_in(plain_fetching(), None);
    let missing = content
        .open_text(&format!("{}/missing", origin.url), true, 1024)
        .expect_err("404");
    assert_eq!(missing.kind, RefusalKind::NotFound, "{}", missing.why);
    let big = content
        .open_text(&format!("{}/big", origin.url), true, 1024)
        .expect_err("big");
    assert_eq!(big.kind, RefusalKind::TooLarge, "{}", big.why);
    // A web address says nothing of itself until it is read, and lists nothing.
    assert!(content.stat(&format!("{}/big", origin.url), true).is_err());
    assert!(content.list(&format!("{}/", origin.url), true, 10).is_err());
}

/// `git` with the identity a test commit needs and none of the person's configuration.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=tester",
            "-c",
            "user.email=tester@example.test",
        ])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

#[test]
fn a_git_repository_is_read_at_a_branch_a_commit_and_its_default_and_listed() {
    let dir = folder("git");
    let work = dir.join("work");
    std::fs::create_dir_all(work.join("sub")).expect("folders");
    git(&work, &["init", "-q", "-b", "main"]);
    std::fs::write(work.join("catalog.toml"), "name = \"one\"\n").expect("written");
    std::fs::write(work.join("sub").join("deep.txt"), "deep").expect("written");
    git(&work, &["add", "."]);
    git(&work, &["commit", "-q", "-m", "first"]);
    let first = git(&work, &["rev-parse", "HEAD"]);

    let content = Content::built_in(Fetching::default(), Some(&dir.join("content")));
    let repo = format!("git+{}", uri_of(&work));
    let main = format!("{repo}!/catalog.toml?ref=main");
    assert_eq!(
        content.open_text(&main, true, 1024).as_deref(),
        Ok("name = \"one\"\n")
    );
    // With no ref it is the default branch, and as a catalog document named it (the isolated git) it reads the same.
    let default = format!("{repo}!/catalog.toml");
    assert_eq!(
        content.open_text(&default, false, 1024).as_deref(),
        Ok("name = \"one\"\n")
    );

    let stat = content.stat(&main, true).expect("a stat");
    assert_eq!(stat.revision.as_deref(), Some(first.as_str()));
    assert_eq!(stat.digests[0].algorithm, "git_blob");
    assert!(!stat.immutable, "a branch moves");
    let pinned = format!("{repo}!/catalog.toml?ref={first}");
    assert!(content.stat(&pinned, true).expect("pinned").immutable);

    let listed = content.list(&repo, true, 100).expect("listed");
    let names: Vec<(&str, EntryKind)> = listed
        .entries
        .iter()
        .map(|each| (each.path.as_str(), each.kind))
        .collect();
    assert_eq!(
        names,
        [("catalog.toml", EntryKind::File), ("sub", EntryKind::Dir)]
    );
    assert_eq!(
        content
            .list(&format!("{repo}!/sub"), true, 100)
            .expect("sub")
            .entries[0]
            .path,
        "deep.txt"
    );

    // The branch moves on: what is read follows it, and a commit id is still the first.
    std::fs::write(work.join("catalog.toml"), "name = \"two\"\n").expect("written");
    git(&work, &["commit", "-q", "-am", "second"]);
    assert_eq!(
        content.open_text(&main, true, 1024).as_deref(),
        Ok("name = \"two\"\n")
    );
    assert_eq!(
        content.open_text(&pinned, true, 1024).as_deref(),
        Ok("name = \"one\"\n")
    );
    let resolved = content.resolve(&main, true).expect("resolved");
    assert_eq!(resolved.revision, git(&work, &["rev-parse", "HEAD"]));
    assert!(resolved.immutable);
    assert!(
        resolved
            .uri
            .ends_with(&format!("?ref={}", resolved.revision)),
        "{}",
        resolved.uri
    );

    // Absent things are not found.
    let missing = content
        .open_text(&format!("{repo}!/nope.toml?ref=main"), true, 10)
        .expect_err("absent");
    assert_eq!(missing.kind, RefusalKind::NotFound, "{}", missing.why);
    let no_ref = content
        .open_text(&format!("{repo}!/catalog.toml?ref=nobranch"), true, 10)
        .expect_err("no such branch");
    assert_eq!(no_ref.kind, RefusalKind::NotFound, "{}", no_ref.why);
    // A file over the limit is refused before it is read.
    let long = content.open_text(&main, true, 3).expect_err("long");
    assert_eq!(long.kind, RefusalKind::TooLarge);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_git_address_with_no_folder_to_keep_repositories_says_so() {
    let content = Content::built_in(Fetching::default(), None);
    let why = content
        .open_text("git+https://example.test/o/r.git!/c.toml", true, 10)
        .expect_err("no folder");
    assert!(why.why.contains("no folder"), "{}", why.why);
}
