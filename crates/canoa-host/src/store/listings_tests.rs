use serde_json::json;

use super::{SkillSpec, holds_password, mcp_page, skills_listing};

const COMMIT: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678";

fn parse(text: &str) -> SkillSpec {
    SkillSpec::parse(text).unwrap_or_else(|why| panic!("{text}: {why}"))
}

fn refused(text: &str) -> String {
    SkillSpec::parse(text).expect_err(text)
}

#[test]
fn a_skills_source_is_written_the_way_npx_skills_takes_it() {
    let anthropic = "https://github.com/anthropics/skills.git";
    let source = parse("anthropics/skills");
    assert_eq!(
        (&*source.repo, source.reference, &*source.path, source.only),
        (anthropic, None, "", None)
    );
    let source = parse("owner/repo/skills/pdf");
    assert_eq!(
        (&*source.repo, &*source.path),
        ("https://github.com/owner/repo.git", "skills/pdf")
    );
    assert_eq!(parse("owner/repo#dev").reference.as_deref(), Some("dev"));
    assert_eq!(
        parse("owner/repo@v1.2.3").reference.as_deref(),
        Some("v1.2.3")
    );
    let named = parse("owner/repo@pdf");
    assert_eq!(
        (named.reference, named.only.as_deref()),
        (None, Some("pdf"))
    );
    assert_eq!(
        parse(&format!("owner/repo@{COMMIT}")).reference.as_deref(),
        Some(COMMIT)
    );
    assert_eq!(
        parse("github:owner/repo").repo,
        "https://github.com/owner/repo.git"
    );
    assert_eq!(
        parse("gitlab:owner/repo").repo,
        "https://gitlab.com/owner/repo.git"
    );
}

#[test]
fn an_address_a_tree_address_and_a_folder_are_sources_too() {
    let source = parse("https://github.com/o/r/tree/main/skills/pdf");
    assert_eq!(
        (&*source.repo, source.reference.as_deref(), &*source.path),
        ("https://github.com/o/r", Some("main"), "skills/pdf")
    );
    let source = parse("https://gitlab.com/o/r/-/tree/v1.0.0");
    assert_eq!(
        (&*source.repo, source.reference.as_deref(), &*source.path),
        ("https://gitlab.com/o/r", Some("v1.0.0"), "")
    );
    let source = parse("https://example.test/team/skills.git@v2.0.0");
    assert_eq!(
        (&*source.repo, source.reference.as_deref()),
        ("https://example.test/team/skills.git", Some("v2.0.0"))
    );
    let source = parse("git+https://example.test/team/skills.git#release");
    assert_eq!(
        (&*source.repo, source.reference.as_deref()),
        ("https://example.test/team/skills.git", Some("release"))
    );
    let source = parse("git@github.com:o/r.git@pdf");
    assert_eq!(
        (&*source.repo, source.only.as_deref()),
        ("git@github.com:o/r.git", Some("pdf"))
    );
    assert_eq!(
        parse("https://example.test:8443/o/r.git").repo,
        "https://example.test:8443/o/r.git"
    );
    for folder in [
        "/home/ana/skills",
        "./skills",
        "../skills",
        r"C:\skills",
        "C:/skills",
        "file:///home/ana/skills",
    ] {
        let source = parse(folder);
        assert_eq!(
            (&*source.repo, source.reference, &*source.path),
            (folder, None, ""),
            "{folder}"
        );
    }
}

#[test]
fn what_cannot_be_a_source_is_refused_with_the_reason() {
    for (text, why) in [
        ("", "Name a source"),
        ("owner", "owner/repo"),
        ("owner/repo/../x", ".."),
        ("owner/repo#", "Nothing follows"),
        ("owner/repo@a b", "skill name"),
        ("https://user:secret@example.test/o/r.git", "password"),
        ("https://example.test/o/SKILL.md", "SKILL.md"),
        ("https://example.test/o/skills.tar.gz", "archive"),
        (
            "https://skills.example.test/.well-known/agent-skills/index.json",
            "well-known",
        ),
        ("two\nlines", "one line"),
    ] {
        assert!(refused(text).contains(why), "{text:?}: {}", refused(text));
    }
    assert!(holds_password("ssh://u:p@host/x"));
    assert!(!holds_password("ssh://git@host/x"));
}

