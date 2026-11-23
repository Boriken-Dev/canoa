//! A stand-in for Piper's command line that the tests run. Given `--model FILE`, `--output-raw`, `--length_scale N` and
//! `--speaker N`, it reads its standard input a line at a time and writes `[line|model=STEM|scale=N|speaker=N]` for each, so a test
//! reads what was spoken, in what order and with which voice.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "an example names what it uses from std"
)]

use std::io::{BufRead, Write};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|at| args.get(at + 1))
            .cloned()
    };
    let model = flag("--model").map(|path| {
        let name = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_owned();
        name.trim_end_matches(".onnx").to_owned()
    });
    let scale = flag("--length_scale").unwrap_or_else(|| "1".to_owned());
    let speaker = flag("--speaker").unwrap_or_else(|| "0".to_owned());
    let mut stdout = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let line = line.expect("standard input");
        write!(
            stdout,
            "[{line}|model={}|scale={scale}|speaker={speaker}]",
            model.as_deref().unwrap_or("")
        )
        .expect("standard output");
        stdout.flush().expect("flushed");
    }
}
