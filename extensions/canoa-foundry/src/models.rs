//! Foundry Local's models as the command line lists and moves them: which arguments ask, and what the answers say.
//!
//! The command line is the only source: a model, an alias or a device build is named here only as the program's own answer names
//! it, and one model is the variants that share an alias ([`group`]).
//!
//! The command line's `-o json` answers are read for what they name, not for one fixed shape: a list of rows, or an object
//! holding the list under `models`, `items`, `data` or `results`. A row's identity is the first of `id`, `variantId`, `modelId`
//! and `name`; the other keys are optional. Nothing here starts a process.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use serde_json::Value;

/// Why a Foundry model action could not be prepared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The model name does not match the allowed characters and length.
    InvalidModel,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidModel => {
                f.write_str("a model is 1 to 128 letters, digits and . _ - : + /")
            }
        }
    }
}

impl core::error::Error for Error {}

/// What the command line is asked to do with one model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelAction {
    /// `foundry model download`: fetch the model into the cache, which can be many gigabytes.
    Download,
    /// `foundry cache rm`: take the model out of the cache.
    Remove,
    /// `foundry model load`: put a downloaded model in memory, where it stays until the service stops.
    Load,
    /// `foundry model unload`: take it out of memory again (the service keeps what the runtime reserved).
    Unload,
}

impl ModelAction {
    /// The action named `word` (`download`, `load` or `unload`).
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "download" => Some(Self::Download),
            "remove" => Some(Self::Remove),
            "load" => Some(Self::Load),
            "unload" => Some(Self::Unload),
            _ => None,
        }
    }

    /// The word the command line spells it with.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Download => "download",
            Self::Remove => "remove",
            Self::Load => "load",
            Self::Unload => "unload",
        }
    }
}

/// Whether `id` is a model name the command line takes: 1 to 128 of letters, digits and `.`, `_`, `-`, `:`, `+`, `/`, and not
/// opening with a dash, so it can never be read as an option.
#[must_use]
pub fn valid_model(id: &str) -> bool {
    (1..=128).contains(&id.len())
        && !id.starts_with('-')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-:+/".contains(&b))
}

/// The arguments that make the program do `action` to `model` (an alias or a variant's id), with its answer as JSON.
///
/// # Errors
///
/// `model` is not a model name.
pub fn action_args(action: ModelAction, model: &str) -> Result<Vec<String>, Error> {
    if !valid_model(model) {
        return Err(Error::InvalidModel);
    }
    let words: [&str; 5] = match action {
        ModelAction::Remove => ["cache", "rm", model, "-o", "json"],
        ModelAction::Download | ModelAction::Load | ModelAction::Unload => {
            ["model", action.word(), model, "-o", "json"]
        }
    };
    Ok(words.map(str::to_owned).to_vec())
}

/// The arguments that list the catalog's variants, those in the cache (`foundry cache list`), or those in memory (`loaded`).
#[must_use]
pub fn list_args(which: Listing) -> Vec<String> {
    let words: &[&str] = match which {
        Listing::All => &["model", "list", "--variants"],
        Listing::Cached => &["cache", "list", "--variants"],
        Listing::Loaded => &["model", "list", "--variants", "--loaded"],
    };
    let mut args: Vec<String> = words.iter().map(|word| (*word).to_owned()).collect();
    args.extend(["-o", "json"].map(str::to_owned));
    args
}

/// Which models a listing asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Listing {
    /// Every variant the catalog offers.
    All,
    /// The ones downloaded.
    Cached,
    /// The ones in memory.
    Loaded,
}

/// One row of a listing.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Row {
    /// The name `download`, `load` and `unload` take.
    pub id: String,
    /// The shorter name several variants share.
    pub alias: Option<String>,
    /// What it is for: chat, speech, embedding.
    pub task: Option<String>,
    /// The hardware it runs on: cpu, gpu, npu.
    pub device: Option<String>,
    /// The runtime that runs it on that hardware, as the program names it.
    pub provider: Option<String>,
    /// Its licence, as the catalog states it.
    pub license: Option<String>,
    /// Its size on disk, in megabytes.
    pub size_mb: Option<u64>,
}

