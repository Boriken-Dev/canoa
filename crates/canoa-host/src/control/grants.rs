//! Control channel grants (`[host.control] grants`), their config names, and caller identity.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "sets and JSON answers are std's"
)]

use std::collections::BTreeSet;

use serde_json::{Value, json};

use super::DENIED;

/// The owner of the app's own records.
pub(super) const OWNER: &str = "local";

/// What a paired key may be allowed to do on the control channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Grant {
    /// `ledger/list`, of the records the key owns.
    LedgerRead,
    /// `ledger/put`, `ledger/remove` and `ledger/clear`, of the records the key owns.
    LedgerWrite,
    /// `worktree/new`, `worktree/status` and `worktree/remove`, under the working roots of the entries the key may use.
    Worktree,
    /// `store/list`, `store/report`, `store/latest` and `store/plan`: what the host finds and would do, and nothing it writes.
    /// Also `host/which`: whether a program is on the host's `PATH`, which starts nothing.
    /// Installing, updating, rolling back and removing is [`Grant::StorePrograms`], which is not implied by this.
    StoreRead,
    /// `fs/list`: the folders under the user's home and the working roots of the entries the key may use. Also `fs/info`,
    /// `fs/mentions` and `fs/resolve` in the same places (whether a folder is there and in a git repository, the names an `@`
    /// offers inside one, and one name resolved inside it), and `fs/scratch`, `fs/scratch_remove` and `fs/scratch_preview`: the
    /// folder the person's `[sessions] workspace` makes for a chat, made, taken away again if it is empty, or worked out for a
    /// template being edited. Also `fs/realpath` and `fs/repo_root`: where a path goes once its links are followed and where the
    /// repository of a folder starts, and `fs/guards`: the home folder and the folder of canoa.cfg, which the app asks to judge a
    /// remembered rule for a path. Also `profile/project`: a folder's project profile file, read to be shown and trusted.
    Folders,
    /// `config/get`: a named section of canoa.cfg, the app's own tables only. Also `trust/list`: the projects whose profiles the person trusts, and `looks/list`: the vendor families' colour tokens.
    ConfigRead,
    /// `config/set`: a named section of canoa.cfg, the app's own tables only; never `host`, `agents` or `profile`. Also `trust/revoke`.
    ConfigWrite,
    /// `config/set` of `overlays`, `loadouts`, `defaults` and `permissions`, which change what an agent may do or is given: as much as access to the agent. Also `trust/set`: a project's profile then applies to the sessions in its folder.
    ConfigWriteSessions,
    /// `upload/put` and `upload/remove`: files in a folder of the key's own, within the limits of `[host.control]`.
    Upload,
    /// `overlay/installed`, `overlay/read`, `catalog/read`, `skill/list` and `skill/read`: what is installed and what a check or a catalog fetch holds. Also `profile/canonical` and `profile/tree`: an overlay's folder with its links resolved, and its files read whole, for the app to show a profile's layers. The
    /// host fetches nothing for these. Also `store/contents` and `store/contents_put`: the key's own cache of what the entries of a
    /// catalog provide, which no other key reads.
    OverlayRead,
    /// `registry/fetch` (one page of an MCP registry's list, from the official registry or one `[store.mcps.catalogs]` lists), `store/catalog` (the entries of one Skills or MCP servers catalog), `overlay/check`, `overlay/apply`, `overlay/remove`, `overlay/discard`, `catalog/fetch`, `catalog/remove`, `skill/fetch`, `skill/scan`, `skill/save`, `skill/remove` and `skill/search` (the skills directory `[store] skills_search` names, once the person turned it on): fetch sources with
    /// git as this user and install what AOS digests, under the overlays folder beside canoa.cfg. Nothing is run.
    OverlayWrite,
    /// `host/restart`: ends this host so that its supervisor starts it again with the canoa.cfg then.
    HostRestart,
    /// `store/apply`, `store/rollback` and `store/remove`, and the probing `store/report`: install, update, roll back and remove an
    /// agent program on the host's programs shelf, which downloads and runs code as this user. Also `store/models`, `store/model`
    /// and `store/model_setting`: list, move and set up the models a Store type's library lists, which runs the type's program as
    /// this user.
    StorePrograms,
    /// `accounts/providers`, `accounts/catalog`, `accounts/list`, `accounts/add`, `accounts/login`, `accounts/remove`, `accounts/entries` and
    /// `accounts/offer`, and `pools/list`, `pools/set` `pools/remove` and `pools/turn` (sets of accounts of one standard, and which of them takes a turn): sign in to a provider on this host, which runs the provider's program as this user and keeps a credential in
    /// the host's keyring, and choose which accounts an agent entry offers. A key with it can attach any account to any harness entry,
    /// including an entry whose `peers` leave that key out, and can make an entry for a built-in source.
    Accounts,
    /// `discover/nearby`: the agents this host finds on the computer and the link it is on, for a client that cannot search itself.
    /// It makes the host send multicast queries and tells the key what answered; it starts and changes nothing.
    Discover,
    /// `usage/summary` and `usage/limits`: what the agents on this host spent, by account, harness, model and session, and the
    /// allowances they last reported. It reads the host's records and changes nothing; a delegated child connection never holds it.
    Usage,
    /// `workspace/openers` and `workspace/open`: what this computer has to open a folder with (VS Code, its file manager) and starting
    /// one for a folder in the places `folders` browses, as this user. The program starts on the host's screen.
    WorkspaceOpen,
    /// `fs/read`, and with `folders` an `fs/list` not kept to any place: this computer's files as the account the host runs as can
    /// read them. The grant is the only gate.
    Files,
    /// `fs/write`, together with `files`: writes a text file anywhere this computer's account can, but not a place on the person's
    /// protected list, and not canoa.cfg or the host's own state folder, which `files.write.host` is for. What a file holds can be what
    /// an agent or a program reads next, so it is as much as access to the agent; a delegated child connection never has it.
    FilesWrite,
    /// `fs/write` of canoa.cfg and of the host's own state folder, besides `files.write`: those files name the programs the host
    /// runs and the keys it trusts, so this is as much as access to the host itself.
    FilesWriteHost,
    /// `terminal/open`, `terminal/write`, `terminal/resize`, `terminal/scrollback` and `terminal/close`: the person's own shell on this
    /// computer, as this user, with output pushed back on the same link as `terminal/output`. It is a shell, so it is as much as
    /// access to the agent and to everything else this user can do here; a delegated child connection never holds it.
    Terminal,
    /// `speech/providers`, `speech/offers`, `speech/open`, `speech/push`, `speech/finish` and `speech/cancel`: audio goes to a speech provider the person
    /// set up on this host, which may run a program of theirs or send it to an address of theirs, and the text comes back. A key
    /// reaches only its own recordings, and no read answers a provider's key.
    Voice,
    /// `speech/configure` and `speech/enable`: defines or removes a speech provider in canoa.cfg (or makes the one of an offer from a
    /// program the Store installed), which names a program this host runs as this user or
    /// an address it sends audio to: as much as access to the agent.
    VoiceSetup,
    /// `state/get`, `state/put`, `state/append`, `state/list` and `state/remove`: named documents in the host's home, shared by every
    /// key that holds it, where an app keeps what a browser would lose: drafts, the window layout and the permission rules with
    /// their log. A remembered rule decides what an agent may do unasked, so this is as much as access to the agent.
    State,
    /// `host/keys`, `host/accept`, `host/revoke` and `host/pair`, and `[host]` through `config/get`, `config/set` and `config/check`:
    /// pair and remove the keys that may connect, show a PIN and edit what the host serves. A paired key reaches whatever its own
    /// grants let it, and `[host]` names the programs this host runs and the grants its keys hold, so this is as much as access to
    /// the agent.
    Hosting,
    /// Opening `@tools` to offer a program's tools (a browser extension's, an Office add-in's) to the sessions of this host. What
    /// such a provider answers goes to an agent as its tools' own results, so it is as much as access to the agent's context.
    ToolsProvide,
    /// The sessions of a connection whose key holds it are given the MCP servers of the session services (`canoa::session_service`)
    /// the host's libraries offer, each when it has something to give: the person's mail (`canoa-mail`: search, read, draft and
    /// organize the mail of the accounts the service's table names), for one. What a service gives is the person's own and what an
    /// agent reads of it enters its context, so a key gets it only when canoa.cfg says so; the application in this process holds
    /// it. Not a control method.
    Services,
    /// The control methods of the session services, each under the namespace of its service (`mail/accounts`, `mail/add`,
    /// `mail/test`, `mail/remove` for the mail service): what a service lets the person set up, such as the mail accounts of
    /// canoa.cfg, whose passwords the host keeps and whose servers the host connects to with them. What it sets up is what a key
    /// holding `services` then reads, so it is as much as access to the person's mail.
    ServicesSetup,
    /// `secrets/providers`, `secrets/list`, `secrets/reveal`, `secrets/forget` and `secrets/set`: the list of the secrets this host holds
    /// (where each is kept, its name, how long it has left), showing the value of one on request, forgetting one and setting one by a
    /// form sealed to the key that asked. A secret is whatever an agent may be given a handle to use, so this is as much as access to
    /// them. A key never holds it unless canoa.cfg gives it; the application in this process does.
    Secrets,
}

