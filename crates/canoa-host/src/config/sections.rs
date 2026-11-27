//! Host subsection parsers (`history`, `native`, `control`, `accounts`, `pools`, `web`) and error formatting.

use alloc::collections::BTreeSet;
use alloc::string::{String, ToString};
use canoa::brand::CONFIG_FILE;
use canoa::brand::PROGRAM;
use core::time::Duration;
use std::path::{Component, Path, PathBuf};

use coqui_peer_webcrypto::PeerId;
use indexmap::IndexMap;

use super::profiles::settings_of;
use super::raw::{
    RawAccount, RawControl, RawFile, RawHistory, RawNative, RawPool, RawRole, RawWeb,
};
use super::{
    AccountConfig, ConfigError, ControlConfig, Entry, Handover, HistoryConfig, HostProfile, Listen,
    MAX_NAME, MAX_TITLE, NativeConfig, PoolConfig, PoolIssue, RoleConfig, RoleMember, WebConfig,
    WebMode, WebTls,
};
use crate::format::formats;

/// `[host.history]`. Errors name a key, never a value.
pub(super) fn history_of(raw: RawHistory) -> Result<HistoryConfig, ConfigError> {
    let refused = |what: &str| ConfigError::Invalid {
        entry: None,
        what: format!("host.history: {what}"),
    };
    let default = HistoryConfig::default();
    let config = HistoryConfig {
        record: raw.record.unwrap_or(default.record),
        max_session_bytes: raw.max_session_bytes.unwrap_or(default.max_session_bytes),
        max_sessions: raw.max_sessions.unwrap_or(default.max_sessions),
        max_total_bytes: raw.max_total_bytes.unwrap_or(default.max_total_bytes),
        store: raw.store.filter(|id| !id.is_empty()),
        store_settings: match &raw.store_settings {
            None => default.store_settings,
            Some(table) => serde_json::to_string(table)
                .map_err(|_| refused("store_settings is a table of the store's settings"))?,
        },
    };
    if config.max_session_bytes == 0 || config.max_sessions == 0 || config.max_total_bytes == 0 {
        return Err(refused(
            "max_session_bytes, max_sessions and max_total_bytes are at least 1; record = false records nothing",
        ));
    }
    Ok(config)
}

/// `[host.native]`. Errors name a key, never a value.
pub(super) fn native_of(raw: RawNative) -> Result<NativeConfig, ConfigError> {
    let refused = |what: &str| ConfigError::Invalid {
        entry: None,
        what: format!("host.native: {what}"),
    };
    let default = NativeConfig::default();
    let control = match raw.control {
        None => default.control,
        Some(names) => {
            let mut set = BTreeSet::new();
            for name in names {
                set.insert(
                    crate::control::Grant::named(&name)
                        .ok_or_else(|| refused("control names a grant that does not exist"))?,
                );
            }
            // Changing what a program may be given or run reaches as far as the agent's own access.
            if set.iter().any(|grant| {
                matches!(
                    grant,
                    crate::control::Grant::ConfigWriteSessions
                        | crate::control::Grant::OverlayWrite
                        | crate::control::Grant::HostRestart
                        | crate::control::Grant::Upload
                        | crate::control::Grant::FilesWrite
                        | crate::control::Grant::FilesWriteHost
                        | crate::control::Grant::Terminal
                        | crate::control::Grant::VoiceSetup
                        | crate::control::Grant::Hosting
                )
            }) {
                return Err(refused(
                    "control cannot hold config.write.sessions, overlay.write, host.restart, upload, files.write, files.write.host, terminal, voice.setup or hosting: a browser key does not get them through the door",
                ));
            }
            set
        }
    };
    let launch = raw.program.map(|program| crate::native::Launch {
        program: program.into(),
        args: raw.args.unwrap_or_default(),
    });
    Ok(NativeConfig {
        enabled: raw.enabled.unwrap_or(default.enabled),
        control,
        launch,
    })
}