fn text(row: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| row.get(*key).and_then(Value::as_str))
        .filter(|word| !word.is_empty() && word.len() <= 256)
        .map(str::to_owned)
}

fn number(row: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        let value = row.get(*key)?;
        value
            .as_u64()
            .or_else(|| value.as_f64().filter(|n| *n >= 0.0).map(|n| n as u64))
    })
}

/// The rows a command line answer holds; an answer that is neither a list nor an object with one holds none.
#[must_use]
pub fn rows(answer: &Value) -> Vec<Row> {
    let list = match answer {
        Value::Array(list) => Some(list),
        // Foundry Local 0.14 answers `{"variants": [...]}`.
        Value::Object(map) => ["variants", "models", "items", "data", "results"]
            .iter()
            .find_map(|key| map.get(*key).and_then(Value::as_array)),
        _ => None,
    };
    list.into_iter()
        .flatten()
        .filter_map(|row| {
            let id = text(row, &["id", "variantId", "modelId", "name"])?;
            valid_model(&id).then(|| Row {
                alias: text(row, &["alias"]).filter(|alias| *alias != id),
                task: text(row, &["task", "type"]),
                device: text(row, &["device", "deviceType", "device_type"]),
                provider: text(row, &["executionProvider", "execution_provider"]),
                license: text(row, &["license"]),
                size_mb: number(row, &["fileSizeMb", "sizeMb", "size_mb", "fileSizeMB"]),
                id,
            })
        })
        .collect()
}

/// What a catalog task says a model can do, as the ids a picker filters by: `chat` for the models that answer a prompt, `embed` for
/// the ones that make vectors and `speech_to_text` for the ones that transcribe. A task this does not know says nothing, so the model
/// is listed as one that reports no capabilities, never guessed to be a chat model.
#[must_use]
pub fn capabilities_of(task: &str) -> Vec<&'static str> {
    match task.trim().to_ascii_lowercase().as_str() {
        "chat" | "chat-completion" | "chat-completions" | "text-generation" => vec!["chat"],
        "embedding" | "embeddings" | "text-embedding" => vec!["embed"],
        "speech" | "speech-to-text" | "audio-transcription" | "transcription" => {
            vec!["speech_to_text"]
        }
        _ => Vec::new(),
    }
}

impl Row {
    /// What the row's task says the model can do ([`capabilities_of`]); empty when it names none or one this does not know.
    #[must_use]
    pub fn capabilities(&self) -> Vec<&'static str> {
        self.task
            .as_deref()
            .map(capabilities_of)
            .unwrap_or_default()
    }
}

/// A model as the person is shown it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Model {
    /// The row.
    pub row: Row,
    /// Whether it is downloaded.
    pub cached: bool,
    /// Whether it is in memory.
    pub loaded: bool,
}

/// The catalog's rows with what is downloaded and what is in memory marked; a downloaded or loaded model the catalog does not
/// list (one the person registered) is kept. Sorted: loaded first, then downloaded, then by name.
#[must_use]
pub fn merge(all: Vec<Row>, cached: &[Row], loaded: &[Row]) -> Vec<Model> {
    let mut models: Vec<Model> = Vec::with_capacity(all.len());
    for row in all
        .into_iter()
        .chain(cached.iter().cloned())
        .chain(loaded.iter().cloned())
    {
        if !models.iter().any(|model| model.row.id == row.id) {
            models.push(Model {
                cached: cached.iter().any(|each| each.id == row.id),
                loaded: loaded.iter().any(|each| each.id == row.id),
                row,
            });
        }
    }
    models.sort_by(|a, b| {
        b.loaded
            .cmp(&a.loaded)
            .then(b.cached.cmp(&a.cached))
            .then_with(|| a.row.id.cmp(&b.row.id))
    });
    models
}

/// One model as the person is shown it: the variants that share an alias, each the build for some hardware. A variant with no alias
/// is a model of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Entry {
    /// The alias the variants share (or the one variant's own name): what `download` and `cache rm` take for the model as a whole,
    /// where the program picks the variant that suits this computer.
    pub id: String,
    /// The builds, by hardware and then name.
    pub variants: Vec<Model>,
}

