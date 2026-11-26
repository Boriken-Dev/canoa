//! A stand-in for Foundry Local's `foundry` command line, for the tests of the models page: it answers the commands the page runs
//! with recorded outputs and exit codes, and downloads nothing. A model whose name opens with `unknown` is one the catalog does not
//! know (exit 3), and `not-cached` is one that is not in the cache (exit 2 for a removal); any other command is refused with exit 2,
//! as the real program refuses one it does not know.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    clippy::print_stdout,
    clippy::print_stderr,
    missing_docs,
    reason = "a test program"
)]

use std::process::ExitCode;

const CATALOG: &str = include_str!("../tests/fixtures/model_list_recorded.json");
const CACHED: &str = include_str!("../tests/fixtures/foundry_cache_list.json");
const REMOVED: &str = include_str!("../tests/fixtures/foundry_cache_rm.json");
const NOT_FOUND: &str = include_str!("../tests/fixtures/foundry_error_model_not_found.json");
const INVALID: &str = include_str!("../tests/fixtures/foundry_error_invalid_argument.json");

fn refuse(code: u8, json: &str) -> ExitCode {
    println!("{}", json.trim_end());
    ExitCode::from(code)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["model", "list", "--variants", "--loaded", "-o", "json"] => {
            println!("{{\"variants\": []}}");
            ExitCode::SUCCESS
        }
        ["model", "list", "--variants", "-o", "json"] => {
            println!("{}", CATALOG.trim_end());
            ExitCode::SUCCESS
        }
        ["cache", "list", "--variants", "-o", "json"] => {
            println!("{}", CACHED.trim_end());
            ExitCode::SUCCESS
        }
        ["model", "download", name, "-o", "json"] => {
            if name.starts_with("unknown") {
                return refuse(3, NOT_FOUND);
            }
            // The text a download prints: its size, a percentage line rewritten with carriage returns, then its closing sentence.
            print!(
                "Downloading model ({name}, 7.1 GB)\n\r{name}: 15%\r{name}: 62%\r{name}: 100%\n"
            );
            println!("Downloaded '{name}' to the local cache.");
            ExitCode::SUCCESS
        }
        ["cache", "rm", name, "-o", "json"] => {
            if name.starts_with("unknown") {
                return refuse(3, NOT_FOUND);
            }
            if name.starts_with("not-cached") {
                return refuse(2, INVALID);
            }
            println!("{}", REMOVED.trim_end().replace("deepseek-r1-14b", name));
            ExitCode::SUCCESS
        }
        ["model", "load" | "unload", name, "-o", "json"] => {
            println!("{{\"success\":true,\"message\":\"done '{name}'\"}}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("error: unrecognized command {words:?}");
            ExitCode::from(2)
        }
    }
}