/// `[host.control]`; errors name a key, never a value. `admits` is whether the add-in's own page is admitted (`[host.web] office_local`), whose key is not known
/// to canoa.cfg: a channel with no key under `grants` is then not one that turns every key away.
pub(super) fn control_of(raw: RawControl, admits: bool) -> Result<ControlConfig, ConfigError> {
    let refused = |what: &str| ConfigError::Invalid {
        entry: None,
        what: format!("host.control: {what}"),
    };
    let listen = raw
        .listen
        .map(|text| Listen::parse(&text).map_err(|what| refused(&format!("listen: {what}"))))
        .transpose()?;
    let mut grants = IndexMap::new();
    for (peer, names) in raw.grants {
        let id = peer
            .parse::<PeerId>()
            .map_err(|_| refused("a key under grants is not a peer id"))?;
        let mut set = BTreeSet::new();
        for name in names {
            set.insert(crate::control::Grant::named(&name).ok_or_else(|| {
                if name == "store.write" {
                    refused("store.write is no grant: name store.programs to let a key install programs")
                } else {
                    refused("a grant is not one this host knows")
                }
            })?);
        }
        grants.insert(id, set);
    }
    let max_records = raw
        .max_records
        .unwrap_or(crate::control::DEFAULT_MAX_RECORDS);
    let max_record_bytes = raw
        .max_record_bytes
        .unwrap_or(crate::control::DEFAULT_MAX_RECORD_BYTES);
    if max_records == 0 || max_record_bytes == 0 {
        return Err(refused("max_records and max_record_bytes are at least 1"));
    }
    let max_upload_bytes = raw
        .max_upload_bytes
        .unwrap_or(crate::control::DEFAULT_MAX_UPLOAD_BYTES);
    let max_upload_total_bytes = raw
        .max_upload_total_bytes
        .unwrap_or(crate::control::DEFAULT_MAX_UPLOAD_TOTAL);
    let max_uploads = raw
        .max_uploads
        .unwrap_or(crate::control::DEFAULT_MAX_UPLOADS);
    if max_upload_bytes == 0 || max_upload_total_bytes == 0 || max_uploads == 0 {
        return Err(refused(
            "max_upload_bytes, max_upload_total_bytes and max_uploads are at least 1",
        ));
    }
    if listen.is_some() && grants.is_empty() && !admits {
        return Err(refused(
            "listen without a key under grants would turn every key away",
        ));
    }
    Ok(ControlConfig {
        listen,
        grants,
        max_records,
        max_record_bytes,
        max_upload_bytes,
        max_upload_total_bytes,
        max_uploads,
    })
}

pub(super) fn invalid(name: &str, what: &str) -> ConfigError {
    ConfigError::Invalid {
        entry: Some(name.to_owned()),
        what: what.to_owned(),
    }
}

/// `base` made absolute; an empty one is the current directory, as a path joined to it is taken from there.
pub(super) fn directory_of(base: &Path) -> PathBuf {
    if base.as_os_str().is_empty() {
        return std::env::current_dir().unwrap_or_default();
    }
    std::path::absolute(base).unwrap_or_else(|_| base.to_owned())
}

/// A title is shown to a person in place of the name: not blank, no control characters, at most [`MAX_TITLE`] characters.
pub(super) fn check_title(name: &str, title: &str) -> Result<(), ConfigError> {
    if title.trim().is_empty() {
        return Err(invalid(name, "title is blank"));
    }
    if title.chars().any(char::is_control) {
        return Err(invalid(name, "title holds a control character"));
    }
    if title.chars().count() > MAX_TITLE {
        return Err(invalid(
            name,
            &format!("title is longer than {MAX_TITLE} characters"),
        ));
    }
    Ok(())
}

