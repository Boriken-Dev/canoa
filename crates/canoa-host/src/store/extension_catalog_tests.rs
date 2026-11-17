//! Canoa's extensions catalog: what an entry may say, what refuses it alone, and what a relative source means.

use serde_json::{Value, json};

use super::extension_catalog::{Base, Digest, Entry, Refusal, Source, Unfit, entry_of, read};

fn base() -> Base<'static> {
    Base {
        repo: "https://example.org/catalogs.git",
        reference: "main",
        path: "lists/extensions.toml",
    }
}

fn pin() -> String {
    format!("sha256:{}", "ab".repeat(32))
}

fn sha() -> String {
    "cd".repeat(32)
}

#[test]
fn a_catalog_is_read_with_its_tree_and_file_entries() {
    let (pin, sha) = (pin(), sha());
    let text = format!(
        r#"name = "Example extensions"
description = "What the team installs."

[extensions.speech-kit]
source = "https://github.com/example/extensions.git"
ref = "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
pin = "{pin}"
load = "bin/speech_kit.dll"
provider = "example_speech"
description = "Adds a speech engine."

[extensions.speech-kit.needs]
kind = "store_extension"
version = "1"
platforms = ["windows-x86_64", "linux"]

[extensions.speech-kit.about]
summary = "Speech engines."
homepage = "https://example.org/speech"
tags = ["speech", "audio-in"]

[extensions.speech-lib]
source = "https://github.com/example/extensions/releases/download/v1.2.0/speech_lib.so"
sha256 = "{sha}"
provider = "example_speech"

[extensions.local]
source = "./speech"
pin = "{pin}"
load = "lib.bin"
provider = "p"
"#
    );
    let read = read(&text, &base()).expect("a catalog");
    assert_eq!(read.name.as_deref(), Some("Example extensions"));
    assert_eq!(read.description.as_deref(), Some("What the team installs."));
    assert!(read.refused.is_empty(), "{:?}", read.refused);
    let kit = &read.entries["speech-kit"];
    assert_eq!(
        kit.source,
        Source::Git {
            repo: "https://github.com/example/extensions.git".into(),
            reference: Some("4b825dc642cb6eb9a060e54bf8d69288fbee4904".into()),
            path: None
        }
    );
    assert_eq!(kit.digest, Digest::Tree(pin.clone()));
    assert_eq!(kit.load.as_deref(), Some("bin/speech_kit.dll"));
    assert_eq!(kit.needs.kind.as_deref(), Some("store_extension"));
    assert_eq!(kit.needs.platforms.as_ref().map(Vec::len), Some(2));
    assert_eq!(kit.about["tags"], json!(["speech", "audio-in"]));
    let lib = &read.entries["speech-lib"];
    assert!(matches!(&lib.source, Source::Url(url) if url.ends_with("speech_lib.so")));
    assert_eq!(lib.digest, Digest::File(sha.clone()));
    let local = &read.entries["local"];
    assert_eq!(
        local.source,
        Source::Git {
            repo: "https://example.org/catalogs.git".into(),
            reference: Some("main".into()),
            path: Some("lists/speech".into())
        },
        "a relative source is a folder of the catalog's own repository at its ref, from the catalog file's folder"
    );
}

fn refused(table: &Value) -> Refusal {
    entry_of("x", table, &base()).expect_err("refused")
}

