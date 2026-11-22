//! Route adapters and shared request helpers dispatched by [`super::dispatch::ROUTES`].
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths and JSON are std's"
)]

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::worktree::Worktrees;

use super::{
    BAD_PARAMS, Caller, Control, DENIED, Grant, REFUSED, Stores, allow, choices, discover,
    error_line, files, hosting, looks, panel_tabs, profile_items, providers, registry, secrets,
    settings_sections, speech_offers, usage_read,
};

impl Control {
    pub(super) fn r_ledger_list(
        &self,
        caller: &Caller,
        _: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        self.list(caller, params)
    }

    pub(super) fn r_running_sessions(
        &self,
        caller: &Caller,
        _: &str,
        _: &Value,
    ) -> Result<Value, (i32, String)> {
        self.running_sessions(caller)
    }

    pub(super) fn r_ledger_put(
        &self,
        caller: &Caller,
        _: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        self.put(caller, params)
    }

    pub(super) fn r_ledger_remove(
        &self,
        caller: &Caller,
        _: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        self.remove(caller, params)
    }

    pub(super) fn r_ledger_clear(
        &self,
        caller: &Caller,
        _: &str,
        _: &Value,
    ) -> Result<Value, (i32, String)> {
        self.clear(caller)
    }

    pub(super) fn r_forget(
        &self,
        caller: &Caller,
        _: &str,
        _: &Value,
    ) -> Result<Value, (i32, String)> {
        self.forget(caller)
    }

    pub(super) fn r_grants(
        &self,
        caller: &Caller,
        _: &str,
        _: &Value,
    ) -> Result<Value, (i32, String)> {
        Ok(caller.grants_json())
    }

    pub(super) fn r_fs_read(
        &self,
        _: &Caller,
        _: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        self.files.read(params)
    }

    pub(super) fn r_state(
        &self,
        _: &Caller,
        method: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        match method {
            "state/get" => self.state.get(params),
            "state/put" => self.state.put(params),
            "state/append" => self.state.append(params),
            "state/list" => self.state.list(params),
            _ => self.state.remove(params),
        }
    }

    pub(super) fn r_files(
        &self,
        caller: &Caller,
        method: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        match method {
            "fs/list" => self.files.list(caller, params),
            "fs/info" => self.files.info(caller, params),
            "fs/realpath" => self.files.realpath(caller, params),
            "fs/guards" => self.files.guards(),
            "fs/repo_root" => self.files.repo_root(caller, params),
            "fs/mentions" => self.files.mentions(caller, params),
            "fs/resolve" => self.files.resolve(caller, params),
            "fs/scratch" => self.files.scratch(params, files::Scratch::Make),
            "fs/scratch_remove" => self.files.scratch(params, files::Scratch::Remove),
            "fs/scratch_preview" => self.files.scratch(params, files::Scratch::Preview),
            "profile/canonical" => self.files.profile_canonical(caller, params),
            "profile/tree" => self.files.profile_tree(params),
            "profile/project" => self.files.profile_project(caller, params),
            "workspace/openers" => self.files.workspace_openers(),
            "workspace/open" => self.files.workspace_open(caller, params),
            "trust/list" => self.files.trust_list(),
            "trust/set" => self.files.trust_set(caller, params),
            _ => self.files.trust_revoke(caller, params),
        }
    }

    pub(super) fn r_config_get(
        &self,
        caller: &Caller,
        _: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        if let Some(section) = owner_section(caller, params) {
            return self.files.owner_get(section);
        }
        if names_host(params) {
            return allow(caller, Grant::Hosting).and_then(|()| self.files.host_get());
        }
        allow(caller, Grant::ConfigRead).and_then(|()| self.files.config_get(params))
    }

    pub(super) fn r_config_set(
        &self,
        caller: &Caller,
        _: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        self.config_write(caller, params, true)
    }

    pub(super) fn r_config_check(
        &self,
        caller: &Caller,
        _: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        self.config_write(caller, params, false)
    }

    fn config_write(
        &self,
        caller: &Caller,
        params: &Value,
        apply: bool,
    ) -> Result<Value, (i32, String)> {
        if let Some(section) = owner_section(caller, params) {
            return self.files.owner_set(params, section, apply);
        }
        if names_host(params) {
            return allow(caller, Grant::Hosting).and_then(|()| self.files.host_set(params, apply));
        }
        allow(caller, Grant::ConfigWrite)?;
        self.guard_sources(caller, params)?;
        if apply {
            self.files
                .config_set(params, caller.may(Grant::ConfigWriteSessions))
        } else {
            self.files
                .config_check(params, caller.may(Grant::ConfigWriteSessions))
        }
    }