pub(super) struct GrantSpec {
    pub(super) grant: Grant,
    pub(super) name: &'static str,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "checked against ROUTES in tests")
    )]
    pub(super) control: bool,
}

pub(super) const GRANT_SPECS: [GrantSpec; 29] = [
    GrantSpec {
        grant: Grant::LedgerRead,
        name: "ledger.read",
        control: true,
    },
    GrantSpec {
        grant: Grant::LedgerWrite,
        name: "ledger.write",
        control: true,
    },
    GrantSpec {
        grant: Grant::Worktree,
        name: "worktree",
        control: true,
    },
    GrantSpec {
        grant: Grant::StoreRead,
        name: "store.read",
        control: true,
    },
    GrantSpec {
        grant: Grant::Folders,
        name: "folders",
        control: true,
    },
    GrantSpec {
        grant: Grant::WorkspaceOpen,
        name: "workspace.open",
        control: true,
    },
    GrantSpec {
        grant: Grant::ConfigRead,
        name: "config.read",
        control: true,
    },
    GrantSpec {
        grant: Grant::ConfigWrite,
        name: "config.write",
        control: true,
    },
    GrantSpec {
        grant: Grant::ConfigWriteSessions,
        name: "config.write.sessions",
        control: true,
    },
    GrantSpec {
        grant: Grant::Upload,
        name: "upload",
        control: true,
    },
    GrantSpec {
        grant: Grant::HostRestart,
        name: "host.restart",
        control: true,
    },
    GrantSpec {
        grant: Grant::OverlayRead,
        name: "overlay.read",
        control: true,
    },
    GrantSpec {
        grant: Grant::OverlayWrite,
        name: "overlay.write",
        control: true,
    },
    GrantSpec {
        grant: Grant::StorePrograms,
        name: "store.programs",
        control: true,
    },
    GrantSpec {
        grant: Grant::Accounts,
        name: "accounts",
        control: true,
    },
    GrantSpec {
        grant: Grant::Discover,
        name: "discover",
        control: true,
    },
    GrantSpec {
        grant: Grant::Usage,
        name: "usage",
        control: true,
    },
    GrantSpec {
        grant: Grant::Files,
        name: "files",
        control: true,
    },
    GrantSpec {
        grant: Grant::FilesWrite,
        name: "files.write",
        control: true,
    },
    GrantSpec {
        grant: Grant::FilesWriteHost,
        name: "files.write.host",
        control: false,
    },
    GrantSpec {
        grant: Grant::Terminal,
        name: "terminal",
        control: true,
    },
    GrantSpec {
        grant: Grant::Voice,
        name: "voice",
        control: true,
    },
    GrantSpec {
        grant: Grant::VoiceSetup,
        name: "voice.setup",
        control: true,
    },
    GrantSpec {
        grant: Grant::State,
        name: "state",
        control: true,
    },
    GrantSpec {
        grant: Grant::Hosting,
        name: "hosting",
        control: true,
    },
    GrantSpec {
        grant: Grant::ToolsProvide,
        name: "tools.provide",
        control: false,
    },
    GrantSpec {
        grant: Grant::Services,
        name: "services",
        control: false,
    },
    GrantSpec {
        grant: Grant::ServicesSetup,
        name: "services.setup",
        control: false,
    },
    GrantSpec {
        grant: Grant::Secrets,
        name: "secrets",
        control: true,
    },
];