#[test]
fn an_entry_is_refused_alone_by_what_is_wrong_with_it() {
    let tree = |extra: Value| {
        let mut table = json!({"source": "./x", "pin": pin(), "load": "l.bin", "provider": "p"});
        for (key, value) in extra.as_object().expect("an object") {
            table[key] = value.clone();
        }
        table
    };
    let file = |extra: Value| {
        let mut table =
            json!({"source": "https://example.org/x.so", "sha256": sha(), "provider": "p"});
        for (key, value) in extra.as_object().expect("an object") {
            table[key] = value.clone();
        }
        table
    };
    assert!(entry_of("x", &tree(json!({})), &base()).is_ok());
    assert!(entry_of("x", &file(json!({})), &base()).is_ok());
    for (table, why) in [
        (json!("./x"), Refusal::InvalidEntry),
        (tree(json!({"extra": 1})), Refusal::InvalidEntry),
        (
            tree(json!({"provider": "two words"})),
            Refusal::InvalidEntry,
        ),
        (tree(json!({"provider": null})), Refusal::InvalidEntry),
        (tree(json!({"load": "../x.so"})), Refusal::InvalidEntry),
        (
            tree(json!({"needs": {"version": "1"}})),
            Refusal::InvalidEntry,
        ),
        (tree(json!({"needs": {"arch": "x"}})), Refusal::InvalidEntry),
        (
            tree(json!({"needs": {"platforms": []}})),
            Refusal::InvalidEntry,
        ),
        (
            tree(json!({"needs": {"platforms": ["Windows"]}})),
            Refusal::InvalidEntry,
        ),
        (
            tree(json!({"needs": {"platforms": ["linux", "linux"]}})),
            Refusal::InvalidEntry,
        ),
        (tree(json!({"sha256": sha()})), Refusal::InvalidEntry),
        (file(json!({"pin": pin()})), Refusal::InvalidEntry),
        (file(json!({"load": "x.so"})), Refusal::InvalidEntry),
        (tree(json!({"ref": 3})), Refusal::InvalidEntry),
        (
            json!({"source": "./x", "provider": "p", "load": "l"}),
            Refusal::Unpinned,
        ),
        (
            json!({"source": "https://example.org/x.so", "provider": "p"}),
            Refusal::Unpinned,
        ),
        (tree(json!({"pin": "sha1:00"})), Refusal::InvalidPin),
        (tree(json!({"pin": "abab"})), Refusal::InvalidPin),
        (file(json!({"sha256": "abcd"})), Refusal::InvalidPin),
        (
            tree(json!({"source": "../../../out"})),
            Refusal::LeavesRepository,
        ),
        (
            tree(json!({"source": "./x", "ref": "v2"})),
            Refusal::RefNotAllowed,
        ),
        (file(json!({"ref": "v2"})), Refusal::RefNotAllowed),
        (
            tree(json!({"source": "https://example.org/x.git@v1", "ref": "v2"})),
            Refusal::RefNotAllowed,
        ),
        (
            tree(json!({"source": "https://example.org/x.git", "ref": "-x"})),
            Refusal::InvalidEntry,
        ),
        (
            tree(json!({"source": "s3://bucket/x"})),
            Refusal::InvalidSource,
        ),
        (
            file(json!({"source": "https://example.org/x.so#part"})),
            Refusal::InvalidSource,
        ),
        (
            tree(json!({"source": "https://example.org/x.git#../up"})),
            Refusal::InvalidSource,
        ),
    ] {
        assert_eq!(refused(&table), why, "{table}");
    }
}

#[test]
fn what_the_index_kept_names_a_source_of_the_two_forms_and_no_other() {
    let facts = json!({"provider": "p", "pin": pin(), "load": "l", "needs": {}});
    let entry =
        |origin: Value| Entry::from_index("x", &json!({"origin": origin, "extension": facts}));
    assert!(entry(json!({"kind": "git", "location": "https://example.org/x.git", "ref": "v1", "path": "a/b"})).is_ok());
    for origin in [
        json!({"kind": "directory", "location": "./x"}),
        json!({"kind": "git", "location": "https://example.org/x.git", "ref": "-x"}),
        json!({"kind": "git", "location": "https://example.org/x.git", "path": "../up"}),
        json!({"kind": "url", "location": "ftp://example.org/x.so"}),
        json!({"kind": "git"}),
    ] {
        let refused = entry(origin.clone()).expect_err("refused");
        assert!(
            matches!(refused, Refusal::InvalidSource | Refusal::InvalidEntry),
            "{origin}: {refused:?}"
        );
    }
}