impl Entry {
    /// Whether any variant is downloaded.
    #[must_use]
    pub fn cached(&self) -> bool {
        self.variants.iter().any(|variant| variant.cached)
    }

    /// Whether any variant is in memory.
    #[must_use]
    pub fn loaded(&self) -> bool {
        self.variants.iter().any(|variant| variant.loaded)
    }

    /// What the model is for, as its first variant that names one.
    #[must_use]
    pub fn task(&self) -> Option<&str> {
        self.variants
            .iter()
            .find_map(|variant| variant.row.task.as_deref())
    }

    /// Its licence, as its first variant that states one does.
    #[must_use]
    pub fn license(&self) -> Option<&str> {
        self.variants
            .iter()
            .find_map(|variant| variant.row.license.as_deref())
    }

    /// The hardware its variants run on, each once, in lower case, in the order the variants have them.
    #[must_use]
    pub fn devices(&self) -> Vec<String> {
        let mut seen: Vec<String> = Vec::new();
        for device in self
            .variants
            .iter()
            .filter_map(|variant| variant.row.device.as_deref())
        {
            let device = device.to_ascii_lowercase();
            if !seen.contains(&device) {
                seen.push(device);
            }
        }
        seen
    }

    /// The smallest and the largest size of its variants, in megabytes; none when none states one.
    #[must_use]
    pub fn sizes_mb(&self) -> Option<(u64, u64)> {
        let sizes = self
            .variants
            .iter()
            .filter_map(|variant| variant.row.size_mb);
        Some((sizes.clone().min()?, sizes.max()?))
    }
}

/// The models grouped the way the person is shown them: one entry for each alias, its variants inside. Loaded entries first, then
/// downloaded, then by name.
#[must_use]
pub fn group(models: Vec<Model>) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    for model in models {
        let id = model
            .row
            .alias
            .clone()
            .unwrap_or_else(|| model.row.id.clone());
        match entries.iter_mut().find(|entry| entry.id == id) {
            Some(entry) => entry.variants.push(model),
            None => entries.push(Entry {
                id,
                variants: vec![model],
            }),
        }
    }
    for entry in &mut entries {
        let device = |model: &Model| {
            model
                .row
                .device
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase()
        };
        entry.variants.sort_by(|a, b| {
            device(a)
                .cmp(&device(b))
                .then_with(|| a.row.id.cmp(&b.row.id))
        });
    }
    entries.sort_by(|a, b| {
        b.loaded()
            .cmp(&a.loaded())
            .then(b.cached().cmp(&a.cached()))
            .then_with(|| a.id.cmp(&b.id))
    });
    entries
}

/// What the program said when it refused, from its `-o json` error `{"error": {"code", "message", "hint"}}` found in `out` or
/// `err`: `(code, message, hint)`, each when it said one.
#[must_use]
pub fn error_of(out: &str, err: &str) -> Option<(Option<String>, Option<String>, Option<String>)> {
    for text in [out, err] {
        let mut at = 0;
        for line in text.split_inclusive('\n') {
            let start = at + (line.len() - line.trim_start().len());
            at += line.len();
            if !line.trim_start().starts_with('{') {
                continue;
            }
            let Some(Ok(found)) = serde_json::Deserializer::from_str(&text[start..])
                .into_iter::<Value>()
                .next()
            else {
                continue;
            };
            if let Some(error) = found.get("error").filter(|error| error.is_object()) {
                let word = |key: &str| {
                    error
                        .get(key)
                        .and_then(Value::as_str)
                        .map(|word| plain(word, 400))
                        .filter(|word| !word.is_empty())
                };
                return Some((word("code"), word("message"), word("hint")));
            }
        }
    }
    None
}