impl Grant {
    /// Every grant: what a user's own browser holds on a host run for a portal.
    #[must_use]
    pub const fn all() -> [Self; 29] {
        [
            GRANT_SPECS[0].grant,
            GRANT_SPECS[1].grant,
            GRANT_SPECS[2].grant,
            GRANT_SPECS[3].grant,
            GRANT_SPECS[4].grant,
            GRANT_SPECS[5].grant,
            GRANT_SPECS[6].grant,
            GRANT_SPECS[7].grant,
            GRANT_SPECS[8].grant,
            GRANT_SPECS[9].grant,
            GRANT_SPECS[10].grant,
            GRANT_SPECS[11].grant,
            GRANT_SPECS[12].grant,
            GRANT_SPECS[13].grant,
            GRANT_SPECS[14].grant,
            GRANT_SPECS[15].grant,
            GRANT_SPECS[16].grant,
            GRANT_SPECS[17].grant,
            GRANT_SPECS[18].grant,
            GRANT_SPECS[19].grant,
            GRANT_SPECS[20].grant,
            GRANT_SPECS[21].grant,
            GRANT_SPECS[22].grant,
            GRANT_SPECS[23].grant,
            GRANT_SPECS[24].grant,
            GRANT_SPECS[25].grant,
            GRANT_SPECS[26].grant,
            GRANT_SPECS[27].grant,
            GRANT_SPECS[28].grant,
        ]
    }

