//! Foundry Local's models as the command line lists and moves them: which arguments ask, and what the answers say.
//!
//! The command line's `-o json` answers are read for what they name, not for one fixed shape: a list of rows, or an object
//! holding the list under `models`, `items`, `data` or `results`. A row's identity is the first of `id`, `variantId`, `modelId`
//! and `name`; the other keys are optional. Nothing here starts a process.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde_json::Value;

/// What the command line is asked to do with one model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelAction {
    /// `foundry model download`: fetch the model into the cache, which can be many gigabytes.
    Download,
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

/// The arguments that make the program do `action` to `model`, with its answer as JSON.
///
/// # Errors
///
/// `model` is not a model name.
pub fn action_args(action: ModelAction, model: &str) -> Result<Vec<String>, String> {
    if !valid_model(model) {
        return Err("a model is 1 to 128 letters, digits and . _ - : + /".to_owned());
    }
    Ok(["model", action.word(), model, "-o", "json"]
        .map(str::to_owned)
        .to_vec())
}

/// The arguments that list the catalog's variants, those in the cache (`cached`), or those in memory (`loaded`).
#[must_use]
pub fn list_args(which: Listing) -> Vec<String> {
    let mut args: Vec<String> = ["model", "list", "--variants"].map(str::to_owned).to_vec();
    match which {
        Listing::All => {}
        Listing::Cached => args.push("--cached".to_owned()),
        Listing::Loaded => args.push("--loaded".to_owned()),
    }
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
        Value::Object(map) => ["models", "items", "data", "results"]
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
                size_mb: number(row, &["fileSizeMb", "sizeMb", "size_mb", "fileSizeMB"]),
                id,
            })
        })
        .collect()
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
}
