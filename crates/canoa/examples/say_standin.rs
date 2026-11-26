//! A stand-in text-to-speech program the tests run. It reads its standard input a line at a time and writes the "audio" of
//! each line: the line itself between brackets, so a test reads what was spoken, in what order, in what voice.
//!
//! The first argument picks a mode. `lines` (the default) writes `[line]`; `env` writes the `SAY_*` variables, with the build's prefix, and the other arguments once; `wav` writes a WAV header and then `lines`;
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
    let mode = args.first().map_or("lines", String::as_str);
    let mut stdout = std::io::stdout().lock();
    match mode {
        "env" => {
            let said: Vec<String> = ["VOICE", "MODEL", "STYLE", "SPEED", "LANGUAGE"]
                .iter()
                .map(|name| {
                    let value = std::env::var(format!("{}_SAY_{name}", canoa::brand::ENV_PREFIX))
                        .unwrap_or_default();
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
    for line in std::io::stdin().lock().lines() {
        let line = line.expect("standard input");
        write!(stdout, "[{line}]").expect("standard output");
        stdout.flush().expect("flushed");
        if mode == "fail" {
            eprintln!("boom");
            std::process::exit(3);
        }
    }
}
