//! A stand-in for Foundry Local's command line, for the tests of the host's model calls. It holds a three-model catalog and keeps
//! which are downloaded and loaded in the files `cached` and `loaded` of its own folder, one model a line, so a test copies it into a
//! folder of its own. A file `sleep_ms` there (a number) makes every `model` action wait that long first; a file `fail` makes `list`
//! fail. It loads nothing and downloads nothing.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    missing_docs,
    reason = "a test program"
)]

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

const CATALOG: [(&str, &str, &str, &str, u64); 3] = [
    ("qwen3-14b-generic-gpu:2", "qwen3-14b", "chat", "gpu", 10137),
    (
        "phi-4-mini-generic-cpu:1",
        "phi-4-mini",
        "chat",
        "cpu",
        2400,
    ),
    (
        "openai-whisper-tiny-generic-cpu:1",
        "openai-whisper-tiny",
        "speech",
        "cpu",
        151,
    ),
];

fn state(name: &str) -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join(name))
}

fn lines(name: &str) -> Vec<String> {
    state(name)
        .and_then(|path| fs::read_to_string(path).ok())
        .map(|text| text.lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

fn keep(name: &str, lines: &[String]) {
    if let Some(path) = state(name) {
        let _ = fs::write(path, lines.join("\n"));
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let word = |at: usize| args.get(at).map(String::as_str);
    if word(0) == Some("--version") {
        println!("0.14.0");
        return ExitCode::SUCCESS;
    }
    if word(0) != Some("model") || !args.iter().any(|arg| arg == "json") {
        eprintln!("unexpected arguments: {args:?}");
        return ExitCode::from(2);
    }
    if let Some(ms) = state("sleep_ms")
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|ms| ms.trim().parse().ok())
    {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
    let (cached, loaded) = (lines("cached"), lines("loaded"));
    match word(1) {
        Some("list") => {
            if state("fail").is_some_and(|path| path.exists()) {
                eprintln!("\u{1b}[31merror:\u{1b}[0m the catalog is not reachable");
                return ExitCode::from(1);
            }
            let rows: Vec<serde_json::Value> = CATALOG
                .iter()
                .filter(|(id, ..)| {
                    (!args.iter().any(|a| a == "--cached") || cached.iter().any(|c| c == id))
                        && (!args.iter().any(|a| a == "--loaded") || loaded.iter().any(|l| l == id))
                })
                .map(|(id, alias, task, device, mb)| {
                    serde_json::json!({"id": id, "alias": alias, "task": task, "device": device, "fileSizeMb": mb})
                })
                .collect();
            println!("{}", serde_json::Value::Array(rows));
        }
        Some(action @ ("download" | "load" | "unload")) => {
            let Some(model) = word(2).filter(|id| CATALOG.iter().any(|(each, ..)| each == id))
            else {
                eprintln!("no such model: {:?}", word(2));
                return ExitCode::from(1);
            };
            let model = model.to_owned();
            match action {
                "download" => {
                    let mut now = cached;
                    if !now.contains(&model) {
                        now.push(model);
                    }
                    keep("cached", &now);
                }
                "load" => {
                    if !cached.contains(&model) {
                        eprintln!("{model} is not downloaded");
                        return ExitCode::from(1);
                    }
                    let mut now = loaded;
                    if !now.contains(&model) {
                        now.push(model);
                    }
                    keep("loaded", &now);
                }
                _ => keep(
                    "loaded",
                    &loaded
                        .into_iter()
                        .filter(|each| *each != model)
                        .collect::<Vec<_>>(),
                ),
            }
            println!("{{\"ok\": true}}");
        }
        _ => {
            eprintln!("unexpected arguments: {args:?}");
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}
