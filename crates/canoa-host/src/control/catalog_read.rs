//! `store/catalog`: the entries of one catalog of the Skills or MCP servers type, read by the `catalog_source` provider canoa.cfg
//! names for it. The place a catalog is read from is always canoa.cfg's, never the caller's: a key names a label.
//!
//! `read` is the type's own reader (a place git fetches and scans for `SKILL.md`; a registry's address or a git file of its list
//! shape); any other provider, a program or an extension's, answers with text in the form the type documents, and the host
//! reads that as data.

use serde_json::{Value, json};
use sha2::Digest as _;

use super::{Caller, Control, NOT_FOUND, REFUSED, bad, text_param};
use crate::store::catalogs::{self, Entry, TypeCatalogs, built_in_types, table_of};
use crate::store::listings::{self, SkillSpec};

/// How many servers one page of a registry asks for.
const PAGE: u32 = 30;

type Failure = (i32, String);
type Outcome = Result<Value, Failure>;

impl Control {
    /// `store/catalog` `{"type": "skills" | "mcps", "label", "search"?, "cursor"?}` -> for Skills `{"type", "label", "provider",
    /// "commit"?, "skills": [{"name", "description", "source", "path", "ref"?, "problem"?}], "skipped", "truncated"}`, where `source`,
    /// `ref` and `path` say where the skill is and `commit` is the one a scan of the source read; for MCP servers `{"type",
    /// "label", "provider", "servers": [{"server", "_meta"?}], "metadata": {"nextCursor"}}`, the registry's list shape, narrowed
    /// to `search`. Overlays and extensions are read through `store/index`. A catalog that is not `read` runs its provider each
    /// time; a failure is the provider's reason. Needs `overlay.write`, as `skill/scan` and `registry/fetch` do.
    pub(super) fn store_catalog(&self, caller: &Caller, params: &Value) -> Outcome {
        let id = text_param(params, "type")?.ok_or_else(|| bad("type is needed"))?;
        let label = text_param(params, "label")?.ok_or_else(|| bad("label is needed"))?;
        let search = text_param(params, "search")?.unwrap_or_default();
        let cursor = text_param(params, "cursor")?.filter(|cursor| !cursor.is_empty());
        let refresh = match params.get("refresh") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(value)) => *value,
            Some(_) => return Err(bad("refresh is true or false")),
        };
        // A scope whose catalogs answer in one of the host's formats (a type's models, a program's releases) is read by its
        // entry's source and kept on disk.
        if let Ok(stores) = self.stores()
            && let Some(scope) = stores
                .catalog_types()
                .into_iter()
                .find(|scope| scope.id == id && scope.format.is_some())
        {
            let store = self.store_section();
            let entry = catalogs::find(&scope, table_of(&store, &scope.id), label)
                .map_err(|why| (NOT_FOUND, why))?;
            let reading = stores
                .catalog_run()
                .read(&scope, &entry, refresh)
                .map_err(|why| (REFUSED, why))?;
            return crate::store::catalog_view::view(&scope, &entry, &reading).map_err(|why| {
                (
                    REFUSED,
                    format!("{} gave no catalog: {why}", entry.provider),
                )
            });
        }
        let kind: TypeCatalogs = match id {
            "skills" | "mcps" => built_in_types()
                .into_iter()
                .find(|kind| kind.id == id)
                .ok_or_else(|| (NOT_FOUND, format!("this host has no Store type {id:?}")))?,
            "overlays" | "extensions" => {
                return Err(bad(
                    "overlays and extensions are read through store/index, which lists their entries",
                ));
            }
            _ => {
                return Err((
                    NOT_FOUND,
                    format!("this host reads no catalogs of the type {id:?} through store/catalog"),
                ));
            }
        };
        let store = self.store_section();
        let entry = catalogs::find(&kind, table_of(&store, &kind.id), label)
            .map_err(|why| (NOT_FOUND, why))?;
        // What a catalog lists is kept for `[host.store] catalog_ttl`, per caller, since what a caller may read differs.
        let key = format!(
            "{}\n{id}\n{label}\n{search}\n{}\n{}",
            caller.owner(),
            cursor.unwrap_or_default(),
            entry.source().unwrap_or_default()
        );
        // The app's own list of a catalog (no search, no page) is asked again in the background once it is past its time.
        if matches!(caller, Caller::Owner) && search.is_empty() && cursor.is_none() {
            self.caches
                .catalog
                .note(&key, &json!({"type": id, "label": label, "refresh": true}));
        }
        self.caches.catalog.get(&key, refresh, || {
            let mut answer = if entry.is_document() {
                match id {
                    "skills" => self.read_skills(caller, &entry)?,
                    _ => self.read_mcps(caller, &entry, search, cursor)?,
                }
            } else {
                self.read_provided(&kind, &entry, search)?
            };
            answer["type"] = json!(id);
            answer["label"] = json!(entry.label);
            answer["provider"] = json!(entry.provider);
            Ok(answer)
        })
    }

    /// The skills a source holds, found as `npx skills` finds them. A source the person typed is fetched with their own git
    /// settings, credentials and keys; the one Canoa offers is not.
    fn read_skills(&self, caller: &Caller, entry: &Entry) -> Result<Value, Failure> {
        let text = entry
            .source()
            .ok_or_else(|| bad("a skills source is written in the source field"))?;
        let spec = SkillSpec::parse(text).map_err(|why| (REFUSED, why))?;
        let own = !catalogs::is_default_skill_source(&entry.label, &spec.spec);
        let mut asked = json!({"repo": spec.repo, "path": spec.path, "own": own});
        if let Some(reference) = &spec.reference {
            asked["ref"] = json!(reference);
        }
        let scanned = self
            .overlays
            .skill_scan(&asked, &|repo| self.may_read_source(caller, repo))?;
        let skills: Vec<Value> = scanned["skills"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|skill| {
                spec.only.as_ref().is_none_or(|only| {
                    skill["name"]
                        .as_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case(only))
                })
            })
            .map(|skill| {
                let mut skill = skill.clone();
                skill["source"] = json!(spec.spec);
                skill
            })
            .collect();
        Ok(json!({
            "commit": scanned["commit"],
            "skills": skills,
            "skipped": scanned["skipped"],
            "truncated": scanned["truncated"],
        }))
    }

    /// One page of a registry's list: an address that serves the registry's API, or a git source of a file in its list shape.
    fn read_mcps(
        &self,
        caller: &Caller,
        entry: &Entry,
        search: &str,
        cursor: Option<&str>,
    ) -> Result<Value, Failure> {
        let source = entry
            .source()
            .ok_or_else(|| bad("a registry is written in the source field"))?;
        let body = if source.contains(['@', '#']) {
            self.read_git_registry(caller, source)?
        } else {
            use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
            let mut url = format!("{}/v0.1/servers?limit={PAGE}", source.trim_end_matches('/'));
            if let Some(cursor) = cursor {
                url.push_str(&format!(
                    "&cursor={}",
                    utf8_percent_encode(cursor, NON_ALPHANUMERIC)
                ));
            }
            if !search.trim().is_empty() {
                url.push_str(&format!(
                    "&search={}",
                    utf8_percent_encode(search.trim(), NON_ALPHANUMERIC)
                ));
            }
            // The address is canoa.cfg's, so any key may ask for it: it is the person's own choice of registry.
            self.registry
                .fetch(&json!({"url": url}), None)?
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        listings::mcp_page(&body, search)
            .map(|page| page.to_value())
            .map_err(|why| (REFUSED, format!("The registry gave no list: {why}")))
    }

    /// The text of the file a git registry names, at the commit its ref names now.
    fn read_git_registry(&self, caller: &Caller, source: &str) -> Result<String, Failure> {
        if listings::holds_password(source) {
            return Err((
                REFUSED,
                "A registry's address may not hold a password; use a credential helper".to_owned(),
            ));
        }
        let canonical = if source.starts_with("git+") {
            source.to_owned()
        } else {
            format!("git+{source}")
        };
        let Ok(crate::profile::OverlaySource::Git {
            repo,
            reference,
            path,
        }) = crate::profile::OverlaySource::parse_git(&canonical)
        else {
            return Err((
                REFUSED,
                "A git registry is <repository>@<ref>#<file>, a source git may fetch".to_owned(),
            ));
        };
        if path.is_empty() {
            return Err((REFUSED, "A git registry names its file after #".to_owned()));
        }
        // Kept where the catalogs are, under a name fixed by its address, so reading it again fetches only what moved.
        let label = format!(
            "registry-{}",
            &data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(source.as_bytes()))[..16]
        );
        let fetched = self.overlays.catalog_fetch(
            &json!({"label": label, "extensions": false, "repo": repo, "ref": reference, "path": path}),
            &|repo| self.may_read_source(caller, repo),
        )?;
        let commit = fetched
            .get("commit")
            .and_then(Value::as_str)
            .ok_or_else(|| (REFUSED, "the fetch named no commit".to_owned()))?;
        let read = self.overlays.catalog_read(&json!({
            "label": label, "extensions": false, "commit": commit, "paths": [path],
            "maxBytes": super::registry::MAX_REGISTRY_BYTES,
        }))?;
        let file = read
            .get("files")
            .and_then(|files| files.get(&path))
            .filter(|file| file.is_object())
            .ok_or_else(|| (REFUSED, format!("It has no file {path} at that commit")))?;
        let data = file.get("data").and_then(Value::as_str).ok_or_else(|| {
            (
                REFUSED,
                "The registry's file is larger than Canoa reads".to_owned(),
            )
        })?;
        let bytes = data_encoding::BASE64.decode(data.as_bytes()).map_err(|_| {
            (
                REFUSED,
                "The registry's file did not come through".to_owned(),
            )
        })?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// A catalog a provider other than `read` answers: a program the person named, or an extension's source.
    fn read_provided(
        &self,
        kind: &TypeCatalogs,
        entry: &Entry,
        search: &str,
    ) -> Result<Value, Failure> {
        let text = self
            .catalog_sources()
            .make(kind.id.as_str(), entry)
            .map_err(|why| (REFUSED, why))?;
        if kind.id == "skills" {
            let listing = listings::skills_listing(&text).map_err(|why| {
                (
                    REFUSED,
                    format!("{} gave no skills listing: {why}", entry.maker()),
                )
            })?;
            Ok(json!({
                "commit": Value::Null,
                "skills": listing.skills.iter().map(listings::ListedSkill::to_value).collect::<Vec<_>>(),
                "skipped": listing.skipped,
                "truncated": listing.truncated,
            }))
        } else {
            listings::mcp_page(&text, search)
                .map(|page| page.to_value())
                .map_err(|why| {
                    (
                        REFUSED,
                        format!("{} gave no registry list: {why}", entry.maker()),
                    )
                })
        }
    }
}
