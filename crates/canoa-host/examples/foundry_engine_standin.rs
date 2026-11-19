//! A stand-in for `canoa foundry-engine`, for the tests of the host's model process: it serves the worker's lines with an engine
//! that echoes a request, and acts on a request's own words: `crash` ends the process the way a native crash does, `wait` works
//! until it is cancelled, `deaf` works for a minute and does not look at a cancel. With `--fail-open` before the subcommand it says
//! it cannot open its engine. It loads nothing.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    missing_docs,
    reason = "a test program"
)]

use std::time::{Duration, Instant};

use canoa_foundry::worker::serve;
use canoa_foundry::{Cancel, Engine, EngineError, ModelInfo};
use serde_json::{Value, json};

struct Scripted;

impl Engine for Scripted {
    fn model(&self) -> Result<ModelInfo, EngineError> {
        Ok(ModelInfo::new("stand-in").tools(true).context(8192))
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo::new("stand-in").tools(true).context(8192)]
    }

    fn chat(&self, request: &Value, cancel: &Cancel) -> Result<Value, EngineError> {
        if request.get("crash").is_some() {
            std::process::exit(139);
        }
        let until = Instant::now() + Duration::from_secs(60);
        if request.get("wait").is_some() {
            while !cancel.is_cancelled() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(10));
            }
            return Err(EngineError::Cancelled);
        }
        if request.get("deaf").is_some() {
            std::thread::sleep(Duration::from_secs(60));
        }
        Ok(json!({"echo": request, "pid": std::process::id()}))
    }
}

fn main() -> std::io::Result<()> {
    let fail = std::env::args().any(|arg| arg == "--fail-open");
    // Whatever a native library would print lands among the lines.
    println!("a line a library printed");
    serve(
        || {
            if fail {
                Err(EngineError::Failed("no native library".to_owned()))
            } else {
                Ok(Scripted)
            }
        },
        std::io::stdin().lock(),
        std::io::stdout(),
    )
}
