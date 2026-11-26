//! `content/schemes`: the URI schemes this host reads, the provider that serves each and who brought it, the catalog generators, and
//! the providers a library offered that this host does not run, with the reason.

use serde_json::{Value, json};

use super::Control;

type Outcome = Result<Value, (i32, String)>;

impl Control {
    /// `content/schemes` `{}` -> `{"schema": 1, "schemes": [{"scheme", "grammar", "title", "provider", "extension"?, "lists",
    /// "resolves", "state": "on" | "off"}], "generators": [{"id", "title", "help"?, "extension"?, "types", "source", "grammar"?,
    /// "schema"?, "form"?}], "refused": [{"kind", "id", "extension"?, "why"}]}`. A provider `[host.disabled]` turns off stays
    /// listed with `state` `off`. Needs `config.read`.
    pub(super) fn content_schemes(&self, _params: &Value) -> Outcome {
        let sources = self.catalog_sources();
        let content = sources.content();
        let schemes: Vec<Value> = content
            .schemes()
            .into_iter()
            .map(|row| {
                json!({
                    "scheme": row.scheme,
                    "grammar": row.grammar,
                    "title": row.title,
                    "provider": row.provider,
                    "extension": row.extension,
                    "lists": row.lists,
                    "resolves": row.resolves,
                    "state": if row.on { "on" } else { "off" },
                })
            })
            .collect();
        let generators: Vec<Value> = content
            .generators()
            .map(|(info, extension)| {
                json!({
                    "id": info.id,
                    "title": info.title,
                    "help": info.help,
                    "extension": extension,
                    "types": info.types,
                    "source": info.source.word(),
                    "grammar": info.grammar,
                    "schema": info.schema.as_ref().and_then(|schema| crate::forms::json_of(schema).ok()),
                    "form": info.form.as_ref().and_then(|form| crate::forms::json_of(form).ok()),
                })
            })
            .collect();
        let refused: Vec<Value> = content
            .refused()
            .iter()
            .map(|each| {
                json!({
                    "kind": each.kind,
                    "id": each.id,
                    "extension": each.extension,
                    "why": each.why,
                })
            })
            .collect();
        Ok(json!({"schema": 1, "schemes": schemes, "generators": generators, "refused": refused}))
    }
}