/// One `[host.accounts.NAME]`. Errors name a key, never a value: a credential written here is refused, and not quoted.
pub(super) fn account_of(name: &str, raw: RawAccount) -> Result<AccountConfig, ConfigError> {
    check_label(name)?;
    let refused = |what: String| ConfigError::Invalid {
        entry: None,
        what: format!("account {name}: {what}"),
    };
    if raw.rest.contains_key("harness") {
        return Err(refused(
            "harness is not a key of an account: it names its kind, and the account provider that offers the kind made it".to_owned(),
        ));
    }
    if let Some(key) = raw.rest.keys().next() {
        return Err(refused(format!(
            "{key} is not a key of an account, and a credential never goes in {CONFIG_FILE}: `{PROGRAM} accounts add` keeps it in the account store"
        )));
    }
    let text = |value: Option<String>, key: &str| match value {
        Some(value) if !value.is_empty() => Ok(value),
        _ => Err(refused(format!("{key} is missing or empty"))),
    };
    let id = text(raw.id, "id")?;
    if !coqui_acp_api::is_account_id(&id) {
        return Err(refused(
            "id is 1 to 128 bytes with no control character".to_owned(),
        ));
    }
    let settings = raw
        .settings
        .map(|table| settings_of(name, table))
        .transpose()?;
    Ok(AccountConfig {
        owner: None,
        kind: text(raw.kind, "kind")?,
        id,
        settings,
        route: raw
            .route
            .map(|route| text(Some(route), "route"))
            .transpose()?,
    })
}

/// One `[host.pools.NAME]`. A pool is a name for a set of accounts, so it may not be another account's or the host's own
/// `any-<standard>` pools', and its members are accounts that exist; that they provide its standard is for [`crate::pools`] to
/// check, which knows the kinds.
pub(super) fn pool_of(
    name: &str,
    raw: RawPool,
    accounts: &IndexMap<String, AccountConfig>,
) -> Result<PoolConfig, ConfigError> {
    let refused = |what: String| ConfigError::Invalid {
        entry: None,
        what: format!("pool {name}: {what}"),
    };
    let standard = raw
        .standard
        .ok_or_else(|| refused("standard is missing".to_owned()))?;
    let strategy = match raw.strategy.as_deref() {
        None => coqui_acp_pool::Strategy::RoundRobin,
        Some(text) => text
            .parse()
            .map_err(|error: coqui_acp_pool::UnknownStrategy| {
                refused(format!("strategy: {error}"))
            })?,
    };
    let pool = PoolConfig {
        standard,
        accounts: raw.accounts,
        strategy,
    };
    if let Err(issue) = pool.validate(name, accounts) {
        return Err(match issue {
            PoolIssue::UnknownMember(account) => refused(format!(
                "accounts names {account}, which [host.accounts] does not hold"
            )),
            PoolIssue::DuplicateMember(account) => {
                refused(format!("accounts names {account} twice"))
            }
            PoolIssue::NameTaken(_) => refused(
                "that name is an account's, and a pool and an account do not share one".to_owned(),
            ),
            other => refused(other.to_string()),
        });
    }
    Ok(pool)
}

/// What a role's members are checked against: the accounts, the pools and the profiles of the file.
pub(super) type Held<'a> = (
    &'a IndexMap<String, AccountConfig>,
    &'a IndexMap<String, PoolConfig>,
    &'a IndexMap<String, HostProfile>,
);