    pub(super) fn r_config_view(
        &self,
        _: &Caller,
        method: &str,
        _: &Value,
    ) -> Result<Value, (i32, String)> {
        let config = self.loaded_config()?;
        Ok(match method {
            "looks/list" => looks::list(&config),
            "profile_items/list" => profile_items::list(&config),
            "panel_tabs/list" => panel_tabs::list(&crate::panel_tabs::TabCatalog::of(&config)),
            _ => providers::list(&config),
        })
    }

    pub(super) fn r_settings_sections(
        &self,
        caller: &Caller,
        _: &str,
        _: &Value,
    ) -> Result<Value, (i32, String)> {
        // The list of pages is the application's own preferences too, so any key reads it; the pages a provider adds, whose forms
        // hold what the host keeps, are listed only to a key that reads the configuration.
        if caller.may(Grant::ConfigRead) {
            self.loaded_config().map(|config| {
                settings_sections::list(
                    &crate::settings_sections::SectionCatalog::of(&config),
                    true,
                )
            })
        } else {
            Ok(settings_sections::list(
                &crate::settings_sections::SectionCatalog::from_registries(Vec::new()),
                false,
            ))
        }
    }

    pub(super) fn r_section_get(
        &self,
        _: &Caller,
        _: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        let config = self.loaded_config()?;
        settings_sections::get(
            &crate::settings_sections::SectionCatalog::of(&config),
            &self.files,
            &settings_sections::Keeper::new(&config, self.section_home()),
            params,
        )
    }

    pub(super) fn r_section_set(
        &self,
        _: &Caller,
        _: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        let config = self.loaded_config()?;
        settings_sections::set(
            &crate::settings_sections::SectionCatalog::of(&config),
            &self.files,
            &settings_sections::Keeper::new(&config, self.section_home()),
            params,
        )
    }

    pub(super) fn r_panel_tabs_tail(
        &self,
        _: &Caller,
        _: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        let config = self.loaded_config()?;
        panel_tabs::tail(&crate::panel_tabs::TabCatalog::of(&config), params)
    }

    pub(super) fn r_hosting(
        &self,
        _: &Caller,
        method: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        let store = self.keys_store()?;
        match method {
            "host/keys" => hosting::keys(store),
            "host/accept" => hosting::accept(store, params),
            _ => hosting::revoke(store, params),
        }
    }

    pub(super) fn r_which(
        &self,
        _: &Caller,
        _: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        registry::which(params, &crate::sources::runtime_dirs())
    }

    pub(super) fn r_restart(&self, _: &Caller, _: &str, _: &Value) -> Result<Value, (i32, String)> {
        self.restart_now()
    }

    pub(super) fn r_extension_catalogs(
        &self,
        caller: &Caller,
        _: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        self.extension_catalogs(caller, params)
    }

    pub(super) fn r_store_catalogs(
        &self,
        caller: &Caller,
        _: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        self.store_catalogs(caller, params)
    }

    pub(super) fn r_store_catalog(
        &self,
        caller: &Caller,
        _: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        self.store_catalog(caller, params)
    }

    pub(super) fn r_extension_act(
        &self,
        _: &Caller,
        method: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        match method {
            "extension/plan" => self.extension_plan(params),
            "extension/apply" => self.extension_apply(params),
            _ => self.extension_remove(params),
        }
    }

    pub(super) fn r_upload(
        &self,
        caller: &Caller,
        method: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        match method {
            "upload/put" => self.files.upload_put(caller, params),
            _ => self.files.upload_remove(caller, params),
        }
    }

    pub(super) fn r_overlays(
        &self,
        caller: &Caller,
        method: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        match method {
            "overlay/installed" => self.overlays.installed(params),
            "overlay/needs" => self.overlays.needs(params),
            "overlay/prepare" => self.overlays.prepare(params),
            "overlay/read" => self.overlays.read(caller, params),
            "catalog/read" => self.overlays.catalog_read(params),
            "overlay/check" => self
                .overlays
                .check(caller, params, &|repo| self.may_read_source(caller, repo)),
            "overlay/apply" => self.overlays.apply(caller, params),
            "overlay/remove" => self.overlays.remove(params),
            "overlay/discard" => self.overlays.discard(params),
            "catalog/fetch" => self
                .overlays
                .catalog_fetch(params, &|repo| self.may_read_source(caller, repo)),
            "catalog/remove" => self.overlays.catalog_remove(params),
            "skill/list" => self.overlays.skill_list(),
            "skill/read" => self.overlays.skill_read(params),
            "skill/fetch" => self
                .overlays
                .skill_fetch(params, &|repo| self.may_read_source(caller, repo)),
            "skill/scan" => self
                .overlays
                .skill_scan(params, &|repo| self.may_read_source(caller, repo)),
            "skill/save" => self.overlays.skill_save(params),
            "skill/remove" => self.overlays.skill_remove(params),
            "registry/fetch" => self.registry_fetch(caller, params),
            "skill/search" => self.skill_search(params),
            "store/read_ahead" => self
                .overlays
                .read_ahead(caller, params, &|repo| self.may_read_source(caller, repo)),
            "store/contents" => self.registry.contents(caller, params),
            _ => self.registry.put_contents(caller, params),
        }
    }