    /// Its name in canoa.cfg.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        let mut index = 0;
        while index < GRANT_SPECS.len() {
            if GRANT_SPECS[index].grant as u8 == self as u8 {
                return GRANT_SPECS[index].name;
            }
            index += 1;
        }
        ""
    }

    /// The grant of that name.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        GRANT_SPECS
            .iter()
            .find(|spec| spec.name == name)
            .map(|spec| spec.grant)
    }
}

/// Who is asking.
#[derive(Clone, Debug)]
pub(crate) enum Caller {
    /// This process: every method, every record.
    Owner,
    /// A paired key, with the grants canoa.cfg gives it.
    Peer { id: String, grants: BTreeSet<Grant> },
}

impl Caller {
    /// What a record this caller puts is owned by.
    pub(super) fn owner(&self) -> &str {
        match self {
            Self::Owner => OWNER,
            Self::Peer { id, .. } => id,
        }
    }

    /// `host/grants`: the names of what this caller may do beyond the ledger, so an application shows only what it can use.
    pub(super) fn grants_json(&self) -> Value {
        let names: Vec<&str> = Grant::all()
            .into_iter()
            .filter(|grant| self.may(*grant))
            .map(Grant::as_str)
            .collect();
        json!({"grants": names})
    }

    pub(super) fn may(&self, grant: Grant) -> bool {
        match self {
            Self::Owner => true,
            Self::Peer { grants, .. } => grants.contains(&grant),
        }
    }

    /// Whether this caller sees and changes a record owned by `owner`.
    pub(super) fn reaches(&self, owner: &str) -> bool {
        match self {
            Self::Owner => true,
            Self::Peer { id, .. } => id == owner,
        }
    }
}

pub(super) fn allow(caller: &Caller, grant: Grant) -> Result<(), (i32, String)> {
    if caller.may(grant) {
        Ok(())
    } else {
        Err((DENIED, format!("this key has no {} grant", grant.as_str())))
    }
}
