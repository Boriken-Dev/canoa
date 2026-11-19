//! `extension/*` against a local catalog repository: what the plan shows, what is kept only once what was fetched has the digest the
//! catalog gave, and what is left behind when it has not. Nothing here reaches the network: a URL is served from memory.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::extension_install::ExtensionInstall;
use super::overlays_tests::{ask, change, dir, key, repo};
use super::*;
use crate::store::index::StoreIndex;
use crate::store::tools::{Reply, ToolsLimits, Transport};

const LIBRARY: &str = "library bytes the host loads";
const SCRIPT_MANIFEST: &str =
    "type = \"js\"\nname = \"notes_extension\"\nkinds = [\"store_extension\"]\npaths = [\".\"]\n";
const SCRIPT_PACKAGE: &str = "{\"name\": \"notes_extension\", \"version\": \"1.0.0\", \"type\": \"module\", \"exports\": \"./notes.js\"}\n";

/// A transport that serves fixed bytes by URL, and says 404 to any other.
#[derive(Debug)]
struct Serves(BTreeMap<String, Vec<u8>>);

impl Transport for Serves {
    fn get(&self, url: &str, _limits: &ToolsLimits) -> Result<Reply, String> {
        Ok(match self.0.get(url) {
            Some(bytes) => Reply {
                status: 200,
                location: None,
                body: Box::new(Cursor::new(bytes.clone())),
            },
            None => Reply {
                status: 404,
                location: None,
                body: Box::new(Cursor::new(Vec::new())),
            },
        })
    }
}

fn url(repo: &Path) -> String {
    let text = repo.to_string_lossy().replace('\\', "/");
    if text.starts_with('/') {
        format!("file://{text}")
    } else {
        format!("file:///{text}")
    }
}

fn sha256_of(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(bytes))
}

fn tree_digest(folder: &Path) -> String {
    coqui_aos::tree_digest(
        &coqui_aos::StdFs,
        folder,
        &super::extension_install::tree_settings(),
    )
    .expect("a digest")
    .digest
    .to_string()
}

struct Rig {
    tmp: PathBuf,
    control: Control,
    home: PathBuf,
    config: PathBuf,
    /// The pin of the `pack` folder of the catalog.
    pin: String,
    /// The pin of its `scripts` folder, a package of scripts.
    scripts_pin: String,
}

fn rig(served: &[(&str, &[u8])]) -> Rig {
    rig_with(served, LIBRARY)
}