/// One `[host.roles.NAME]`: its name, strategy, handover and members, with each member's account and profile checked against what the
/// file holds (`accounts`, `pools` and `profiles`).
pub(super) fn role_of(
    name: &str,
    raw: RawRole,
    (accounts, pools, profiles): Held<'_>,
) -> Result<RoleConfig, ConfigError> {
    let refused = |what: String| ConfigError::Invalid {
        entry: None,
        what: format!("role {name}: {what}"),
    };
    if name.chars().count() > MAX_ROLE_NAME
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        || name.is_empty()
    {
        return Err(refused(format!(
            "a role's name is 1 to {MAX_ROLE_NAME} lower-case letters, digits, hyphens and underscores"
        )));
    }
    if let Some(title) = &raw.title {
        check_title(name, title)?;
    }
    let strategy = match raw.strategy.as_deref() {
        None => coqui_acp_pool::Strategy::Priority,
        Some(text) => text
            .parse()
            .map_err(|error: coqui_acp_pool::UnknownStrategy| {
                refused(format!("strategy: {error}"))
            })?,
    };
    let handover = match raw.handover.as_deref() {
        None | Some("auto") => Handover::Auto,
        Some("ask") => Handover::Ask,
        Some(_) => return Err(refused("handover is auto or ask".to_owned())),
    };
    let mut members = Vec::new();
    for (at, member) in raw.members.into_iter().enumerate() {
        let place = format!("members[{at}]");
        let agent = member
            .agent
            .filter(|agent| !agent.trim().is_empty())
            .ok_or_else(|| refused(format!("{place}: agent is missing")))?;
        if let Some(account) = &member.account
            && account != coqui_acp_api::LOCAL_ACCOUNT
            && !accounts.contains_key(account)
            && !pools.contains_key(account)
            && coqui_acp_pool::accounts::automatic_standard(account).is_none()
        {
            return Err(refused(format!(
                "{place}: account names {account}, which neither [host.accounts] nor [host.pools] holds"
            )));
        }
        if let Some(profile) = &member.profile
            && !profiles.contains_key(profile)
        {
            return Err(refused(format!(
                "{place}: profile names {profile}, which [host.profiles] does not hold"
            )));
        }
        members.push(RoleMember {
            agent,
            account: member.account,
            model: member.model,
            effort: member.effort,
            profile: member.profile,
            priority: member.priority.unwrap_or(0),
            max_concurrent: member.max_concurrent,
            stop_permille: member.stop_permille,
        });
    }
    let role = RoleConfig {
        title: raw.title,
        strategy,
        needs: raw.needs,
        stick: raw.stick.unwrap_or(true),
        handover,
        resume_prompt: raw.resume_prompt.filter(|text| !text.trim().is_empty()),
        retry_task: raw.retry_task.unwrap_or(false),
        members,
    };
    let ids: Vec<String> = role.members.iter().map(RoleMember::id).collect();
    if let Err(issue) = coqui_acp_pool::check_pool(
        name,
        None,
        &ids,
        &EveryMember,
        &coqui_acp_pool::Settings::default(),
    ) {
        return Err(match issue {
            coqui_acp_pool::PoolIssue::DuplicateMember(_) => refused(
                "two members name the same agent, account, model, effort and profile".to_owned(),
            ),
            other => refused(other.to_string()),
        });
    }
    Ok(role)
}

/// The most characters a role's name has.
const MAX_ROLE_NAME: usize = 32;

/// A role's members are checked by the host's own lookups, so the shared check asks nothing more of them.
struct EveryMember;

impl coqui_acp_pool::Known for EveryMember {
    fn taken(&self, _name: &str) -> bool {
        false
    }

    fn member(&self, _name: &str) -> bool {
        true
    }
}

/// A name the person gives an account or a pool: any text they can read, 1 to [`MAX_NAME`] characters and no control character. It
/// is a key of canoa.cfg, quoted where it has to be, and is never part of an address.
pub(crate) fn check_label(name: &str) -> Result<(), ConfigError> {
    if name.is_empty() || name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        return Err(ConfigError::Invalid {
            entry: None,
            what: format!("a name has 1 to {MAX_NAME} characters, none of them control characters"),
        });
    }
    Ok(())
}

/// Checked before anything puts the name in an address, which only letters, digits, hyphens and underscores are safe in.
pub(crate) fn check_name(name: &str) -> Result<(), ConfigError> {
    if name.is_empty()
        || name.len() > MAX_NAME
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(ConfigError::Invalid {
            entry: None,
            what: format!(
                "an entry name has 1 to {MAX_NAME} letters, digits, hyphens or underscores"
            ),
        });
    }
    Ok(())
}