#[test]
fn a_name_the_config_does_not_keep_and_two_that_differ_in_case_are_refused() {
    let pin = pin();
    let entry = |name: &str| {
        format!(
            "[extensions.{name}]\nsource = \"./x\"\npin = \"{pin}\"\nload = \"l\"\nprovider = \"p\"\n"
        )
    };
    let text = format!(
        "{}{}{}{}",
        entry("good"),
        entry("\"a b\""),
        entry("Twin"),
        entry("twin")
    );
    let read = read(&text, &base()).expect("a catalog");
    assert_eq!(read.entries.keys().collect::<Vec<_>>(), ["good"]);
    let mut why: Vec<(&str, Refusal)> = read
        .refused
        .iter()
        .map(|(name, why)| (name.as_str(), *why))
        .collect();
    why.sort_by_key(|(name, _)| *name);
    assert_eq!(
        why,
        [
            ("Twin", Refusal::DuplicateName),
            ("a b", Refusal::InvalidName),
            ("twin", Refusal::DuplicateName)
        ]
    );
}

#[test]
fn informational_fields_that_are_not_valid_are_left_out_and_the_entry_stays() {
    let table = json!({
        "source": "./x", "pin": pin(), "load": "l", "provider": "p",
        "description": "ok",
        "about": {
            "summary": "Two\nlines",
            "homepage": "http://example.org",
            "repository": "https://example.org/r",
            "license": "MIT",
            "icon": "icons/x.svg",
            "tags": ["Upper"],
            "authors": ["A <a@example.org>"],
            "stars": 5
        }
    });
    let entry = entry_of("x", &table, &base()).expect("listed");
    assert_eq!(entry.description.as_deref(), Some("ok"));
    let mut kept: Vec<&str> = entry.about.keys().map(String::as_str).collect();
    kept.sort_unstable();
    assert_eq!(kept, ["authors", "homepage", "license", "repository"]);
}

#[test]
fn a_file_that_is_not_a_catalog_is_refused_whole() {
    let big = "#".repeat(super::extension_catalog::MAX_BYTES + 1);
    for (text, says) in [
        ("[[", "TOML"),
        ("name = \"x\"\n", "[extensions]"),
        ("extensions = 3\n", "[extensions]"),
        (big.as_str(), "larger"),
    ] {
        let why = read(text, &base()).expect_err("refused");
        assert!(why.contains(says), "{why}");
    }
}

#[test]
fn what_the_index_kept_is_read_again_into_the_same_entry() {
    let (pin, sha) = (pin(), sha());
    let text = format!(
        "[extensions.t]\nsource = \"https://example.org/e.git@v1#sub/dir\"\npin = \"{pin}\"\nload = \"l.bin\"\nprovider = \"p\"\n\
         needs = {{ kind = \"store_extension\", version = \"1\" }}\n\n\
         [extensions.f]\nsource = \"https://example.org/f.so\"\nsha256 = \"{sha}\"\nprovider = \"q\"\n"
    );
    let read = read(&text, &base()).expect("a catalog");
    for (name, entry) in &read.entries {
        let index = json!({"origin": entry.origin_json(), "extension": entry.extension_json()});
        let again = Entry::from_index(name, &index).expect("read again");
        assert_eq!(again.source, entry.source, "{name}");
        assert_eq!(again.digest, entry.digest, "{name}");
        assert_eq!(again.load, entry.load, "{name}");
        assert_eq!(again.needs, entry.needs, "{name}");
        assert_eq!(again.provider, entry.provider, "{name}");
    }
    // A digest of the other kind, or a file where the source has none, is refused whatever the index says.
    let mut crossed = json!({"origin": read.entries["f"].origin_json(), "extension": read.entries["t"].extension_json()});
    assert_eq!(Entry::from_index("f", &crossed), Err(Refusal::InvalidEntry));
    crossed["extension"]["pin"] = json!("abc");
    crossed["origin"] = read.entries["t"].origin_json();
    assert_eq!(Entry::from_index("t", &crossed), Err(Refusal::InvalidPin));
}