    pub(super) fn r_network(
        &self,
        _: &Caller,
        method: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        match method {
            "network/fetch" => self.fetch.run(params),
            "network/tunnel" => self.tunnel.run(params),
            _ => Ok(self.client_certificates.list()),
        }
    }

    pub(super) fn r_worktree(
        &self,
        caller: &Caller,
        method: &str,
        request: &Value,
        id: &Value,
    ) -> Vec<u8> {
        self.worktree(caller, method, request, id)
    }

    pub(super) fn r_stores_read(
        &self,
        caller: &Caller,
        method: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        self.stores()?.read(
            method,
            params,
            caller.may(Grant::StorePrograms).then(|| caller.owner()),
        )
    }

    pub(super) fn r_stores_report(
        &self,
        caller: &Caller,
        method: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        // Running a program of the user's, and choosing which copy, is the person's; a key reads what is found.
        let runs = params
            .get("probe")
            .is_some_and(|probe| probe != &Value::Bool(false))
            || params.get("pick").is_some_and(|pick| !pick.is_null());
        allow(caller, Grant::StoreRead)?;
        if runs {
            allow(caller, Grant::StorePrograms)?;
        }
        self.r_stores_read(caller, method, params)
    }

    pub(super) fn r_stores_programs(
        &self,
        _: &Caller,
        method: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        let stores = self.stores()?;
        match method {
            "store/models" => stores.models_read(params, &self.files),
            "store/model" => stores.model_act(params),
            "store/model_setting" => stores.model_setting(params, &self.files),
            _ => stores.change(method, params),
        }
    }

    pub(super) fn r_store_index(
        &self,
        caller: &Caller,
        _: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        self.store_index(caller, params)
    }

    pub(super) fn r_history(
        &self,
        _: &Caller,
        method: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        match method {
            "history/export" | "history/move" => self.history_call(method, params),
            "history/portable" => self.history_portable(),
            "history/account" => self.account_move(params),
            _ => self.import_call(method, params),
        }
    }

    pub(super) fn r_accounts(
        &self,
        _: &Caller,
        method: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        self.accounts()?.call(method, params)
    }

    pub(super) fn r_mail(
        &self,
        _: &Caller,
        method: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        self.mail_setup()?.call(method, params)
    }

    pub(super) fn r_usage(
        &self,
        _: &Caller,
        method: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        match method {
            "usage/read" => usage_read::call(
                self.accounts()?,
                self.usage.as_deref(),
                &self.usage_cache,
                params,
            ),
            _ => self.usage_call(method, params),
        }
    }

    pub(super) fn r_options_choices(
        &self,
        caller: &Caller,
        _: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        choices::call(self.accounts()?, caller, params)
    }

    pub(super) fn r_discover(
        &self,
        _: &Caller,
        _: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        discover::nearby(params)
    }

    pub(super) fn r_speech_offers(
        &self,
        caller: &Caller,
        method: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        speech_offers::call(
            caller,
            &self.files,
            &speech_offers::Lend::new(self.accounts.as_ref(), self.stores().ok()),
            method,
            params,
        )
    }

    pub(super) fn r_say(
        &self,
        caller: &Caller,
        method: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        self.say.call(caller, &self.files, method, params)
    }

    pub(super) fn r_speech(
        &self,
        caller: &Caller,
        method: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        self.speech.call(
            caller,
            &self.files,
            &speech_offers::Lend::new(self.accounts.as_ref(), self.stores().ok()),
            method,
            params,
        )
    }

    pub(super) fn r_shell_list(
        &self,
        _: &Caller,
        _: &str,
        _: &Value,
    ) -> Result<Value, (i32, String)> {
        // The shells a terminal could start, and which one each agent is told to use: for a key that may open a terminal
        // or read what programs are installed here (their paths are in `store/report` too).
        Ok(self.shells.report(&crate::shells::Lookup::here()))
    }

