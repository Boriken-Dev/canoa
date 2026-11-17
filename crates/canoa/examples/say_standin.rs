//! A stand-in text-to-speech program the tests run. It reads its standard input a line at a time and writes the "audio" of
//! each line: the line itself between brackets, so a test reads what was spoken, in what order, in what voice.
//!
//! The first argument picks a mode. `lines` (the default) writes `[line]`; `piper` answers as Piper's command line does, given
//! `--model FILE`, `--output-raw`, `--length_scale N` and `--speaker N`, and writes `[line|model=STEM|scale=N|speaker=N]`;
//! `env` writes the `CANOA_SAY_*` variables and the other arguments once; `wav` writes a WAV header and then `lines`;
//! `fail` writes one line and exits with an error; `hang` writes nothing; `flood` writes more than anyone buffers; `args` writes
//! its other arguments (the text, when it is one of them) once and reads nothing.
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
    match mode {
        "env" => {
            let said: Vec<String> = ["VOICE", "MODEL", "STYLE", "SPEED", "LANGUAGE"]
                .iter()
                .map(|name| {
                    let value = std::env::var(format!("CANOA_SAY_{name}")).unwrap_or_default();
                    format!("{name}={value}")
                })
                .collect();
            write!(stdout, "{};args={}", said.join(";"), args[1..].join(" "))
                .expect("standard output");
            return;
        }
        "args" => {
            write!(stdout, "{}", args[1..].join(" ")).expect("standard output");
            return;
        }
        "hang" => {
            std::thread::sleep(std::time::Duration::from_secs(60));
            return;
        }
        "flood" => {
            let block = [7_u8; 65536];
            for _ in 0..200 {
                if stdout.write_all(&block).is_err() {
                    return;
                }
            }
            return;
        }
        "wav" => {
            let mut header = Vec::new();
            header.extend_from_slice(b"RIFF\xff\xff\xff\xffWAVEfmt ");
            header.extend_from_slice(&16_u32.to_le_bytes());
            header.extend_from_slice(&[1, 0, 1, 0]);
            header.extend_from_slice(&22_050_u32.to_le_bytes());
            header.extend_from_slice(&44_100_u32.to_le_bytes());
            header.extend_from_slice(&[2, 0, 16, 0]);
            header.extend_from_slice(b"data\xff\xff\xff\xff");
            stdout.write_all(&header).expect("standard output");
        }
        _ => {}
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
        if mode == "fail" {
            eprintln!("boom");
            std::process::exit(3);
        }
    }
}