/// A rig whose `pack` extension holds `library` as `lib.bin`.
fn rig_with(served: &[(&str, &[u8])], library: &str) -> Rig {
    let tmp = dir("extensions");
    let upstream = repo(
        &tmp,
        "catalog",
        &[
            ("ext/pack/lib.bin", library),
            ("ext/pack/notes/readme.txt", "what it is"),
            ("ext/scripts/notes.guatiao", SCRIPT_MANIFEST),
            ("ext/scripts/package.json", SCRIPT_PACKAGE),
            ("ext/scripts/notes.js", "export const notes = 1;\n"),
            ("ext/bare/readme.txt", "no manifest here"),
        ],
    );
    let pin = tree_digest(&upstream.join("ext").join("pack"));
    let scripts_pin = tree_digest(&upstream.join("ext").join("scripts"));
    let bare_pin = tree_digest(&upstream.join("ext").join("bare"));
    let wrong = format!("sha256:{}", "ab".repeat(32));
    let catalog = format!(
        "[extensions.pack]\nsource = \"./ext/pack\"\npin = \"{pin}\"\nload = \"lib.bin\"\nprovider = \"p\"\n\n\
         [extensions.liar]\nsource = \"./ext/pack\"\npin = \"{wrong}\"\nload = \"lib.bin\"\nprovider = \"p\"\n\n\
         [extensions.noload]\nsource = \"./ext/pack\"\npin = \"{pin}\"\nprovider = \"p\"\n\n\
         [extensions.elsewhere]\nsource = \"./ext/pack\"\npin = \"{pin}\"\nload = \"lib.bin\"\nprovider = \"p\"\n\
         needs = {{ platforms = [\"plan9\"] }}\n\n\
         [extensions.viewer]\nsource = \"./ext/pack\"\npin = \"{pin}\"\nload = \"lib.bin\"\nprovider = \"p\"\n\
         needs = {{ kind = \"viewer\", version = \"1\" }}\n\n\
         [extensions.file]\nsource = \"https://downloads.example/files/file.lib\"\nsha256 = \"{}\"\nprovider = \"q\"\n\n\
         [extensions.tampered]\nsource = \"https://downloads.example/files/tampered.lib\"\nsha256 = \"{}\"\nprovider = \"q\"\n\n\
         [extensions.abs]\nsource = \"{}#ext/pack\"\nref = \"main\"\npin = \"{pin}\"\nload = \"lib.bin\"\nprovider = \"p\"\n\n\
         [extensions.plain]\nsource = \"http://downloads.example/files/plain.lib\"\nsha256 = \"{}\"\nprovider = \"q\"\n\n\
         [extensions.signed]\nsource = \"https://user:pw@downloads.example/files/signed.lib?sig=1\"\nsha256 = \"{}\"\nprovider = \"q\"\n\n\
         [extensions.scripts]\nsource = \"./ext/scripts\"\npin = \"{scripts_pin}\"\nruntime = \"js\"\nprovider = \"notes_extension_notes\"\n\n\
         [extensions.scripts_py]\nsource = \"./ext/scripts\"\npin = \"{scripts_pin}\"\nruntime = \"python\"\nprovider = \"p\"\n\n\
         [extensions.bare]\nsource = \"./ext/bare\"\npin = \"{bare_pin}\"\nruntime = \"js\"\nprovider = \"p\"\n",
        sha256_of(b"file bytes"),
        sha256_of(b"the bytes the catalog meant"),
        url(&upstream),
        sha256_of(b"plain bytes"),
        sha256_of(b"signed bytes"),
    );
    change(&upstream, &[("extensions.toml", &catalog)], "the catalog");
    let config = tmp.join("cfg").join("canoa.cfg");
    std::fs::create_dir_all(config.parent().expect("parent")).expect("dir");
    std::fs::write(
        &config,
        format!(
            "[store.extensions.catalogs]\nteam = \"{}@main#extensions.toml\"\n",
            // The catalog is a folder of this computer, named by its path as the person writes it.
            upstream.to_string_lossy().replace('\\', "/")
        ),
    )
    .expect("config");
    let home = tmp.join("home").join("extensions");
    let transport = Serves(
        served
            .iter()
            .map(|(url, bytes)| ((*url).to_owned(), bytes.to_vec()))
            .collect(),
    );
    let control = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_files(Files::new(
        None,
        Vec::new(),
        Some(config.clone()),
        None,
        UploadLimits::default(),
    ))
    .with_overlays(Overlays::new(
        Some(tmp.join("cfg").join("overlays")),
        OverlayLimits::default(),
    ))
    .with_index(StoreIndex::new(
        Some(tmp.join("home").join("cache").join("store")),
        Duration::from_secs(3600),
        1 << 20,
    ))
    .with_extensions(
        ExtensionInstall::new(Some(home.clone())).with_transport(Arc::new(transport), false),
    );
    Rig {
        tmp,
        control,
        home,
        config,
        pin,
        scripts_pin,
    }
}

impl Rig {
    fn plan(&self, name: &str) -> Value {
        ask(
            &self.control,
            &Caller::Owner,
            "extension/plan",
            json!({"catalog": "team", "name": name}),
        )
    }

    fn apply(&self, plan: &Value) -> Value {
        ask(
            &self.control,
            &Caller::Owner,
            "extension/apply",
            json!({"plan": plan["result"]}),
        )
    }

    fn host(&self) -> Value {
        ask(
            &self.control,
            &Caller::Owner,
            "config/get",
            json!({"section": "host"}),
        )["result"]["value"]
            .clone()
    }

