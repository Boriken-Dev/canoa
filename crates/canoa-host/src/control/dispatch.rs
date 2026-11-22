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
const OWNER_HISTORY_MSG: &str =
    "only the person running this host reads or moves a recorded session";

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
        "providers/list",
        Access::Require(Grant::ConfigRead),
        Control::r_config_view,
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
    rpc(
        "store/list",
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
        "mail/accounts",
        Access::Require(Grant::MailSetup),
        Control::r_mail,
    ),
    rpc(
        "mail/add",
        Access::Require(Grant::MailSetup),
        Control::r_mail,
    ),
    rpc(
        "mail/test",
        Access::Require(Grant::MailSetup),
        Control::r_mail,
    ),
    rpc(
        "mail/remove",
        Access::Require(Grant::MailSetup),
        Control::r_mail,
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
    rpc("options/choices", Access::Open, Control::r_options_choices),
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
                None => Err((NO_METHOD, "this host has no such method".to_owned())),
            },
            None => Err((NO_METHOD, "this host has no such method".to_owned())),
        };
        Some(match outcome {
            Ok(result) => {
                serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": id, "result": result}))
                    .unwrap_or_default()
            }
            Err((code, message)) => error_line(&id, code, &message),
        })
    }
}