/// `[host.web]`, its paths made absolute from canoa.cfg's directory. Errors name a setting, never a value.
pub(super) fn web_of(
    raw: RawWeb,
    directory: &Path,
    entries: &[Entry],
) -> Result<WebConfig, ConfigError> {
    let refused = |what: &str| ConfigError::Invalid {
        entry: None,
        what: format!("web: {what}"),
    };
    let path = |text: String, setting: &str| {
        if text.is_empty() {
            return Err(refused(&format!("{setting} is empty")));
        }
        let path = directory.join(text);
        Ok(std::path::absolute(&path).unwrap_or(path))
    };
    let mut web = WebConfig::new();
    web.dir = raw.dir.map(|dir| path(dir, "dir")).transpose()?;
    if let Some(listen) = raw.listen {
        web.listen = listen
            .parse()
            .map_err(|_| refused("listen is not an address and port"))?;
    }
    web.tls = match (raw.cert, raw.key) {
        (Some(cert), Some(key)) => Some(WebTls::new(path(cert, "cert")?, path(key, "key")?)),
        (None, None) => None,
        _ => return Err(refused("cert and key go together")),
    };
    // Unlike an entry's link, the page is not sealed: whoever is on the path can rewrite the code that holds the person's
    // key, and a browser gives plain HTTP no secure context for WebCrypto.
    if web.tls.is_none() && !web.listen.ip().is_loopback() {
        return Err(refused(
            "listen is beyond loopback, where plain HTTP is refused: give cert and key",
        ));
    }
    for host in raw.hosts {
        let plain = !host.is_empty()
            && host.len() <= 253
            && host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.');
        if !plain {
            return Err(refused(
                "a host is not a name of letters, digits, hyphens and dots",
            ));
        }
        web.hosts.push(host.to_ascii_lowercase());
    }
    for source in raw.connect {
        if !crate::web::connect_source(&source) {
            return Err(refused(
                "a connect source is not http, https, ws or wss and a host, with an optional port and path",
            ));
        }
        web.connect.push(source);
    }
    web.office = raw.office.unwrap_or(false);
    if !raw.office_frames.is_empty() && !web.office {
        return Err(refused("office_frames needs office = true"));
    }
    for source in raw.office_frames {
        if !crate::web::frame_source(&source) {
            return Err(refused(
                "an office_frames source is not https, a host (a leading *. is allowed) and an optional port",
            ));
        }
        web.office_frames.push(source);
    }
    web.office_local = raw.office_local.unwrap_or(false);
    if web.office_local && !web.office {
        return Err(refused("office_local needs office = true"));
    }
    if web.office_local && !web.listen.ip().is_loopback() {
        return Err(refused(
            "office_local needs a loopback listen address: behind another address or a proxy every request looks local",
        ));
    }
    web.office_secret = raw.office_secret.unwrap_or(false);
    if web.office_secret && !web.office_local {
        return Err(refused("office_secret needs office_local = true"));
    }
    web.mode = match raw.mode.as_deref() {
        None => None,
        Some("local") => Some(WebMode::Local),
        Some("remote") => Some(WebMode::Remote),
        Some(_) => return Err(refused("mode is local or remote")),
    };
    let count = |value: Option<usize>, setting: &str, default: usize| match value {
        Some(0) => Err(refused(&format!("{setting} must be at least 1"))),
        Some(value) => Ok(value),
        None => Ok(default),
    };
    let seconds = |value: Option<u64>, setting: &str, default: Duration| match value {
        Some(0) => Err(refused(&format!("{setting} must be at least 1"))),
        Some(value) => Ok(Duration::from_secs(value)),
        None => Ok(default),
    };
    let limits = web.limits;
    web.limits = limits
        .with_max_connections(count(
            raw.max_connections,
            "max_connections",
            limits.max_connections,
        )?)
        .with_max_connections_per_address(count(
            raw.max_connections_per_address,
            "max_connections_per_address",
            limits.max_connections_per_address,
        )?)
        .with_connection_time(seconds(
            raw.connection_seconds,
            "connection_seconds",
            limits.connection_time,
        )?)
        .with_max_request_bytes(count(
            raw.max_request_bytes,
            "max_request_bytes",
            limits.max_request_bytes,
        )?)
        .with_max_requests(count(
            raw.max_requests,
            "max_requests",
            limits.max_requests,
        )?)
        .with_request_time(seconds(
            raw.request_seconds,
            "request_seconds",
            limits.request_time,
        )?)
        .with_idle_time(seconds(raw.idle_seconds, "idle_seconds", limits.idle_time)?)
        .with_send_time(seconds(raw.send_seconds, "send_seconds", limits.send_time)?)
        .with_immutable_age(
            raw.immutable_seconds
                .map_or(limits.immutable_age, Duration::from_secs),
        );
    if web.listen.port() != 0
        && entries
            .iter()
            .any(|entry| entry.listen.address() == Some(web.listen))
    {
        return Err(refused("listen is an entry's address too"));
    }
    Ok(web)
}

