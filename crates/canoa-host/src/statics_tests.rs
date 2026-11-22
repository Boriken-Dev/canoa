#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests walk files with std::fs"
)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Documented process-wide statics and `OnceLock` / `LazyLock` / `thread_local!` exceptions allowed in non-test code of
/// `canoa-host` (per `API.md` Process-wide state: every exception is listed here by file and declaration).
/// Home-bound runtime state (such as `usage/read`'s allowance cache) belongs on `Control` or `Serving`, not here.
const ALLOWED: &[(&str, &str)] = &[
    // Config serialization lock across file reads/rewrites
    ("accounts/commands/cfg.rs", "static CONFIG_WRITES:"),
    // Process-wide OS keyring serialization lock & mock/platform store handles
    ("accounts/store.rs", "static TURN:"),
    ("accounts/store.rs", "static ONCE:"),
    ("accounts/store.rs", "static NEXT:"),
    // Immutable default preset cell
    ("config/types.rs", "static EMPTY:"),
    // Control channel dispatch table & lazy Stores cell on Control instances
    ("control.rs", "stores: OnceLock::new(),"),
    ("control/dispatch.rs", "pub(super) static ROUTES:"),
    // Single-flight mDNS socket coordinator
    ("control/discover.rs", "static SLOT:"),
    ("control/discover.rs", "static DONE:"),
    // Per-instance lazy speech service discovery cells
    ("control/speech_offers.rs", "services: OnceLock::new(),"),
    // Per-process delegation host/bridge callback cells (set by binary main)
    ("delegate.rs", "host: OnceLock::new(),"),
    ("delegate.rs", "url: OnceLock::new(),"),
    ("delegate.rs", "bridge: OnceLock::new(),"),
    // Generated canoa-kinds temp descriptor file & anycfg format registry
    ("descriptions.rs", "static FILE:"),
    ("format.rs", "static FORMATS:"),
    // Bundled vendor gateways and account makers
    ("gateways.rs", "static OFFERED:"),
    ("gateways.rs", "static EXTRA_OFFERS:"),
    ("makers.rs", "static OFFERED:"),
    // Counter that gives each file made beside another a name of its own
    ("replace.rs", "static NEXT:"),
    // Process tracing stderr ring buffer for control diagnostics
    ("host_log.rs", "static LINES:"),
    // Process-wide network settings snapshot, generation counter & proxy CA authorities
    ("network.rs", "static GENERATION:"),
    ("network.rs", "static INSTALLED:"),
    ("network/proxy_ca.rs", "static AUTHORITIES:"),
    ("network/session.rs", "static REGISTRY:"),
    // Short-lived provider model probe / pool turn caches & cursors
    ("options.rs", "static PROBED:"),
    ("pools/gather.rs", "static LISTED:"),
    ("pools/gather.rs", "static REPORTED:"),
    ("pools/turn.rs", "pub(crate) static CURSOR:"),
    // Bundled JS script bundles & interpreter/overlay/version caches
    ("profile/bundled.rs", "static BUNDLE:"),
    ("profile/hosted/cache.rs", "static INTERPRETERS:"),
    ("profile/hosted/cache.rs", "static OVERLAYS:"),
    ("profile/hosted/cache.rs", "static ONCE:"),
    ("profile/hosted/private.rs", "made: OnceLock::new(),"),
    ("profile/hosted/private.rs", "held: OnceLock::new(),"),
    ("profile/items.rs", "static KEPT:"),
    ("profile/needs.rs", "static VERSIONS:"),
    ("provider.rs", "static JS:"),
    ("scope.rs", "thread_local! {"),
    ("scope.rs", "static HOST_OWN:"),
    ("scripts.rs", "static ROOT:"),
    ("secrets/bundled.rs", "static BUNDLE:"),
    ("sources.rs", "static VERIFIED:"),
    // Process-wide fork/exec FD inheritance guard & detached starter handle
    ("spawn/apart.rs", "static FORK_GATE:"),
    ("spawn/apart.rs", "static PREPARE:"),
    ("spawn/apart.rs", "static STARTER:"),
    // Bundled speech and catalog source JS registries
    ("speech_scripts.rs", "static BUNDLE:"),
    ("speech_scripts.rs", "static BUNDLED:"),
    ("store/mcp.rs", "backend: OnceLock::new(),"),
    ("store/sources.rs", "static BUNDLE:"),
    ("store/sources.rs", "static MADE:"),
    // Bundled agent vendor registry & interned metadata
    ("vendors.rs", "static KEPT:"),
    ("vendors.rs", "static BUNDLED:"),
    ("vendors.rs", "static KEPT_PROVIDERS:"),
    ("vendors.rs", "static BUNDLED_PROVIDERS:"),
    // Embedded web site assets & file entity-tag cache
    ("web/assets.rs", "pub(super) static BUILT:"),
    ("web/assets.rs", "static TAGS:"),
];

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read source dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "tests") {
                continue;
            }
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "tests.rs"
                || name.ends_with("_tests.rs")
                || name == "statics_tests.rs"
                || name == "real_store.rs"
                || name == "contract.rs"
                || name == "standin.rs"
            {
                continue;
            }
            out.push(path);
        }
    }
}

