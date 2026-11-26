//! The `command` catalog generator: a program the person named, run each time the catalog is refreshed, as the person and with
//! their environment. What it prints (or a file it writes, `file`, in its folder `cwd`) is the catalog, in the format the Store
//! type documents. It reads no source.

use std::time::{Duration, Instant};

use canoa::content::{
    CatalogGeneratorProvider, ContentHost, GenerateRequest, GeneratorInfo, SourceUse,
};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};
use coqui_acp_api::{ProcessSpawner, READ_LINK_DOWN, READ_TIMED_OUT, SpawnSpec};
use guatiao_intake::Form;

use super::{Backend, GeneratorHeld};

/// How long a program runs unless the form says otherwise, in seconds.
const DEFAULT_SECONDS: u64 = 60;

/// The most a program's output may hold.
const MOST_BYTES: usize = 8 << 20;

/// The last bytes of a program's error output kept to say why it failed.
const TAIL_BYTES: usize = 2048;

/// What the generator's form asks for.
#[derive(ToValue, FromValue, Schema, Form)]
#[allow(dead_code, reason = "only its description is read")]
struct CommandValues {
    /// The program first, then each argument.
    #[schema(title = "Program and arguments")]
    command: Vec<String>,
    /// Where it runs; the host's own folder when empty.
    #[schema(title = "Folder")]
    cwd: Option<String>,
    /// A file it writes, in its folder, to take instead of what it prints.
    #[schema(title = "Output file")]
    file: Option<String>,
    /// How long it may run.
    #[schema(title = "Seconds it may run")]
    timeout: Option<u32>,
}

/// The `command` generator.
pub(super) struct CommandGenerator;

impl CommandGenerator {
    /// The generator as the registry holds it.
    pub(super) fn held() -> Result<GeneratorHeld, ProviderError> {
        let info = Self.describe()?;
        Ok(GeneratorHeld {
            backend: Backend::Native(std::sync::Arc::new(Self)),
            extension: None,
            info,
        })
    }
}

fn internal(why: impl AsRef<str>) -> ProviderError {
    canoa::status::internal(why)
}

/// The text of what a program printed, or why it is none.
fn text_of(bytes: Vec<u8>) -> Result<String, ProviderError> {
    String::from_utf8(bytes).map_err(|_| internal("the catalog is not text"))
}

/// Reads one stream until it ends or `deadline` passes, giving each piece to `take`; `false` when the deadline came first.
fn drain(
    mut read: impl FnMut(&mut [u8], u64) -> Result<i64, ProviderError>,
    deadline: Instant,
    mut take: impl FnMut(&[u8]),
) -> Result<bool, ProviderError> {
    let mut buffer = vec![0_u8; 16 * 1024];
    loop {
        if Instant::now() > deadline {
            return Ok(false);
        }
        let got = read(&mut buffer, 200)?;
        match got {
            0 => return Ok(true),
            READ_TIMED_OUT | READ_LINK_DOWN => {}
            n if n > 0 => take(&buffer[..usize::try_from(n).unwrap_or(0)]),
            _ => {}
        }
    }
}

impl CatalogGeneratorProvider for CommandGenerator {
    fn describe(&self) -> Result<GeneratorInfo, ProviderError> {
        let mut info = GeneratorInfo::new("command", "Run a program").asking::<CommandValues>()?;
        info.help = Some(
            "A program and its arguments, run each time the catalog is refreshed, as you, with your environment. What it prints is the catalog, in the form the type reads."
                .to_owned(),
        );
        info.source = SourceUse::Absent;
        Ok(info)
    }

    fn generate(
        &self,
        request: GenerateRequest,
        spawner: Object<dyn ProcessSpawner>,
        _host: Object<dyn ContentHost>,
    ) -> Result<String, ProviderError> {
        let fields: serde_json::Value = serde_json::from_str(&request.fields)
            .map_err(|_| internal("the entry's fields are not a JSON object"))?;
        let words: Vec<&str> = fields
            .get("command")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .filter_map(|word| word.as_str())
            .collect();
        let Some((program, args)) = words
            .split_first()
            .filter(|(program, _)| !program.is_empty())
        else {
            return Err(internal("the command is a program, then its arguments"));
        };
        let seconds = match fields.get("timeout") {
            None | Some(serde_json::Value::Null) => DEFAULT_SECONDS,
            Some(value) => value
                .as_u64()
                .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
                .ok_or_else(|| internal("the time it may run is a whole number of seconds"))?,
        };
        let dir = fields
            .get("cwd")
            .and_then(|value| value.as_str())
            .filter(|dir| !dir.is_empty());
        let file = fields
            .get("file")
            .and_then(|value| value.as_str())
            .filter(|file| !file.is_empty());
        let mut spec = SpawnSpec::new(*program);
        spec.args = args.iter().map(|arg| (*arg).to_owned()).collect();
        spec.cwd = dir.map(str::to_owned);
        let process = spawner
            .spawn(spec)
            .map_err(|error| internal(format!("{program} could not start: {}", error.message())))?;
        // The program has no input: it reads the end at once.
        let _ = process.close_input();
        let deadline = Instant::now() + Duration::from_secs(seconds);
        let mut printed: Vec<u8> = Vec::new();
        let mut total = 0_usize;
        let mut said: Vec<u8> = Vec::new();
        let ended = drain(
            |buffer, wait| process.read(buffer, wait),
            deadline,
            |piece| {
                total += piece.len();
                // Past what is kept the program is still read, so one that prints on never waits for ever on a full pipe.
                if printed.len() <= MOST_BYTES {
                    printed.extend_from_slice(piece);
                }
            },
        )? && drain(
            |buffer, wait| process.read_error(buffer, wait),
            deadline,
            |piece| {
                said.extend_from_slice(piece);
                if said.len() > TAIL_BYTES {
                    said.drain(..said.len() - TAIL_BYTES);
                }
            },
        )?;
        let tail = String::from_utf8_lossy(&said).trim().to_owned();
        let why = |what: String| {
            if tail.is_empty() {
                what
            } else {
                format!("{what}:\n{tail}")
            }
        };
        let left = u64::try_from(
            deadline
                .saturating_duration_since(Instant::now())
                .as_millis(),
        )
        .unwrap_or(u64::MAX)
        .max(1);
        let exit = if ended { process.wait(left).ok() } else { None };
        let Some(exit) = exit.filter(|exit| !exit.running) else {
            let _ = process.kill();
            return Err(internal(why(format!(
                "{program} was stopped after {seconds} s"
            ))));
        };
        if exit.code != Some(0) {
            let how = exit.code.map_or_else(
                || "was ended by a signal".to_owned(),
                |code| format!("ended with exit code {code}"),
            );
            return Err(internal(why(format!("{program} {how}"))));
        }
        let bytes = if let Some(file) = file {
            let path = if super::file::path_of(file).is_ok_and(|path| path.is_absolute())
                || dir.is_none()
            {
                file.to_owned()
            } else {
                format!(
                    "{}/{file}",
                    dir.unwrap_or_default().trim_end_matches(['/', '\\'])
                )
            };
            let bytes = spawner
                .read_file(&path, MOST_BYTES as u64 + 1)
                .map_err(|error| internal(format!("{path} cannot be read: {}", error.message())))?;
            bytes.expose().as_slice().to_vec()
        } else {
            if total > MOST_BYTES {
                return Err(internal(format!(
                    "the catalog holds more than the {MOST_BYTES} bytes this host reads"
                )));
            }
            printed
        };
        if bytes.is_empty() {
            return Err(internal(why(format!("{program} printed no catalog"))));
        }
        text_of(bytes)
    }
}