/// A refusal in words for the person: the program's own message and hint when it gave an error in JSON, with its exit code and
/// error code, else what its exit code means for what was asked (3 is an unknown model, 2 a request the program did not accept,
/// which for a removal is a model that is not in the cache). None when the program said neither, so its last words are shown.
#[must_use]
pub fn refusal_words(
    action: Option<ModelAction>,
    model: Option<&str>,
    code: Option<i64>,
    out: &str,
    err: &str,
) -> Option<String> {
    let meaning = match (code, action) {
        (Some(3), _) => model.map(|named| format!("Foundry Local does not know {named}")),
        (Some(2), Some(ModelAction::Remove)) => {
            model.map(|named| format!("{named} is not in Foundry Local's cache"))
        }
        (Some(2), _) => Some("Foundry Local did not accept the request".to_owned()),
        _ => None,
    };
    let given = error_of(out, err);
    let message = given.as_ref().and_then(|(_, message, hint)| {
        let message = message.as_deref()?;
        Some(match hint {
            Some(hint) => format!("{message} {hint}"),
            None => message.to_owned(),
        })
    });
    let word = given.as_ref().and_then(|(word, _, _)| word.as_deref());
    let tag = match (code, word) {
        (Some(code), Some(word)) => format!(" (exit {code}, {word})"),
        (Some(code), None) => format!(" (exit {code})"),
        (None, _) => String::new(),
    };
    message.or(meaning).map(|said| format!("{said}{tag}"))
}

/// What a finished action says in a line, for the person to read afterwards: the program's own `message` when it answered in
/// JSON, else the last line it printed (a download rewrites its percentage line with carriage returns, so the last piece is the
/// last progress or the closing sentence).
#[must_use]
pub fn outcome_line(out: &str) -> Option<String> {
    if let Some(message) = serde_json::from_str::<Value>(out.trim())
        .ok()
        .and_then(|answer| {
            answer
                .get("message")
                .and_then(Value::as_str)
                .map(|message| plain(message, 300))
        })
        .filter(|text| !text.is_empty())
    {
        return Some(message);
    }
    out.replace('\r', "\n")
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty() && !line.starts_with(['{', '}', '[', ']', '"']))
        .map(|line| plain(line, 300))
        .filter(|text| !text.is_empty())
}

/// What a model's row says of it in one line: `chat, gpu, 9.9 GB`.
#[must_use]
pub fn describe(row: &Row) -> String {
    let mut parts = Vec::new();
    if let Some(task) = &row.task {
        parts.push(task.clone());
    }
    if let Some(device) = &row.device {
        parts.push(device.clone());
    }
    if let Some(mb) = row.size_mb {
        parts.push(if mb >= 1024 {
            format!("{:.1} GB", mb as f64 / 1024.0)
        } else {
            format!("{mb} MB")
        });
    }
    parts.join(", ")
}