/// Absolute, with no `.` or `..`: such a path is compared as written, so one of those could step outside it. The text is
/// checked as well as the components, since a Unix path's components drop a `.` in the middle.
pub(crate) fn plain_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| !matches!(component, Component::CurDir | Component::ParentDir))
        && !path
            .to_string_lossy()
            .split(['/', '\\'])
            .any(|segment| segment == "." || segment == "..")
}

pub(super) fn read_raw(bytes: &[u8], name: Option<&str>) -> Result<RawFile, anycfg::Error> {
    let options = name.map_or_else(anycfg::Options::new, |name| {
        anycfg::Options::new().name(name)
    });
    formats().from_slice_typed::<RawFile>(bytes, options)
}

/// What a key the host does not know is told as. Its name is a person's own text and may be a pasted secret, so it is told
/// only for a key this host once read, which says what replaced it; any other is told as "a setting this host does not know".
fn unknown_key(table: &str, key: &str) -> String {
    match (table, key) {
        ("host", "account_store") => format!(
            "{key} is not a setting this host knows; `accounts` under `[secrets]` replaced it (`file` is `file`, `os` is `system`)"
        ),
        (table, "own_profile") if table.starts_with("host.agents.") => format!(
            "{key} is not a setting this host knows; an entry that names no `profiles` gets {CONFIG_FILE}'s top level, and one that should run bare names an empty profile"
        ),
        _ => "a setting this host does not know".to_owned(),
    }
}

/// anycfg's refusal as ours: the position or the key's path, and its words only where they come from the schema. A value,
/// or a key nobody declared (bar a removed one that `unknown_key` knows), is never told.
pub(super) fn config_error(error: &anycfg::Error, file: Option<&Path>) -> ConfigError {
    use anycfg::Error;
    let parse = |file: Option<&Path>, line, column, what: String| ConfigError::Parse {
        file: file.map(Path::to_owned),
        line,
        column,
        what,
    };
    match error {
        Error::Source { cause, .. } => config_error(cause, file),
        Error::File { path, cause } => config_error(cause, Some(path)),
        Error::Parse {
            format,
            line,
            column,
            message,
        } => parse(
            file,
            *line,
            *column,
            message
                .clone()
                .unwrap_or_else(|| format!("not valid {}", format.to_uppercase())),
        ),
        Error::Conversion { path, message } => {
            let message = message.as_deref().unwrap_or("not of the expected shape");
            let (path, what) = if message.starts_with("unknown field") {
                // The unknown key is the last of the path.
                match path.as_deref().map(|path| path.rsplit_once('.')) {
                    Some(Some((parent, key))) => {
                        (Some(parent.to_owned()), unknown_key(parent, key))
                    }
                    Some(None) => (None, unknown_key("", path.as_deref().unwrap_or_default())),
                    None => (None, "a setting this host does not know".to_owned()),
                }
            } else if message.starts_with("duplicate field") {
                (path.clone(), "a key is given twice".to_owned())
            } else {
                (path.clone(), message.to_owned())
            };
            let what = match path {
                Some(path) if !path.is_empty() => format!("{path}: {what}"),
                _ => what,
            };
            parse(file, None, None, what)
        }
        Error::UnsupportedFormat { registered } => parse(
            file,
            Some(1),
            None,
            format!(
                "the first line names a format this build does not read; it reads {}",
                registered.join(", ")
            ),
        ),
        other => parse(file, None, None, other.to_string()),
    }
}
