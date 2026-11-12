//! `canoa-pack <app folder> <version> <payload.zip>`: packs the built app folder for the launcher and prints the
//! archive's SHA-256. The packaging step runs it, then builds `canoa-launch` with CANOA_LAUNCH_PAYLOAD naming the archive.

use canoa_launch::{APP_PROGRAM, Error, Payload, pack, sha256_hex};
use std::path::Path;
use std::process::ExitCode;

fn run(dir: &str, version: &str, out: &str) -> Result<String, Error> {
    let bytes = pack(Path::new(dir), version)?;
    let digest = sha256_hex(&bytes);
    // Read back the way the launcher will, so an archive it would refuse is never written.
    let manifest = Payload {
        bytes: &bytes,
        sha256: &digest,
    }
    .manifest()?;
    if manifest.entry(APP_PROGRAM).is_none() {
        return Err(Error::Refused(format!("{dir} holds no {APP_PROGRAM}")));
    }
    std::fs::write(out, &bytes).map_err(|e| Error::Refused(format!("write {out}: {e}")))?;
    Ok(digest)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [dir, version, out] = args.as_slice() else {
        eprintln!("usage: canoa-pack <app folder> <version> <payload.zip>");
        return ExitCode::from(2);
    };
    match run(dir, version, out) {
        Ok(digest) => {
            println!("{digest}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("canoa-pack: {e}");
            ExitCode::FAILURE
        }
    }
}