/// A line of the program's own words with the escapes a terminal would draw and the control characters taken out, cut at
/// `most` characters, for showing a refusal to the person.
#[must_use]
pub fn plain(text: &str, most: usize) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.next_if_eq(&'[').is_some() {
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else if c == '\n' || !c.is_control() {
            out.push(c);
        }
    }
    let trimmed = out.trim();
    trimmed.chars().take(most).collect::<String>().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_model_name_is_never_an_option() {
        assert!(valid_model("qwen3-14b-generic-gpu:2"));
        assert!(valid_model("openai-whisper-tiny-generic-cpu"));
        for bad in ["", "-o", "--help", "a b", "a;b", "a\nb", &"x".repeat(129)] {
            assert!(!valid_model(bad), "{bad:?}");
            assert!(action_args(ModelAction::Load, bad).is_err());
        }
        assert_eq!(
            action_args(ModelAction::Unload, "m:1").unwrap(),
            ["model", "unload", "m:1", "-o", "json"]
        );
    }

    #[test]
    fn rows_are_read_from_a_list_or_an_object_holding_one() {
        let one = json!({"id": "qwen3-14b-generic-gpu:2", "alias": "qwen3-14b", "task": "chat", "device": "gpu", "fileSizeMb": 10137});
        assert_eq!(rows(&json!([one.clone()])), rows(&json!({"models": [one]})));
        let found =
            rows(&json!([{"id": "a", "alias": "a"}, {"name": "b"}, {"id": "-bad"}, {"x": 1}, 4]));
        let ids: Vec<&str> = found.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
        assert_eq!(
            found[0].alias, None,
            "an alias that is the id says nothing more"
        );
        assert!(rows(&json!("text")).is_empty());
    }

    #[test]
    fn rows_are_read_from_the_variants_foundry_local_0_14_answers() {
        // `foundry model list --variants -o json`, version 0.14.0, one row of it.
        let answer = json!({"variants": [{"alias": "clm-v0.1-8b", "source": "Public", "variantName": "clm-v0.1-8b-generic-gpu",
            "variantId": "clm-v0.1-8b-generic-gpu:2", "type": "Unknown", "device": "Gpu",
            "executionProvider": "WebGPUExecutionProvider", "fileSizeMb": 8544, "cached": false, "license": "apache-2.0"}]});
        let found = rows(&answer);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "clm-v0.1-8b-generic-gpu:2");
        assert_eq!(found[0].alias.as_deref(), Some("clm-v0.1-8b"));
        assert_eq!(found[0].device.as_deref(), Some("Gpu"));
        assert_eq!(found[0].size_mb, Some(8544));
    }

    #[test]
    fn merging_marks_what_is_downloaded_and_loaded_and_keeps_what_the_catalog_lacks() {
        let all = rows(&json!([{"id": "b"}, {"id": "a"}, {"id": "c"}]));
        let cached = rows(&json!([{"id": "a"}, {"id": "mine"}]));
        let loaded = rows(&json!([{"id": "c"}]));
        let models = merge(all, &cached, &loaded);
        let seen: Vec<(&str, bool, bool)> = models
            .iter()
            .map(|m| (m.row.id.as_str(), m.cached, m.loaded))
            .collect();
        assert_eq!(
            seen,
            [
                ("c", false, true),
                ("a", true, false),
                ("mine", true, false),
                ("b", false, false)
            ]
        );
    }

    #[test]
    fn a_row_is_described_in_one_line_and_a_terminal_escape_is_not_shown() {
        let row = &rows(&json!([{"id": "m", "task": "chat", "device": "gpu", "sizeMb": 10137}]))[0];
        assert_eq!(describe(row), "chat, gpu, 9.9 GB");
        assert_eq!(
            plain("\u{1b}[31merror:\u{1b}[0m no space\u{7}", 100),
            "error: no space"
        );
        assert_eq!(plain("abcdef", 3), "abc");
    }

    #[test]
    fn a_catalog_task_says_what_a_model_does_and_one_nobody_knows_says_nothing() {
        assert_eq!(capabilities_of("chat"), ["chat"]);
        assert_eq!(capabilities_of("Embedding"), ["embed"]);
        assert_eq!(capabilities_of("speech"), ["speech_to_text"]);
        assert!(capabilities_of("image-diffusion").is_empty());
        let listed = rows(&json!([
            {"id": "qwen", "task": "chat"},
            {"id": "bge", "task": "embedding"},
            {"id": "mystery"},
        ]));
        let by: Vec<_> = listed.iter().map(Row::capabilities).collect();
        assert_eq!(by, [vec!["chat"], vec!["embed"], Vec::<&str>::new()]);
    }

    #[test]
    fn the_commands_are_the_programs_own_listing_cache_and_download() {
        assert_eq!(
            list_args(Listing::All),
            ["model", "list", "--variants", "-o", "json"]
        );
        assert_eq!(
            list_args(Listing::Cached),
            ["cache", "list", "--variants", "-o", "json"],
            "what is installed is the cache's own listing"
        );
        assert_eq!(
            list_args(Listing::Loaded),
            ["model", "list", "--variants", "--loaded", "-o", "json"]
        );
        assert_eq!(
            action_args(ModelAction::Download, "qwen:2").unwrap(),
            ["model", "download", "qwen:2", "-o", "json"]
        );
        assert_eq!(
            action_args(ModelAction::Remove, "qwen").unwrap(),
            ["cache", "rm", "qwen", "-o", "json"]
        );
        assert_eq!(ModelAction::parse("remove"), Some(ModelAction::Remove));
        assert!(action_args(ModelAction::Remove, "-f").is_err());
    }

    #[test]
    fn builds_that_share_an_alias_are_one_entry_and_a_build_with_none_is_its_own() {
        let found = merge(
            rows(&json!([
                {"variantId": "m-gpu:1", "alias": "m", "device": "Gpu", "type": "chat", "fileSizeMb": 900, "license": "mit"},
                {"variantId": "m-cpu:1", "alias": "m", "device": "Cpu", "fileSizeMb": 300},
                {"variantId": "other:1", "device": "Cpu"},
                {"variantId": "z:1", "alias": "z", "device": "Cpu"}
            ])),
            &rows(&json!([{"variantId": "z:1", "alias": "z"}])),
            &[],
        );
        let entries = group(found);
        let ids: Vec<&str> = entries.iter().map(|entry| entry.id.as_str()).collect();
        assert_eq!(ids, ["z", "m", "other:1"], "downloaded first, then by name");
        let m = &entries[1];
        let builds: Vec<&str> = m.variants.iter().map(|each| each.row.id.as_str()).collect();
        assert_eq!(builds, ["m-cpu:1", "m-gpu:1"], "by hardware");
        assert_eq!(m.devices(), ["cpu", "gpu"]);
        assert_eq!(m.sizes_mb(), Some((300, 900)));
        assert_eq!((m.task(), m.license()), (Some("chat"), Some("mit")));
        assert!(!m.cached() && entries[0].cached());
        assert_eq!(entries[2].sizes_mb(), None);
    }

    #[test]
    fn an_exit_code_and_an_error_in_json_are_put_in_words() {
        let error = r#"{"error":{"code":"MODEL_NOT_FOUND","message":"No such model.","hint":"Try the list."}}"#;
        assert_eq!(
            refusal_words(Some(ModelAction::Download), Some("m"), Some(3), error, "").as_deref(),
            Some("No such model. Try the list. (exit 3, MODEL_NOT_FOUND)")
        );
        assert_eq!(
            refusal_words(Some(ModelAction::Download), Some("m"), Some(3), "", "").as_deref(),
            Some("Foundry Local does not know m (exit 3)")
        );
        assert_eq!(
            refusal_words(Some(ModelAction::Remove), Some("m"), Some(2), "", "").as_deref(),
            Some("m is not in Foundry Local's cache (exit 2)")
        );
        assert_eq!(
            refusal_words(Some(ModelAction::Load), Some("m"), Some(2), "", "").as_deref(),
            Some("Foundry Local did not accept the request (exit 2)")
        );
        assert_eq!(refusal_words(None, None, Some(1), "boom", ""), None);
        assert_eq!(refusal_words(None, None, Some(3), "", ""), None);
        let after_notice = format!("A new version is out.\n{error}\n");
        assert_eq!(
            error_of(&after_notice, "")
                .and_then(|(code, _, _)| code)
                .as_deref(),
            Some("MODEL_NOT_FOUND")
        );
        assert_eq!(error_of("{\"success\": true}", "plain"), None);
    }

    #[test]
    fn what_a_download_printed_last_is_its_closing_sentence_or_its_message() {
        let text = "Downloading model (m, 1 GB)\n\rm: 15%\rm: 62%\rm: 100%\nDownloaded 'm' to the local cache.\n";
        assert_eq!(
            outcome_line(text).as_deref(),
            Some("Downloaded 'm' to the local cache.")
        );
        assert_eq!(
            outcome_line("Downloading model (m, 1 GB)\n\rm: 15%\rm: 62%").as_deref(),
            Some("m: 62%"),
            "a download cut short shows how far it got"
        );
        let pretty = "{\n  \"success\": true,\n  \"message\": \"Removed 'm' from the cache.\"\n}\n";
        assert_eq!(
            outcome_line(pretty).as_deref(),
            Some("Removed 'm' from the cache.")
        );
        assert_eq!(outcome_line(" \n"), None);
    }
}