    pub(super) fn r_secrets(
        &self,
        _: &Caller,
        method: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        secrets::call(&self.vault()?, &self.files, method, params)
    }

    pub(super) fn r_needs_output_link(
        &self,
        _: &Caller,
        _: &str,
        _: &Value,
    ) -> Result<Value, (i32, String)> {
        Err((
            REFUSED,
            "this needs a link that can carry its output".to_owned(),
        ))
    }

    pub(super) fn r_needs_progress_link(
        &self,
        _: &Caller,
        _: &str,
        _: &Value,
    ) -> Result<Value, (i32, String)> {
        Err((
            REFUSED,
            "this needs a link that can carry its progress".to_owned(),
        ))
    }

    /// canoa.cfg's `[store]` table as the person wrote it, or null.
    pub(crate) fn store_section(&self) -> Value {
        self.files
            .config_get(&json!({"section": "store"}))
            .ok()
            .and_then(|answer| answer.get("value").cloned())
            .unwrap_or(Value::Null)
    }

    /// A key that does not hold `hosting` may not write a `[store]` that names a catalog source other than `read` it did not find
    /// there: `command` runs a program as the person and an extension's source is code the host loaded, so choosing one is the
    /// same trust as naming a library.
    fn guard_sources(&self, caller: &Caller, params: &Value) -> Result<(), (i32, String)> {
        if caller.may(Grant::Hosting)
            || params.get("section").and_then(Value::as_str) != Some("store")
        {
            return Ok(());
        }
        let wanted = params.get("value").unwrap_or(&Value::Null);
        if crate::store::catalogs::names_new_source(&self.store_section(), wanted) {
            return Err((
                DENIED,
                "a catalog that runs a program or reads through an extension's source takes the hosting grant".to_owned(),
            ));
        }
        Ok(())
    }

    /// The catalog sources this host offers: the built-in ones, and those of the extensions it loaded.
    pub(crate) fn catalog_sources(&self) -> crate::store::sources::Sources {
        self.stores().map_or_else(
            |_| crate::store::sources::Sources::built_in(),
            Stores::sources,
        )
    }

    /// `registry/fetch`: the person running the host reads any registry; a key reads the official one and those canoa.cfg lists.
    fn registry_fetch(&self, caller: &Caller, params: &Value) -> Result<Value, (i32, String)> {
        let bases = match caller {
            Caller::Owner => None,
            Caller::Peer { .. } => Some(registry::listed_bases(&self.store_section())),
        };
        self.registry.fetch(params, bases.as_deref())
    }

    /// A worktree request, answered by the rule an entry's clients have, over the roots this caller may use: the whole machine
    /// for the owner, and for a key the working roots of the entries it may use. Every other check of a worktree stands.
    fn worktree(&self, caller: &Caller, method: &str, request: &Value, id: &Value) -> Vec<u8> {
        if let Err((code, message)) = allow(caller, Grant::Worktree) {
            return error_line(id, code, &message);
        }
        let roots = match caller {
            Caller::Owner => whole_machine(),
            Caller::Peer { id: key, .. } => self
                .entries
                .iter()
                .filter(|entry| {
                    entry
                        .peers
                        .as_ref()
                        .is_none_or(|peers| peers.iter().any(|peer| peer == key))
                })
                .flat_map(|entry| entry.roots.iter().cloned())
                .collect(),
        };
        let mut asked = request.clone();
        asked["method"] = Value::String(format!("_canoa/{method}"));
        let bytes = serde_json::to_vec(&asked).unwrap_or_default();
        Worktrees::new(roots, true)
            .with_limits(self.worktrees)
            .answer(&bytes)
            .unwrap_or_else(|| error_line(id, BAD_PARAMS, "that request cannot be answered"))
    }
}

/// Every place on this computer a path can start: `/`, or each drive that is there.
pub(super) fn whole_machine() -> Vec<PathBuf> {
    if cfg!(windows) {
        ('A'..='Z')
            .map(|letter| PathBuf::from(format!("{letter}:\\")))
            .filter(|drive| drive.exists())
            .collect()
    } else {
        vec![PathBuf::from("/")]
    }
}

/// Whether a request names `[host]`, which only a key that holds `hosting` reads or writes.
fn names_host(params: &Value) -> bool {
    params.get("section").and_then(Value::as_str) == Some(files::HOST)
}

/// The table a request names when it is one only the app this host runs in edits and the caller is that app.
fn owner_section<'a>(caller: &Caller, params: &'a Value) -> Option<&'a str> {
    let section = params.get("section").and_then(Value::as_str)?;
    (matches!(caller, Caller::Owner) && files::OWNER_SECTIONS.contains(&section)).then_some(section)
}
