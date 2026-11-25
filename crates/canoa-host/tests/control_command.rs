//! `canoa control`: one request of the control channel answered as the owner over canoa.cfg's files, with no host running, which is how
//! an application reads and changes its configuration while none serves. A file that is not there yet is made by the first change.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::path::PathBuf;
use std::sync::Mutex;

use canoa_host::cli::{self, Cli, Front, Program};
use clap::Parser as _;
use serde_json::{Value, json};

struct Printed(Mutex<Vec<String>>);

impl Front for Printed {
    fn print(&self, line: &str) -> std::io::Result<()> {
        self.0.lock().expect("not poisoned").push(line.to_owned());
        Ok(())
    }

    fn say(&self, _line: &str) {}

    fn when_input_closes(
        &self,
        _status: u8,
        _then: Box<dyn FnOnce() + Send>,
    ) -> std::io::Result<()> {
        Ok(())
    }

    fn program(&self) -> std::io::Result<Program> {
        Ok(Program::new(PathBuf::from("canoa"), Vec::new()))
    }

    fn home(&self) -> Option<PathBuf> {
        None
    }

    fn holder(&self) -> &str {
        "a test"
    }

    fn ask(&self, _prompt: &str, _secret: bool) -> std::io::Result<Option<String>> {
        Ok(None)
    }
}

fn ask(config: &str, method: &str, params: Value) -> Value {
    let request =
        json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params}).to_string();
    let front = Printed(Mutex::new(Vec::new()));
    let cli =
        Cli::try_parse_from(["canoa", "control", "--config", config, &request]).expect("parses");
    cli::run(cli, &front).expect("answered");
    let lines = front.0.lock().expect("not poisoned");
    serde_json::from_str(lines.last().expect("an answer")).expect("JSON")
}

fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("canoa-control-cmd-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("directory");
    dir
}

#[test]
fn a_file_that_is_not_there_is_read_empty_and_made_by_the_first_change() {
    let dir = dir("first");
    let file = dir.join("canoa.cfg");
    let spec = file.to_string_lossy().into_owned();
    let read = ask(&spec, "config/layers", json!({}));
    assert_eq!(read["id"], 7);
    let layers = read["result"]["layers"].as_array().expect("layers");
    assert_eq!(layers.len(), 1, "{read}");
    assert_eq!(layers[0]["writable"], true);
    assert_eq!(layers[0]["document"], json!({}));
    assert!(!file.exists());

    let changed = ask(
        &spec,
        "config/change",
        json!({"ops": [{"path": ["appearance"], "value": {"brightness": "light"}}]}),
    );
    assert!(changed["result"]["revision"].is_string(), "{changed}");
    assert!(
        std::fs::read_to_string(&file)
            .expect("made")
            .contains("brightness = \"light\"")
    );
    let again = ask(&spec, "config/layers", json!({}));
    assert_eq!(
        again["result"]["layers"][0]["document"]["appearance"]["brightness"],
        "light"
    );
    let removed = ask(
        &spec,
        "config/change",
        json!({"ops": [{"path": ["appearance"]}]}),
    );
    assert!(removed["result"].is_object(), "{removed}");
    assert!(
        !std::fs::read_to_string(&file)
            .expect("read")
            .contains("appearance")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_layers_are_listed_lowest_first_and_only_the_last_is_written() {
    let dir = dir("layers");
    let (low, own) = (dir.join("low.cfg"), dir.join("own.cfg"));
    std::fs::write(&low, "[sessions]\nkeep = 500\n").expect("low");
    let separator = if cfg!(windows) { ';' } else { ':' };
    let spec = format!("{}{separator}{}", low.display(), own.display());
    let read = ask(&spec, "config/layers", json!({}));
    let layers = read["result"]["layers"].as_array().expect("layers");
    assert_eq!(layers.len(), 2, "{read}");
    assert_eq!(layers[0]["document"]["sessions"]["keep"], 500);
    assert_eq!(layers[0]["writable"], false);
    assert_eq!(layers[1]["writable"], true);
    let refused = ask(
        &spec,
        "config/change",
        json!({"ops": [{"path": ["sessions"]}]}),
    );
    assert!(
        refused["error"]["message"]
            .as_str()
            .is_some_and(|why| why.contains("low.cfg") && why.contains("remove it there")),
        "{refused}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