    /// What is in the extensions folder, by name.
    fn kept(&self) -> Vec<String> {
        std::fs::read_dir(&self.home)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[test]
fn a_plan_names_the_source_and_the_digest_and_fetches_nothing() {
    let rig = rig(&[]);
    let plan = rig.plan("pack");
    let result = &plan["result"];
    assert_eq!(result["action"], "install", "{plan}");
    assert_eq!(
        result["downloads"][0]["check"],
        json!({"kind": "sha256", "value": rig.pin.trim_start_matches("sha256:")}),
        "{plan}"
    );
    assert!(
        result["downloads"][0]["url"]
            .as_str()
            .is_some_and(|url| url.contains("catalog") && url.ends_with("#ext/pack")),
        "the source is shown as the catalog gave it: {plan}"
    );
    assert!(
        result["token"]
            .as_str()
            .is_some_and(|token| token.len() == 64)
    );
    let notices = result["notices"].to_string();
    assert!(
        notices.contains("runs with your access") && notices.contains("provider p"),
        "{notices}"
    );
    assert!(
        result["changes"]
            .to_string()
            .contains("[host.extensions.pack]")
    );
    assert!(
        rig.kept().is_empty(),
        "a plan keeps nothing: {:?}",
        rig.kept()
    );
    assert_eq!(rig.host(), Value::Null, "and writes no config");
    let _ = std::fs::remove_dir_all(&rig.tmp);
}

#[test]
fn only_the_hosting_grant_plans_installs_or_removes() {
    let rig = rig(&[]);
    let reader = key(
        "sha256:aa",
        &[
            Grant::OverlayRead,
            Grant::OverlayWrite,
            Grant::StorePrograms,
        ],
    );
    for (method, params) in [
        ("extension/plan", json!({"catalog": "team", "name": "pack"})),
        ("extension/apply", json!({"plan": {}})),
        ("extension/remove", json!({"name": "pack"})),
    ] {
        let refused = ask(&rig.control, &reader, method, params);
        assert_eq!(refused["error"]["code"], DENIED, "{method}: {refused}");
    }
    let hosting = key("sha256:bb", &[Grant::Hosting]);
    let allowed = ask(
        &rig.control,
        &hosting,
        "extension/plan",
        json!({"catalog": "team", "name": "pack"}),
    );
    assert!(allowed["result"]["token"].is_string(), "{allowed}");
    let _ = std::fs::remove_dir_all(&rig.tmp);
}

#[test]
fn a_tree_is_kept_and_named_in_canoa_cfg_once_it_has_the_digest_and_removed_with_its_files() {
    let rig = rig(&[]);
    let applied = rig.apply(&rig.plan("pack"));
    assert_eq!(applied["result"]["installed"], true, "{applied}");
    assert_eq!(applied["result"]["restart"], true);
    let folder = rig.home.join("pack");
    assert_eq!(
        std::fs::read_to_string(folder.join("lib.bin")).expect("the library"),
        LIBRARY
    );
    assert!(folder.join("notes").join("readme.txt").is_file());
    assert_eq!(
        tree_digest(&folder),
        rig.pin,
        "what is kept is what was pinned"
    );
    let host = rig.host();
    let table = &host["extensions"]["pack"];
    assert_eq!(table["provider"], "p", "{host}");
    assert_eq!(table["pin"], rig.pin, "{host}");
    assert_eq!(
        PathBuf::from(table["library"].as_str().expect("a library")),
        folder.join("lib.bin")
    );
    assert_eq!(
        PathBuf::from(table["root"].as_str().expect("a root")),
        folder
    );
    assert!(
        std::fs::read_to_string(&rig.config)
            .expect("canoa.cfg")
            .contains("[host.extensions.pack]"),
        "the host wrote it"
    );

    // A second plan says what it would replace, in canoa.cfg and in the folder.
    let again = rig.plan("pack")["result"]["notices"].to_string();
    assert!(
        again.contains("already names an extension pack") && again.contains("this replaces it"),
        "{again}"
    );
    assert!(again.contains("its files are replaced"), "{again}");

    let removed = ask(
        &rig.control,
        &Caller::Owner,
        "extension/remove",
        json!({"name": "pack"}),
    );
    assert_eq!(removed["result"]["removed"], true, "{removed}");
    assert_eq!(removed["result"]["files_removed"], true, "{removed}");
    assert!(!folder.exists(), "its files went with it");
    assert_eq!(rig.host(), Value::Null, "and the empty table");
    let again = ask(
        &rig.control,
        &Caller::Owner,
        "extension/remove",
        json!({"name": "pack"}),
    );
    assert_eq!(again["error"]["code"], REFUSED, "{again}");
    let _ = std::fs::remove_dir_all(&rig.tmp);
}

#[test]
fn an_extension_library_larger_than_an_overlay_may_be_is_installed_and_loads() {
    use crate::config::{Extension, ExtensionCheck, Origin};
    // A native library is often far over what an overlay may hold, in the file and in the tree.
    let library =
        "x".repeat(usize::try_from(super::overlays::DEFAULT_MAX_BYTES).expect("fits") + 1);
    let rig = rig_with(&[], &library);
    let applied = rig.apply(&rig.plan("pack"));
    assert_eq!(applied["result"]["installed"], true, "{applied}");
    let folder = rig.home.join("pack");
    assert_eq!(
        tree_digest(&folder),
        rig.pin,
        "what is kept is what was pinned"
    );
    let kept = Extension {
        name: "pack".to_owned(),
        origin: Origin::Library(folder.join("lib.bin")),
        provider: "p".to_owned(),
        check: Some(ExtensionCheck::Tree {
            root: folder,
            pin: rig.pin.clone(),
        }),
    };
    assert_eq!(super::stores::verify_extension(&kept), Ok(()));
    let _ = std::fs::remove_dir_all(&rig.tmp);
}

#[test]
fn a_tree_without_the_digest_the_catalog_gave_leaves_nothing_behind() {
    let rig = rig(&[]);
    let applied = rig.apply(&rig.plan("liar"));
    let why = applied["error"]["message"].as_str().expect("a refusal");
    assert!(
        why.contains("do not have the tree digest") && why.contains(&rig.pin),
        "{applied}"
    );
    assert!(
        rig.kept().is_empty(),
        "no folder and no staging is left: {:?}",
        rig.kept()
    );
    assert_eq!(rig.host(), Value::Null, "and canoa.cfg is as it was");
    let _ = std::fs::remove_dir_all(&rig.tmp);
}

#[test]
fn a_url_is_kept_by_its_sha256_and_a_file_that_differs_is_not() {
    let rig = rig(&[
        (
            "https://downloads.example/files/file.lib",
            &b"file bytes"[..],
        ),
        (
            "https://downloads.example/files/tampered.lib",
            &b"bytes someone else made"[..],
        ),
    ]);
    let applied = rig.apply(&rig.plan("file"));
    assert_eq!(applied["result"]["installed"], true, "{applied}");
    let library = rig.home.join("file").join("file.lib");
    assert_eq!(std::fs::read(&library).expect("the file"), b"file bytes");
    let host = rig.host();
    assert_eq!(
        host["extensions"]["file"]["sha256"],
        sha256_of(b"file bytes"),
        "{host}"
    );
    assert!(host["extensions"]["file"].get("pin").is_none());

    let refused = rig.apply(&rig.plan("tampered"));
    assert_eq!(refused["error"]["code"], REFUSED, "{refused}");
    assert!(
        refused["error"]["message"]
            .as_str()
            .is_some_and(|why| why.contains("checksum mismatch")),
        "{refused}"
    );
    assert_eq!(rig.kept(), ["file"], "only the one that matched is kept");
    assert!(
        rig.host()["extensions"].get("tampered").is_none(),
        "{}",
        rig.host()
    );
    let _ = std::fs::remove_dir_all(&rig.tmp);
}

#[test]
fn an_entry_this_host_cannot_use_is_refused_at_the_plan_in_words() {
    let rig = rig(&[]);
    for (name, says) in [
        ("noload", "names none (`load`)"),
        ("elsewhere", "does not run on"),
        ("viewer", "does not load that kind"),
        ("absent", "no extensions catalog of this host lists absent"),
    ] {
        let refused = rig.plan(name);
        assert!(
            refused["error"]["message"]
                .as_str()
                .is_some_and(|why| why.contains(says)),
            "{name}: {refused}"
        );
    }
    let other = ask(
        &rig.control,
        &Caller::Owner,
        "extension/plan",
        json!({"catalog": "mine", "name": "pack"}),
    );
    assert_eq!(
        other["error"]["code"], REFUSED,
        "a catalog it was not told to use: {other}"
    );
    let _ = std::fs::remove_dir_all(&rig.tmp);
}

#[test]
fn a_plan_that_is_not_the_one_the_host_made_is_not_applied() {
    let rig = rig(&[]);
    let mut plan = rig.plan("pack");
    plan["result"]["token"] = json!("0".repeat(64));
    let refused = rig.apply(&plan);
    assert!(
        refused["error"]["message"]
            .as_str()
            .is_some_and(|why| why.contains("plan again")),
        "{refused}"
    );
    // A client that names another extension's plan under this one's name is refused the same way.
    let mut crossed = rig.plan("liar");
    crossed["result"]["extension"]["name"] = json!("pack");
    assert!(rig.apply(&crossed)["error"].is_object());
    assert!(rig.kept().is_empty(), "{:?}", rig.kept());
    let _ = std::fs::remove_dir_all(&rig.tmp);
}

#[test]
fn an_installed_extension_is_checked_again_when_the_host_loads_it() {
    use crate::config::{Extension, ExtensionCheck, Origin};
    let tmp = dir("verify");
    let folder = tmp.join("kit");
    std::fs::create_dir_all(&folder).expect("a folder");
    let library = folder.join("lib.bin");
    std::fs::write(&library, LIBRARY).expect("a library");
    let kit = |check: ExtensionCheck| Extension {
        name: "kit".to_owned(),
        origin: Origin::Library(library.clone()),
        provider: "p".to_owned(),
        check: Some(check),
    };
    let file = kit(ExtensionCheck::File {
        sha256: sha256_of(LIBRARY.as_bytes()),
    });
    let tree = kit(ExtensionCheck::Tree {
        root: folder.clone(),
        pin: tree_digest(&folder),
    });
    assert_eq!(super::stores::verify_extension(&file), Ok(()));
    assert_eq!(super::stores::verify_extension(&tree), Ok(()));
    std::fs::write(&library, "a library someone changed").expect("changed");
    assert!(
        super::stores::verify_extension(&file)
            .expect_err("changed")
            .contains("no longer has the SHA-256")
    );
    assert!(
        super::stores::verify_extension(&tree)
            .expect_err("changed")
            .contains("no longer has the tree digest")
    );
    let outside = Extension {
        origin: Origin::Library(tmp.join("other.bin")),
        ..tree.clone()
    };
    assert!(
        super::stores::verify_extension(&outside)
            .expect_err("outside")
            .contains("not inside the folder")
    );
    let unchecked = Extension {
        check: None,
        ..tree
    };
    assert_eq!(
        super::stores::verify_extension(&unchecked),
        Ok(()),
        "one named by hand has none"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn the_catalogs_are_listed_to_a_key_that_reads_overlays_and_none_until_the_table_names_one() {
    let rig = rig(&[]);
    let reader = key("sha256:aa", &[Grant::OverlayRead]);
    let listed = ask(&rig.control, &reader, "extension/catalogs", json!({}));
    let result = &listed["result"];
    assert_eq!(
        result["sources"][0]["kind"], "extension_catalog",
        "{listed}"
    );
    assert_eq!(result["sources"][0]["id"], "extensions:team", "{listed}");
    assert_eq!(result["sources"][0]["label"], "team");
    let entries = result["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 13, "{listed}");
    let pack = entries
        .iter()
        .find(|entry| entry["name"] == "pack")
        .expect("pack");
    assert_eq!(pack["kind"], "extension");
    assert_eq!(pack["catalog"], "team");
    assert_eq!(pack["extension"]["pin"], rig.pin);
    assert_eq!(pack["extension"]["provider"], "p");
    assert_eq!(pack["origin"]["path"], "ext/pack");
    assert!(
        entries.iter().all(|entry| entry["kind"] == "extension"),
        "only extensions: {listed}"
    );

    let hosting = key("sha256:bb", &[Grant::Hosting]);
    let refused = ask(&rig.control, &hosting, "extension/catalogs", json!({}));
    assert_eq!(refused["error"]["code"], DENIED, "{refused}");
    let forced = ask(
        &rig.control,
        &reader,
        "extension/catalogs",
        json!({"refresh": true}),
    );
    assert_eq!(
        forced["error"]["code"], DENIED,
        "a fetch is the writer's: {forced}"
    );

    // The overlay catalogs are a table of their own: naming the same file there lists no extension.
    let text = std::fs::read_to_string(&rig.config).expect("canoa.cfg");
    let overlay_only = text.replace("[store.extensions.catalogs]", "[store.overlays.catalogs]");
    std::fs::write(&rig.config, overlay_only).expect("written");
    let none = ask(
        &rig.control,
        &Caller::Owner,
        "extension/catalogs",
        json!({}),
    );
    assert_eq!(none["result"]["sources"], json!([]), "{none}");
    assert_eq!(none["result"]["entries"], json!([]), "{none}");
    let _ = std::fs::remove_dir_all(&rig.tmp);
}

#[test]
fn nothing_the_person_chose_to_reach_is_refused_a_local_catalog_a_local_source_http_and_a_signed_url()
 {
    let rig = rig(&[
        (
            "http://downloads.example/files/plain.lib",
            &b"plain bytes"[..],
        ),
        (
            "https://user:pw@downloads.example/files/signed.lib?sig=1",
            &b"signed bytes"[..],
        ),
    ]);
    // The catalog is a folder named by its path, and a folder of the same repository is fetched as the catalog is.
    let own = rig.plan("pack")["result"]["notices"].to_string();
    assert!(own.contains("with your own git settings"), "{own}");
    // A source the catalog names itself, here a `file:` URL, is fetched with none of the person's settings, and works.
    let named = rig.plan("abs");
    let notices = named["result"]["notices"].to_string();
    assert!(notices.contains("none of your git settings"), "{notices}");
    assert_eq!(rig.apply(&named)["result"]["installed"], true);
    assert_eq!(
        std::fs::read_to_string(rig.home.join("abs").join("lib.bin")).expect("the library"),
        LIBRARY
    );
    for name in ["plain", "signed"] {
        let applied = rig.apply(&rig.plan(name));
        assert_eq!(applied["result"]["installed"], true, "{name}: {applied}");
    }
    assert_eq!(
        std::fs::read(rig.home.join("signed").join("signed.lib")).expect("the file"),
        b"signed bytes"
    );
    let mut kept = rig.kept();
    kept.sort();
    assert_eq!(kept, ["abs", "plain", "signed"]);
    let _ = std::fs::remove_dir_all(&rig.tmp);
}

#[test]
fn a_package_of_scripts_is_kept_as_its_folder_and_named_in_canoa_cfg_by_the_source_the_host_carries()
 {
    let rig = rig(&[]);
    let plan = rig.plan("scripts");
    let notices = plan["result"]["notices"].to_string();
    assert!(
        notices.contains("package of scripts") && notices.contains("every platform"),
        "{notices}"
    );
    assert!(
        plan["result"]["changes"]
            .to_string()
            .contains("package's folder")
    );
    let applied = rig.apply(&plan);
    assert_eq!(applied["result"]["installed"], true, "{applied}");
    let folder = rig.home.join("scripts");
    assert!(folder.join("notes.guatiao").is_file() && folder.join("notes.js").is_file());
    assert_eq!(
        tree_digest(&folder),
        rig.scripts_pin,
        "what is kept is what was pinned"
    );
    let host = rig.host();
    let table = &host["extensions"]["scripts"];
    assert_eq!(table["source"], "bundled:js", "{host}");
    assert_eq!(
        table["library"],
        Value::Null,
        "a package is no library file: {host}"
    );
    assert_eq!(table["provider"], "notes_extension_notes");
    assert_eq!(table["pin"], rig.scripts_pin);
    assert_eq!(
        PathBuf::from(table["root"].as_str().expect("a root")),
        folder
    );
    assert_eq!(
        PathBuf::from(
            table["source_settings"]["dirs"][0]
                .as_str()
                .expect("a folder")
        ),
        folder
    );
    // The host reads what it wrote back as a source whose folder it checks against the pin before loading anything.
    let loaded = crate::config::Config::load(&rig.config).expect("canoa.cfg reads");
    let extension = loaded
        .extensions
        .iter()
        .find(|each| each.name == "scripts")
        .expect("named");
    assert!(matches!(
        extension.origin,
        crate::config::Origin::Source { .. }
    ));
    assert_eq!(super::stores::verify_extension(extension), Ok(()));
    std::fs::write(folder.join("notes.js"), "export const notes = 2;\n").expect("changed");
    assert!(
        super::stores::verify_extension(extension).is_err_and(|why| why.contains("tree digest")),
        "a script that changed after it was installed is not loaded"
    );
    let removed = ask(
        &rig.control,
        &Caller::Owner,
        "extension/remove",
        json!({"name": "scripts"}),
    );
    assert_eq!(removed["result"]["files_removed"], true, "{removed}");
    assert!(!folder.exists());
    assert_eq!(rig.host(), Value::Null);
    let _ = std::fs::remove_dir_all(&rig.tmp);
}

#[test]
fn a_package_in_python_or_with_no_manifest_is_refused_with_its_reason() {
    let rig = rig(&[]);
    let python = rig.plan("scripts_py");
    assert!(
        python["error"]["message"]
            .as_str()
            .is_some_and(|why| why.contains("Python") && why.contains("carries no Python")),
        "{python}"
    );
    let bare = rig.apply(&rig.plan("bare"));
    assert!(
        bare["error"]["message"]
            .as_str()
            .is_some_and(|why| why.contains("no manifest")),
        "{bare}"
    );
    assert!(
        rig.kept().is_empty(),
        "nothing is left behind: {:?}",
        rig.kept()
    );
    assert_eq!(rig.host(), Value::Null);
    let _ = std::fs::remove_dir_all(&rig.tmp);
}
