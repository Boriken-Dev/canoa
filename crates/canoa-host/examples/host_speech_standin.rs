//! A stand-in speech-to-text program the tests run: it reads all of its standard input and answers what its first argument
//! says. `fixed` (the default) answers a fixed transcript for any audio, `echo` answers the other arguments and how many bytes
//! it read, `hints` answers the language, accent and voice the host told it.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "an example names what it uses from std"
)]

use std::io::{Read, Write};

/// What `fixed` answers.
const TRANSCRIPT: &str = "the quick brown fox";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map_or("fixed", String::as_str);
    let mut stdin = std::io::stdin().lock();
    let mut chunk = [0_u8; 4096];
    let mut total = 0_usize;
    loop {
        let read = stdin.read(&mut chunk).expect("standard input");
        if read == 0 {
            break;
        }
        total += read;
    }
    let mut stdout = std::io::stdout().lock();
    match mode {
        "echo" => write!(stdout, "{} bytes={total}", args[1..].join(" ")),
        "hints" => {
            let said: Vec<String> = ["LANGUAGE", "ACCENT", "VOICE"]
                .iter()
                .map(|name| {
                    let value = std::env::var(format!("CANOA_SPEECH_{name}")).unwrap_or_default();
                    format!("{name}={value}")
                })
                .collect();
            write!(stdout, "{}", said.join(";"))
        }
        _ => write!(stdout, "{TRANSCRIPT}"),
    }
    .expect("standard output");
}
