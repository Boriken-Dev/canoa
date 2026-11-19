//! A stand-in speech-to-text program the tests run: it reads its whole standard input and answers what its first argument
//! says. `fixed` answers a fixed transcript for any audio, `echo` the other arguments and what it read, `grow` a growing text
//! as audio arrives, `fail` an error, `hang` nothing, `flood` more output than anyone keeps (`flood_utf8`: the same, of `ñ日😀a` repeated), `env` the hints it was given, `beat FILE` a byte appended to FILE
//! every 50 ms for five seconds, which stops when the program is ended.
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
    let mut stdout = std::io::stdout().lock();
    let mut chunk = [0_u8; 1024];
    let (mut total, mut head) = (0_usize, Vec::new());
    loop {
        let read = stdin.read(&mut chunk).expect("standard input");
        if read == 0 {
            break;
        }
        if head.len() < 4 {
            head.extend_from_slice(&chunk[..read.min(4)]);
        }
        total += read;
        if mode == "grow" {
            write!(stdout, "{total} ").expect("standard output");
            stdout.flush().expect("flushed");
        }
    }
    match mode {
        "echo" => write!(
            stdout,
            "{} bytes={total} riff={}",
            args[1..].join(" "),
            head.starts_with(b"RIFF")
        )
        .expect("standard output"),
        "env" => {
            let names = ["LANGUAGE", "VOCABULARY", "ACCENT", "VOICE"];
            let said: Vec<String> = names
                .iter()
                .map(|name| {
                    let value = std::env::var(format!("CANOA_SPEECH_{name}")).unwrap_or_default();
                    format!("{name}={value}")
                })
                .collect();
            write!(stdout, "{}", said.join(";")).expect("standard output");
        }
        "fail" => {
            eprintln!("boom");
            std::process::exit(3);
        }
        "hang" => std::thread::sleep(std::time::Duration::from_secs(60)),
        "beat" => {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&args[1])
                .expect("the beat file");
            for _ in 0..100 {
                file.write_all(b".").expect("a beat");
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        "flood" => {
            let block = [b'a'; 65536];
            for _ in 0..80 {
                stdout.write_all(&block).expect("standard output");
            }
        }
        "flood_utf8" => {
            // Whole units of ten bytes, so a block never ends inside a character.
            let block = "ñ日😀a".repeat(6553).into_bytes();
            for _ in 0..80 {
                stdout.write_all(&block).expect("standard output");
            }
        }
        "grow" => {}
        _ => write!(stdout, "{TRANSCRIPT}").expect("standard output"),
    }
}
