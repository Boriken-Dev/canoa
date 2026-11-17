//! A stand-in text-to-speech program the host's tests run. It reads its standard input a line at a time and writes the "audio" of
//! each line: the line itself between brackets, so a test reads what was spoken, in what order.
//!
//! The first argument picks a mode. `lines` (the default) writes `[line]`; `piper` (chosen when `--output-raw` is among the
//! arguments, as Piper is run) writes `[line|model=STEM|scale=N|speaker=N]`; `env` writes the `CANOA_SAY_*` variables once;
//! `fail` writes one line and exits with an error; `slow` writes a line a second.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "an example names what it uses from std"
)]

use std::io::{BufRead, Write};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = if args.iter().any(|arg| arg == "--output-raw") {
        "piper"
    } else {
        args.first().map_or("lines", String::as_str)
    };
    let mut stdout = std::io::stdout().lock();
    let flag = |name: &str| {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|at| args.get(at + 1))
            .cloned()
    };
    if mode == "env" {
        let said: Vec<String> = ["VOICE", "MODEL", "STYLE", "SPEED", "LANGUAGE"]
            .iter()
            .map(|name| {
                let value = std::env::var(format!("CANOA_SAY_{name}")).unwrap_or_default();
                format!("{name}={value}")
            })
            .collect();
        write!(stdout, "{}", said.join(";")).expect("standard output");
        return;
    }
    let model = flag("--model").map(|path| {
        let name = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_owned();
        name.trim_end_matches(".onnx").to_owned()
    });
    let scale = flag("--length_scale").unwrap_or_else(|| "1".to_owned());
    let speaker = flag("--speaker").unwrap_or_else(|| "0".to_owned());
    for line in std::io::stdin().lock().lines() {
        let line = line.expect("standard input");
        match mode {
            "piper" => write!(
                stdout,
                "[{line}|model={}|scale={scale}|speaker={speaker}]",
                model.as_deref().unwrap_or("")
            ),
            _ => write!(stdout, "[{line}]"),
        }
        .expect("standard output");
        stdout.flush().expect("flushed");
        match mode {
            "fail" => {
                eprintln!("boom");
                std::process::exit(3);
            }
            "slow" => std::thread::sleep(std::time::Duration::from_secs(1)),
            _ => {}
        }
    }
}
