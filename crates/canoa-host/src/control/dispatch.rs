//! The control channel's grant-checked route table and JSON-RPC dispatch.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "JSON answers are std's"
)]

use serde_json::{Value, json};

use super::{Caller, Control, DENIED, Grant, NO_METHOD, NOT_JSON, REFUSED, allow, error_line};

#[derive(Clone, Copy)]
pub(super) enum Access {
    Open,
    Require(Grant),
    All(&'static [Grant]),
    Any(&'static [Grant]),
    Owner(&'static str),
    Dynamic(
        #[cfg_attr(
            not(test),
            expect(dead_code, reason = "checked against GRANT_SPECS in tests")
        )]
        &'static [Grant],
    ),
}

impl Access {
    pub(super) fn check(self, caller: &Caller) -> Result<(), (i32, String)> {
        match self {
            Self::Open | Self::Dynamic(_) => Ok(()),
            Self::Require(grant) => allow(caller, grant),
            Self::All(grants) => {
                for &grant in grants {
                    allow(caller, grant)?;
                }
                Ok(())
            }
            Self::Any(grants) => {
                if grants.iter().any(|&grant| caller.may(grant)) {
                    Ok(())
                } else if let Some(&last) = grants.last() {
                    allow(caller, last)
                } else {
                    Ok(())
                }
            }
            Self::Owner(message) => match caller {
                Caller::Owner => Ok(()),
                Caller::Peer { .. } => Err((DENIED, message.to_owned())),
            },
        }
    }