fn is_static_decl(trimmed: &str) -> bool {
    trimmed.starts_with("static ")
        || trimmed.starts_with("pub static ")
        || trimmed.starts_with("pub(crate) static ")
        || trimmed.starts_with("pub(super) static ")
        || trimmed.contains("OnceLock::new")
        || trimmed.contains("LazyLock::new")
        || trimmed.contains("thread_local!")
}

#[test]
fn non_test_statics_are_all_documented_and_allowlisted() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs(&src, &mut files);
    files.sort();

    let mut seen_patterns = BTreeSet::new();
    let mut unexpected = Vec::new();

    for path in &files {
        let rel = path
            .strip_prefix(&src)
            .expect("under src")
            .to_string_lossy()
            .replace('\\', "/");
        let text = fs::read_to_string(path).expect("read source file");
        let lines: Vec<&str> = text.lines().collect();
        let mut skipping_test_item = false;
        let mut skip_saw_brace = false;
        let mut skip_depth: i32 = 0;

        for (idx, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if skipping_test_item {
                for ch in trimmed.chars() {
                    if ch == '{' {
                        skip_saw_brace = true;
                        skip_depth += 1;
                    } else if ch == '}' {
                        skip_depth -= 1;
                    } else if ch == ';' && !skip_saw_brace {
                        skipping_test_item = false;
                        break;
                    }
                }
                if skip_saw_brace && skip_depth <= 0 {
                    skipping_test_item = false;
                    skip_saw_brace = false;
                    skip_depth = 0;
                }
                continue;
            }
            if trimmed.starts_with("#[cfg(test)]") {
                skipping_test_item = true;
                skip_saw_brace = false;
                skip_depth = 0;
                let rest = trimmed.trim_start_matches("#[cfg(test)]").trim();
                for ch in rest.chars() {
                    if ch == '{' {
                        skip_saw_brace = true;
                        skip_depth += 1;
                    } else if ch == '}' {
                        skip_depth -= 1;
                    } else if ch == ';' && !skip_saw_brace {
                        skipping_test_item = false;
                        break;
                    }
                }
                if skip_saw_brace && skip_depth <= 0 {
                    skipping_test_item = false;
                    skip_saw_brace = false;
                    skip_depth = 0;
                }
                continue;
            }
            if trimmed.starts_with("//") || !is_static_decl(trimmed) {
                continue;
            }
            let matched = ALLOWED
                .iter()
                .filter(|(allowed_file, pat)| *allowed_file == rel && trimmed.contains(*pat))
                .copied()
                .collect::<Vec<_>>();
            if matched.is_empty() {
                unexpected.push(format!("{rel}:{}: {trimmed}", idx + 1));
            } else {
                for entry in matched {
                    seen_patterns.insert(entry);
                }
                if trimmed.contains("static ") {
                    let has_doc = (idx.saturating_sub(4)..idx)
                        .chain(idx.saturating_add(1)..(idx + 3).min(lines.len()))
                        .any(|j| lines[j].trim().starts_with("///"));
                    assert!(
                        has_doc,
                        "allowlisted static lacks a `///` doc comment explaining its process-wide role: {rel}:{}: {trimmed}",
                        idx + 1
                    );
                }
            }
        }
    }

    assert!(
        unexpected.is_empty(),
        "undocumented process-wide static / OnceLock / LazyLock / thread_local! found in canoa-host:\n{}",
        unexpected.join("\n")
    );

    let stale: Vec<String> = ALLOWED
        .iter()
        .filter(|entry| !seen_patterns.contains(*entry))
        .map(|(f, p)| format!("{f} ({p})"))
        .collect();
    assert!(
        stale.is_empty(),
        "stale entries in statics_tests::ALLOWED:\n{}",
        stale.join("\n")
    );
}