#[test]
fn a_skills_listing_names_each_skill_and_the_source_that_holds_it() {
    let text = json!({"skills": [
        {"name": "pdf", "description": "Fill forms.", "source": "anthropics/skills", "path": "skills/pdf"},
        {"name": "lint", "source": "owner/repo/tools/lint@v2.0.0"},
        {"name": "PDF", "source": "owner/other"},
        {"description": "no name", "source": "owner/repo"},
        {"name": "nosource"},
        {"name": "bad name!", "source": "owner/repo"},
        {"name": "far", "source": "owner"},
        {"name": "up", "source": "owner/repo", "path": "../x"},
        {"name": "pinned", "source": "owner/repo", "ref": "-bad"},
        "not an entry",
    ]})
    .to_string();
    let listing = skills_listing(&text).expect("a listing");
    let by_name = |name: &str| {
        listing
            .skills
            .iter()
            .find(|skill| skill.name == name)
            .unwrap_or_else(|| panic!("no {name}"))
    };
    let pdf = by_name("pdf");
    assert_eq!(pdf.description, "Fill forms.");
    assert_eq!(pdf.source, "anthropics/skills");
    assert_eq!(pdf.path, "skills/pdf");
    assert_eq!(pdf.problem, None);
    let lint = by_name("lint");
    assert_eq!(lint.path, "tools/lint");
    assert_eq!(lint.reference.as_deref(), Some("v2.0.0"));
    assert_eq!(lint.problem, None);
    for name in ["bad name!", "far", "up", "pinned"] {
        assert!(by_name(name).problem.is_some(), "{name}");
    }
    // A name already listed (without regard to case), none, no source and a non-entry are left out and counted.
    assert_eq!(listing.skipped, 4);
    assert_eq!(listing.skills.len(), 6);
    assert!(!listing.truncated);
    assert_eq!(pdf.to_value()["source"], "anthropics/skills");
    assert!(pdf.to_value().get("problem").is_none());
    assert!(by_name("far").to_value()["problem"].is_string());

    for wrong in ["", "[]", "{\"skills\": 3}", "{\"servers\": []}"] {
        assert!(skills_listing(wrong).is_err(), "{wrong}");
    }
}

#[test]
fn a_listing_is_cut_at_the_most_skills_one_lists() {
    let skills: Vec<_> = (0..super::MAX_SKILLS + 5)
        .map(|n| json!({"name": format!("s{n}"), "source": "o/r"}))
        .collect();
    let listing = skills_listing(&json!({"skills": skills}).to_string()).expect("listing");
    assert_eq!(listing.skills.len(), super::MAX_SKILLS);
    assert!(listing.truncated);
}

fn server(name: &str, description: &str, latest: bool) -> serde_json::Value {
    json!({
        "server": {"name": name, "title": name, "description": description, "version": "1.0.0"},
        "_meta": {"io.modelcontextprotocol.registry/official": {"isLatest": latest, "status": "active"}},
    })
}

#[test]
fn a_registry_page_keeps_the_latest_named_servers_and_narrows_to_a_search() {
    let text = json!({
        "servers": [
            server("io.example/files", "Reads files", true),
            server("io.example/old", "An old one", false),
            {"server": {"description": "no name"}},
            "nonsense",
            server("io.example/mail", "Sends mail", true),
        ],
        "metadata": {"nextCursor": "page2"},
    })
    .to_string();
    let page = mcp_page(&text, "").expect("a page");
    let names: Vec<_> = page
        .servers
        .iter()
        .map(|each| each["server"]["name"].as_str().expect("name"))
        .collect();
    assert_eq!(names, ["io.example/files", "io.example/mail"]);
    assert_eq!(page.next.as_deref(), Some("page2"));
    assert_eq!(page.to_value()["metadata"]["nextCursor"], "page2");

    let narrowed = mcp_page(&text, "MAIL sends").expect("a page");
    assert_eq!(narrowed.servers.len(), 1);

    let last = mcp_page("{\"servers\": []}", "").expect("a page");
    assert!(last.servers.is_empty() && last.next.is_none());
    for wrong in ["", "{}", "{\"servers\": {}}", "[]"] {
        assert!(mcp_page(wrong, "").is_err(), "{wrong}");
    }
}