#[test]
fn a_host_fits_by_kind_then_version_then_platform() {
    let entry = |needs: Value| {
        let table =
            json!({"source": "./x", "pin": pin(), "load": "l", "provider": "p", "needs": needs});
        entry_of("x", &table, &base()).expect("an entry").needs
    };
    let kinds: &[(&str, &[&str])] = &[("store_extension", &["1", "2"])];
    let fits = |needs: &super::extension_catalog::Needs| needs.fits(kinds, "windows", "x86_64");
    assert_eq!(fits(&entry(json!({}))), Ok(()));
    assert_eq!(
        fits(&entry(
            json!({"kind": "store_extension", "version": "2", "platforms": ["windows"]})
        )),
        Ok(())
    );
    assert_eq!(fits(&entry(json!({"kind": "viewer"}))), Err(Unfit::Kind));
    assert_eq!(
        fits(&entry(json!({"kind": "Store_Extension"}))),
        Err(Unfit::Kind)
    );
    assert_eq!(
        fits(&entry(json!({"kind": "store_extension", "version": "3"}))),
        Err(Unfit::Version)
    );
    assert_eq!(
        fits(&entry(json!({"kind": "store_extension", "version": "1.0"}))),
        Err(Unfit::Version)
    );
    assert_eq!(
        fits(&entry(json!({"platforms": ["windows-aarch64", "linux"]}))),
        Err(Unfit::Platform)
    );
    assert_eq!(
        fits(&entry(json!({"platforms": ["windows-x86_64"]}))),
        Ok(())
    );
    assert_eq!(
        fits(&entry(json!({"kind": "viewer", "platforms": ["linux"]}))),
        Err(Unfit::Kind)
    );
}

#[test]
fn nothing_is_refused_for_where_a_source_is_or_what_it_needs_to_be_reached() {
    let ok = |source: &str| {
        let table = json!({"source": source, "pin": pin(), "load": "l", "provider": "p"});
        entry_of("x", &table, &base())
    };
    let url = |source: &str| {
        let table = json!({"source": source, "sha256": sha(), "provider": "p"});
        entry_of("x", &table, &base())
    };
    for git in [
        "/srv/repos/x",
        "/srv/repos/x.git@v1#ext/a",
        "C:/repos/x",
        "C:\\repos\\x@main",
        "file:///srv/x.git",
        "file:///srv/x",
        "git@example.org:team/x.git@v2#sub",
        "ssh://git@example.org/team/x.git",
        "git://example.org/x.git",
        "https://localhost/x.git",
        "https://tok@example.org/x.git",
        "https://user:secret@example.org/x.git@v1#a",
        "http://example.org/x.git",
        "git+https://example.org/x",
        "./a b/c",
    ] {
        let entry = ok(git).unwrap_or_else(|why| panic!("{git}: {why:?}"));
        assert!(matches!(entry.source, Source::Git { .. }), "{git}");
    }
    let entry = ok("git@example.org:team/x.git@v2#sub").expect("scp-like");
    assert_eq!(
        entry.source,
        Source::Git {
            repo: "git@example.org:team/x.git".into(),
            reference: Some("v2".into()),
            path: Some("sub".into())
        }
    );
    for file in [
        "http://example.org/x.so",
        "https://127.0.0.1/x.so",
        "https://example.org/x.so?sig=1&x=2",
        "https://user:pw@example.org/files/x.so",
    ] {
        let entry = url(file).unwrap_or_else(|why| panic!("{file}: {why:?}"));
        assert_eq!(entry.source, Source::Url(file.into()));
    }
    // A ref with a slash is a branch git knows, given as `ref`.
    let table = json!({"source": "https://example.org/x.git", "ref": "release/1.0", "pin": pin(), "load": "l", "provider": "p"});
    assert!(entry_of("x", &table, &base()).is_ok());
}