    #[cfg(test)]
    pub(super) fn mentions(self, grant: Grant) -> bool {
        match self {
            Self::Open | Self::Owner(_) => false,
            Self::Require(held) => held == grant,
            Self::All(list) | Self::Any(list) | Self::Dynamic(list) => list.contains(&grant),
        }
    }
}

type RpcHandler = fn(&Control, &Caller, &str, &Value) -> Result<Value, (i32, String)>;
type RawHandler = fn(&Control, &Caller, &str, &Value, &Value) -> Vec<u8>;

#[derive(Clone, Copy)]
enum Handler {
    Rpc(RpcHandler),
    Raw(RawHandler),
}

#[derive(Clone, Copy)]
pub(super) struct Route {
    pub(super) method: &'static str,
    pub(super) access: Access,
    handler: Handler,
}

const fn rpc(method: &'static str, access: Access, handler: RpcHandler) -> Route {
    Route {
        method,
        access,
        handler: Handler::Rpc(handler),
    }
}

const fn raw(method: &'static str, access: Access, handler: RawHandler) -> Route {
    Route {
        method,
        access,
        handler: Handler::Raw(handler),
    }
}

const OWNER_NETWORK_MSG: &str =
    "only the person running this host sends requests through its network";
const OWNER_PUBLISH_MSG: &str = "only the person running this host commits, pushes or opens a pull request with their own git and gh";
const OWNER_SCHEDULES_MSG: &str =
    "only the person running this host sets what it runs on a schedule, which runs as them";
const OWNER_HISTORY_MSG: &str =
    "only the person running this host reads or moves a recorded session";
const OWNER_LIMITS_MSG: &str = "only the person running this host lets a turn through a limit";

/// Dispatch table mapping control RPC method names to required grants and handlers.
pub(super) static ROUTES: &[Route] = &[
    rpc(
        "ledger/list",
        Access::Require(Grant::LedgerRead),
        Control::r_ledger_list,
    ),
    rpc(
        "sessions/running",
        Access::Require(Grant::LedgerRead),
        Control::r_running_sessions,
    ),
    rpc(
        "sessions/activity",
        Access::Require(Grant::LedgerRead),
        Control::r_session_activity,
    ),
    rpc(
        "sessions/watch",
        Access::Require(Grant::SessionsWatch),
        Control::r_session_watch,
    ),
    rpc(
        "ledger/put",
        Access::Require(Grant::LedgerWrite),
        Control::r_ledger_put,
    ),
    rpc(
        "ledger/remove",
        Access::Require(Grant::LedgerWrite),
        Control::r_ledger_remove,
    ),
    rpc(
        "ledger/clear",
        Access::Require(Grant::LedgerWrite),
        Control::r_ledger_clear,
    ),
    rpc("peer/forget", Access::Open, Control::r_forget),
    rpc("host/grants", Access::Open, Control::r_grants),
    rpc("fs/read", Access::Require(Grant::Files), Control::r_fs_read),
    rpc(
        "fs/write",
        Access::All(&[Grant::Files, Grant::FilesWrite]),
        Control::r_fs_write,
    ),
    rpc("state/get", Access::Require(Grant::State), Control::r_state),
    rpc("state/put", Access::Require(Grant::State), Control::r_state),
    rpc(
        "state/append",
        Access::Require(Grant::State),
        Control::r_state,
    ),
    rpc(
        "state/list",
        Access::Require(Grant::State),
        Control::r_state,
    ),
    rpc(
        "state/remove",
        Access::Require(Grant::State),
        Control::r_state,
    ),
    rpc("fs/list", Access::Require(Grant::Folders), Control::r_files),
    rpc("fs/info", Access::Require(Grant::Folders), Control::r_files),
    rpc(
        "fs/realpath",
        Access::Require(Grant::Folders),
        Control::r_files,
    ),
    rpc(
        "fs/guards",
        Access::Require(Grant::Folders),
        Control::r_files,
    ),
    rpc(
        "fs/repo_root",
        Access::Require(Grant::Folders),
        Control::r_files,
    ),
    rpc(
        "fs/instructions",
        Access::Require(Grant::Folders),
        Control::r_files,
    ),
    rpc(
        "fs/mentions",
        Access::Require(Grant::Folders),
        Control::r_files,
    ),
    rpc(
        "fs/resolve",
        Access::Require(Grant::Folders),
        Control::r_files,
    ),
    rpc(
        "fs/scratch",
        Access::Require(Grant::Folders),
        Control::r_files,
    ),
    rpc(
        "fs/scratch_remove",
        Access::Require(Grant::Folders),
        Control::r_files,
    ),
    rpc(
        "fs/scratch_preview",
        Access::Require(Grant::Folders),
        Control::r_files,
    ),
    rpc(
        "profile/project",
        Access::Require(Grant::Folders),
        Control::r_files,
    ),
    rpc(
        "workspace/openers",
        Access::Require(Grant::WorkspaceOpen),
        Control::r_files,
    ),
    rpc(
        "workspace/open",
        Access::Require(Grant::WorkspaceOpen),
        Control::r_files,
    ),
    rpc(
        "profile/canonical",
        Access::Require(Grant::OverlayRead),
        Control::r_files,
    ),
    rpc(
        "profile/tree",
        Access::Require(Grant::OverlayRead),
        Control::r_files,
    ),
    rpc(
        "trust/list",
        Access::Require(Grant::ConfigRead),
        Control::r_files,
    ),
    rpc(
        "trust/set",
        Access::Require(Grant::ConfigWriteSessions),
        Control::r_files,
    ),
    rpc(
        "rules/read",
        Access::Require(Grant::ConfigRead),
        Control::r_files,
    ),
    rpc(
        "rules/write",
        Access::Require(Grant::ConfigWriteSessions),
        Control::r_files,
    ),
    rpc(
        "trust/revoke",
        Access::Require(Grant::ConfigWrite),
        Control::r_files,
    ),
    rpc(
        "config/get",
        Access::Dynamic(&[Grant::ConfigRead, Grant::Hosting]),
        Control::r_config_get,
    ),
    rpc(
        "config/set",
        Access::Dynamic(&[
            Grant::ConfigWrite,
            Grant::Hosting,
            Grant::ConfigWriteSessions,
        ]),
        Control::r_config_set,
    ),
    rpc(
        "config/check",
        Access::Dynamic(&[
            Grant::ConfigWrite,
            Grant::Hosting,
            Grant::ConfigWriteSessions,
        ]),
        Control::r_config_check,
    ),
    rpc(
        "config/layers",
        Access::Require(Grant::Hosting),
        Control::r_config_layers,
    ),
    rpc(
        "config/change",
        Access::Require(Grant::Hosting),
        Control::r_config_change,
    ),
    rpc(
        "config/advanced",
        Access::Require(Grant::Hosting),
        Control::r_config_advanced,
    ),
    rpc(
        "config/advanced_set",
        Access::Require(Grant::Hosting),
        Control::r_config_advanced_set,
    ),
    rpc(
        "looks/list",
        Access::Require(Grant::ConfigRead),
        Control::r_config_view,
    ),
    rpc(
        "profile_items/list",
        Access::Require(Grant::ConfigRead),
        Control::r_config_view,
    ),
    rpc(
        "settings/sections",
        Access::Dynamic(&[Grant::ConfigRead]),
        Control::r_settings_sections,
    ),
    rpc(
        "settings/section_get",
        Access::Require(Grant::ConfigRead),
        Control::r_section_get,
    ),
    rpc(
        "settings/section_set",
        Access::Require(Grant::ConfigWrite),
        Control::r_section_set,
    ),
    rpc(
        "panel_tabs/list",
        Access::Require(Grant::ConfigRead),
        Control::r_config_view,
    ),
    rpc(
        "panel_tabs/tail",
        Access::Require(Grant::Hosting),
        Control::r_panel_tabs_tail,
    ),
    rpc(
        "file_viewers/list",
        Access::Require(Grant::ConfigRead),
        Control::r_config_view,
    ),
    rpc(
        "file_viewers/render",
        Access::Require(Grant::Files),
        Control::r_file_viewers_render,
    ),
    rpc(
        "providers/list",
        Access::Require(Grant::ConfigRead),
        Control::r_config_view,
    ),
    rpc(
        "providers/registered",
        Access::Require(Grant::ConfigRead),
        Control::r_config_view,
    ),
    rpc(
        "providers/switch",
        Access::Require(Grant::Hosting),
        Control::r_providers_switch,
    ),
    rpc(
        "host/keys",
        Access::Require(Grant::Hosting),
        Control::r_hosting,
    ),
    rpc(
        "host/accept",
        Access::Require(Grant::Hosting),
        Control::r_hosting,
    ),
    rpc(
        "host/revoke",
        Access::Require(Grant::Hosting),
        Control::r_hosting,
    ),
    rpc(
        "host/which",
        Access::Require(Grant::StoreRead),
        Control::r_which,
    ),
    rpc(
        "host/restart",
        Access::Require(Grant::HostRestart),
        Control::r_restart,
    ),
    rpc(
        "extension/catalogs",
        Access::Dynamic(&[Grant::OverlayRead, Grant::OverlayWrite]),
        Control::r_extension_catalogs,
    ),
    rpc(
        "content/stat",
        Access::Require(Grant::Hosting),
        Control::r_content_stat,
    ),
    rpc(
        "content/list",
        Access::Require(Grant::Hosting),
        Control::r_content_list,
    ),
    rpc(
        "content/read",
        Access::Require(Grant::Hosting),
        Control::r_content_read,
    ),
    rpc(
        "content/resolve",
        Access::Require(Grant::Hosting),
        Control::r_content_resolve,
    ),
    rpc(
        "content/cache",
        Access::Require(Grant::Hosting),
        Control::r_content_cache,
    ),
    rpc(
        "content/schemes",
        Access::Require(Grant::ConfigRead),
        Control::r_content_schemes,
    ),
    rpc(
        "store/catalogs",
        Access::Dynamic(&[Grant::OverlayRead]),
        Control::r_store_catalogs,
    ),
    rpc(
        "store/catalog",
        Access::Require(Grant::OverlayWrite),
        Control::r_store_catalog,
    ),
    rpc(
        "store/catalog_set",
        Access::Require(Grant::ConfigWrite),
        Control::r_store_catalog_set,
    ),
    rpc(
        "extension/plan",
        Access::Require(Grant::Hosting),
        Control::r_extension_act,
    ),
    rpc(
        "extension/apply",
        Access::Require(Grant::Hosting),
        Control::r_extension_act,
    ),
    rpc(
        "extension/remove",
        Access::Require(Grant::Hosting),
        Control::r_extension_act,
    ),
    rpc(
        "upload/put",
        Access::Require(Grant::Upload),
        Control::r_upload,
    ),
    rpc(
        "upload/remove",
        Access::Require(Grant::Upload),
        Control::r_upload,
    ),
    rpc(
        "overlay/installed",
        Access::Require(Grant::OverlayRead),
        Control::r_overlays,
    ),
    rpc(
        "overlay/updates",
        Access::Require(Grant::OverlayRead),
        Control::r_overlays,
    ),
    rpc(
        "overlay/needs",
        Access::All(&[Grant::OverlayRead, Grant::StorePrograms]),
        Control::r_overlays,
    ),
    rpc(
        "overlay/prepare",
        Access::All(&[Grant::OverlayWrite, Grant::StorePrograms]),
        Control::r_overlays,
    ),
    rpc(
        "overlay/read",
        Access::Require(Grant::OverlayRead),
        Control::r_overlays,
    ),
    rpc(
        "catalog/read",
        Access::Require(Grant::OverlayRead),
        Control::r_overlays,
    ),
    rpc(
        "skill/list",
        Access::Require(Grant::OverlayRead),
        Control::r_overlays,
    ),
    rpc(
        "skill/read",
        Access::Require(Grant::OverlayRead),
        Control::r_overlays,
    ),
    rpc(
        "store/contents",
        Access::Require(Grant::OverlayRead),
        Control::r_overlays,
    ),
    rpc(
        "store/contents_put",
        Access::Require(Grant::OverlayRead),
        Control::r_overlays,
    ),
    rpc(
        "overlay/check",
        Access::Require(Grant::OverlayWrite),
        Control::r_overlays,
    ),
    rpc(
        "overlay/apply",
        Access::Require(Grant::OverlayWrite),
        Control::r_overlays,
    ),
    rpc(
        "overlay/remove",
        Access::Require(Grant::OverlayWrite),
        Control::r_overlays,
    ),
    rpc(
        "overlay/discard",
        Access::Require(Grant::OverlayWrite),
        Control::r_overlays,
    ),
    rpc(
        "catalog/fetch",
        Access::Require(Grant::OverlayWrite),
        Control::r_overlays,
    ),
    rpc(
        "catalog/remove",
        Access::Require(Grant::OverlayWrite),
        Control::r_overlays,
    ),
    rpc(
        "skill/fetch",
        Access::Require(Grant::OverlayWrite),
        Control::r_overlays,
    ),
    rpc(
        "skill/scan",
        Access::Require(Grant::OverlayWrite),
        Control::r_overlays,
    ),
    rpc(
        "skill/save",
        Access::Require(Grant::OverlayWrite),
        Control::r_overlays,
    ),
    rpc(
        "skill/remove",
        Access::Require(Grant::OverlayWrite),
        Control::r_overlays,
    ),
    rpc(
        "registry/fetch",
        Access::Require(Grant::OverlayWrite),
        Control::r_overlays,
    ),
    rpc(
        "skill/search",
        Access::Require(Grant::OverlayWrite),
        Control::r_overlays,
    ),
    rpc(
        "store/read_ahead",
        Access::Require(Grant::OverlayWrite),
        Control::r_overlays,
    ),
    rpc(
        "network/fetch",
        Access::Owner(OWNER_NETWORK_MSG),
        Control::r_network,
    ),
    rpc(
        "network/tunnel",
        Access::Owner(OWNER_NETWORK_MSG),
        Control::r_network,
    ),
    rpc(
        "network/client_certificates",
        Access::Require(Grant::ConfigWrite),
        Control::r_network,
    ),
    raw(
        "worktree/new",
        Access::Require(Grant::Worktree),
        Control::r_worktree,
    ),
    raw(
        "worktree/status",
        Access::Require(Grant::Worktree),
        Control::r_worktree,
    ),
    raw(
        "worktree/remove",
        Access::Require(Grant::Worktree),
        Control::r_worktree,
    ),
    raw(
        "worktree/diff",
        Access::Require(Grant::Worktree),
        Control::r_worktree,
    ),
    raw(
        "worktree/commit",
        Access::Owner(OWNER_PUBLISH_MSG),
        Control::r_worktree,
    ),
    raw(
        "worktree/push",
        Access::Owner(OWNER_PUBLISH_MSG),
        Control::r_worktree,
    ),
    raw(
        "worktree/pull_request",
        Access::Owner(OWNER_PUBLISH_MSG),
        Control::r_worktree,
    ),
    rpc(
        "store/list",
        Access::Require(Grant::StoreRead),
        Control::r_stores_read,
    ),
    rpc(
        "store/overview",
        Access::Require(Grant::StoreRead),
        Control::r_stores_read,
    ),
    rpc(
        "store/extensions",
        Access::Require(Grant::StoreRead),
        Control::r_stores_read,
    ),
    rpc(
        "store/report",
        Access::Dynamic(&[Grant::StoreRead, Grant::StorePrograms]),
        Control::r_stores_report,
    ),
    rpc(
        "store/latest",
        Access::Require(Grant::StoreRead),
        Control::r_stores_read,
    ),
    rpc(
        "store/plan",
        Access::Require(Grant::StoreRead),
        Control::r_stores_read,
    ),
    rpc(
        "store/models",
        Access::Require(Grant::StorePrograms),
        Control::r_stores_programs,
    ),
    rpc(
        "store/model",
        Access::Require(Grant::StorePrograms),
        Control::r_stores_programs,
    ),
    rpc(
        "store/model_setting",
        Access::Require(Grant::StorePrograms),
        Control::r_stores_programs,
    ),
    rpc(
        "store/rollback",
        Access::Require(Grant::StorePrograms),
        Control::r_stores_programs,
    ),
    rpc(
        "store/remove",
        Access::Require(Grant::StorePrograms),
        Control::r_stores_programs,
    ),
    rpc(
        "store/index",
        Access::Dynamic(&[
            Grant::OverlayRead,
            Grant::StorePrograms,
            Grant::OverlayWrite,
        ]),
        Control::r_store_index,
    ),
    rpc(
        "history/export",
        Access::Owner(OWNER_HISTORY_MSG),
        Control::r_history,
    ),
    rpc(
        "history/move",
        Access::Owner(OWNER_HISTORY_MSG),
        Control::r_history,
    ),
    rpc(
        "history/portable",
        Access::Owner(OWNER_HISTORY_MSG),
        Control::r_history,
    ),
    rpc(
        "history/status",
        Access::Owner(OWNER_HISTORY_MSG),
        Control::r_history,
    ),
    rpc(
        "history/account",
        Access::Owner(OWNER_HISTORY_MSG),
        Control::r_history,
    ),
    rpc(
        "history/importable",
        Access::Owner(OWNER_HISTORY_MSG),
        Control::r_history,
    ),
    rpc(
        "history/import",
        Access::Owner(OWNER_HISTORY_MSG),
        Control::r_history,
    ),
    rpc(
        "history/imported",
        Access::Owner(OWNER_HISTORY_MSG),
        Control::r_history,
    ),
    rpc(
        "schedules/list",
        Access::Owner(OWNER_SCHEDULES_MSG),
        Control::r_schedules,
    ),
    rpc(
        "schedules/set",
        Access::Owner(OWNER_SCHEDULES_MSG),
        Control::r_schedules,
    ),
    rpc(
        "schedules/remove",
        Access::Owner(OWNER_SCHEDULES_MSG),
        Control::r_schedules,
    ),
    rpc(
        "schedules/run",
        Access::Owner(OWNER_SCHEDULES_MSG),
        Control::r_schedules,
    ),
    rpc(
        "accounts/providers",
        Access::Require(Grant::Accounts),
        Control::r_accounts,
    ),
    rpc(
        "accounts/catalog",
        Access::Require(Grant::Accounts),
        Control::r_accounts,
    ),
    rpc(
        "accounts/list",
        Access::Require(Grant::Accounts),
        Control::r_accounts,
    ),
    rpc(
        "accounts/remove",
        Access::Require(Grant::Accounts),
        Control::r_accounts,
    ),
    rpc(
        "accounts/entries",
        Access::Require(Grant::Accounts),
        Control::r_accounts,
    ),
    rpc(
        "accounts/offer",
        Access::Require(Grant::Accounts),
        Control::r_accounts,
    ),
    rpc(
        "routes/list",
        Access::Require(Grant::ConfigRead),
        Control::r_accounts,
    ),
    rpc(
        "routes/offers",
        Access::Require(Grant::ConfigRead),
        Control::r_accounts,
    ),
    rpc(
        "routes/test",
        Access::Require(Grant::ConfigRead),
        Control::r_accounts,
    ),
    rpc(
        "routes/set",
        Access::Require(Grant::ConfigWrite),
        Control::r_accounts,
    ),
    rpc(
        "routes/remove",
        Access::Require(Grant::ConfigWrite),
        Control::r_accounts,
    ),
    rpc(
        "routes/default",
        Access::Require(Grant::ConfigWrite),
        Control::r_accounts,
    ),
    rpc(
        "accounts/set_route",
        Access::Require(Grant::ConfigWrite),
        Control::r_accounts,
    ),
    rpc(
        "pools/list",
        Access::Require(Grant::Accounts),
        Control::r_accounts,
    ),
    rpc(
        "pools/set",
        Access::Require(Grant::Accounts),
        Control::r_accounts,
    ),
    rpc(
        "pools/remove",
        Access::Require(Grant::Accounts),
        Control::r_accounts,
    ),
    rpc(
        "pools/turn",
        Access::Require(Grant::Accounts),
        Control::r_accounts,
    ),
    rpc(
        "usage/summary",
        Access::Require(Grant::Usage),
        Control::r_usage,
    ),
    rpc(
        "usage/limits",
        Access::Require(Grant::Usage),
        Control::r_usage,
    ),
    rpc(
        "usage/read",
        Access::Require(Grant::Usage),
        Control::r_usage,
    ),
    rpc(
        "limits/state",
        Access::Require(Grant::Usage),
        Control::r_limits,
    ),
    rpc(
        "limits/allow",
        Access::Owner(OWNER_LIMITS_MSG),
        Control::r_limits,
    ),
    rpc("options/choices", Access::Open, Control::r_options_choices),
    rpc("kb/offers", Access::Require(Grant::Accounts), Control::r_kb),
    rpc("kb/test", Access::Require(Grant::Accounts), Control::r_kb),
    rpc(
        "discover/nearby",
        Access::Require(Grant::Discover),
        Control::r_discover,
    ),
    rpc(
        "speech/offers",
        Access::Require(Grant::Voice),
        Control::r_speech_offers,
    ),
    rpc(
        "speech/enable",
        Access::Require(Grant::VoiceSetup),
        Control::r_speech_offers,
    ),
    rpc(
        "speech/voices",
        Access::Require(Grant::Voice),
        Control::r_say,
    ),
    rpc("speech/say", Access::Require(Grant::Voice), Control::r_say),
    rpc(
        "speech/audio",
        Access::Require(Grant::Voice),
        Control::r_say,
    ),
    rpc("speech/stop", Access::Require(Grant::Voice), Control::r_say),
    rpc(
        "speech/say_configure",
        Access::Require(Grant::VoiceSetup),
        Control::r_say,
    ),
    rpc(
        "speech/providers",
        Access::Require(Grant::Voice),
        Control::r_speech,
    ),
    rpc(
        "speech/configure",
        Access::Require(Grant::VoiceSetup),
        Control::r_speech,
    ),
    rpc(
        "speech/open",
        Access::Require(Grant::Voice),
        Control::r_speech,
    ),
    rpc(
        "speech/push",
        Access::Require(Grant::Voice),
        Control::r_speech,
    ),
    rpc(
        "speech/finish",
        Access::Require(Grant::Voice),
        Control::r_speech,
    ),
    rpc(
        "speech/cancel",
        Access::Require(Grant::Voice),
        Control::r_speech,
    ),
    rpc(
        "shell/list",
        Access::Any(&[Grant::StoreRead, Grant::Terminal]),
        Control::r_shell_list,
    ),
    rpc(
        "terminal/open",
        Access::Require(Grant::Terminal),
        Control::r_needs_output_link,
    ),
    rpc(
        "terminal/write",
        Access::Require(Grant::Terminal),
        Control::r_needs_output_link,
    ),
    rpc(
        "terminal/resize",
        Access::Require(Grant::Terminal),
        Control::r_needs_output_link,
    ),
    rpc(
        "terminal/scrollback",
        Access::Require(Grant::Terminal),
        Control::r_needs_output_link,
    ),
    rpc(
        "terminal/close",
        Access::Require(Grant::Terminal),
        Control::r_needs_output_link,
    ),
    rpc(
        "secrets/providers",
        Access::Require(Grant::Secrets),
        Control::r_secrets,
    ),
    rpc(
        "secrets/list",
        Access::Require(Grant::Secrets),
        Control::r_secrets,
    ),
    rpc(
        "secrets/reveal",
        Access::Require(Grant::Secrets),
        Control::r_secrets,
    ),
    rpc(
        "secrets/forget",
        Access::Require(Grant::Secrets),
        Control::r_secrets,
    ),
    rpc(
        "secrets/configure",
        Access::Require(Grant::Secrets),
        Control::r_secrets,
    ),
    rpc(
        "notify/subscribe",
        Access::Require(Grant::Notify),
        Control::r_needs_output_link,
    ),
    rpc(
        "notify/attended",
        Access::Require(Grant::Notify),
        Control::r_needs_output_link,
    ),
    rpc(
        "notify/notifiers",
        Access::Require(Grant::Notify),
        Control::r_notify,
    ),
    rpc(
        "notify/events",
        Access::Require(Grant::Notify),
        Control::r_notify,
    ),
    rpc(
        "notify/test",
        Access::Require(Grant::Notify),
        Control::r_notify,
    ),
    rpc(
        "notify/keep",
        Access::Require(Grant::Notify),
        Control::r_notify,
    ),
    rpc(
        "store/apply",
        Access::Require(Grant::StorePrograms),
        Control::r_needs_progress_link,
    ),
    rpc(
        "accounts/add",
        Access::Require(Grant::Accounts),
        Control::r_needs_progress_link,
    ),
    rpc(
        "accounts/login",
        Access::Require(Grant::Accounts),
        Control::r_needs_progress_link,
    ),
    rpc(
        "host/pair",
        Access::Require(Grant::Hosting),
        Control::r_needs_progress_link,
    ),
    rpc(
        "secrets/set",
        Access::Require(Grant::Secrets),
        Control::r_needs_progress_link,
    ),
];

impl Control {
    /// The answer to one request line.
    pub(crate) fn handle(&self, caller: &Caller, line: &[u8]) -> Option<Vec<u8>> {
        let request: Value = match serde_json::from_slice(line) {
            Ok(request) => request,
            Err(_) => return Some(error_line(&Value::Null, NOT_JSON, "that is not JSON")),
        };
        let id = request.get("id")?.clone();
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        let outcome = match request.get("method").and_then(Value::as_str) {
            Some(method) => match ROUTES.iter().find(|route| route.method == method) {
                Some(route) => match route.access.check(caller) {
                    Err(error) => Err(error),
                    Ok(()) => match route.handler {
                        Handler::Rpc(run) => run(self, caller, method, &params),
                        Handler::Raw(run) => return Some(run(self, caller, method, &request, &id)),
                    },
                },
                None if method.starts_with("terminal/") => {
                    allow(caller, Grant::Terminal).and_then(|()| {
                        Err((
                            REFUSED,
                            "this needs a link that can carry its output".to_owned(),
                        ))
                    })
                }
                None if self
                    .services
                    .as_ref()
                    .is_some_and(|services| services.has_namespace(method)) =>
                {
                    allow(caller, Grant::ServicesSetup)
                        .and_then(|()| self.services_call(method, &params))
                }
                None => Err((NO_METHOD, "this host has no such method".to_owned())),
            },
            None => Err((NO_METHOD, "this host has no such method".to_owned())),
        };
        if outcome.is_err()
            && request.get("method").and_then(Value::as_str) == Some("extension/apply")
        {
            self.update_failed();
        }
        Some(match outcome {
            Ok(result) => {
                serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": id, "result": result}))
                    .unwrap_or_default()
            }
            Err((code, message)) => error_line(&id, code, &message),
        })
    }
}
