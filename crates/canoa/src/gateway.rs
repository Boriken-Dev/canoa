//! The `model_gateway` kind: a model that runs on this computer, served to an agent the way a cloud endpoint is.
//!
//! An account whose kind a gateway serves holds no credential for a service: its settings name a model and where it runs. When a
//! session starts on such an account the host asks the gateway to [`open`](ModelGateway::open) for it, and gives the agent what
//! comes back: the address of a model API on loopback and a bearer good for that session alone. The host owns which agent is
//! started, how that agent takes an address and a bearer (the harness's own account kind), who holds the account and when the
//! session ends; the gateway owns the server, the model and whatever runs it.
//!
//! The server a gateway runs is [`crate::loopback`]'s, so a request is answered only for a program of this user that shows the
//! bearer. A gateway that runs its model in a process of its own starts it through the `spawner` it is handed, and the host's own
//! commands through [`GatewayRequest::helper`].
//!
//! The models are plain data, the same in every language.

use alloc::string::String;
use alloc::vec::Vec;

use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Map, Schema, ToValue, Value};
use coqui_acp_api::{ModelListing, ProcessSpawner};

/// The name of the `model_gateway` kind.
pub const KIND: &str = "model_gateway";

/// Who a gateway serves and what it keeps, from [`ModelGateway::describe`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct GatewayInfo {
    /// The account kind whose sessions it serves. An `account_provider` of the same library offers the kind, so a person can make
    /// an account of it.
    pub kind: String,
    /// The vendor of what runs the model, in the words a session's identity variables use for an account's provider (`AGENTS_ACCOUNT_PROVIDER`).
    pub vendor: String,
    /// The name of the table of canoa.cfg that holds settings of the gateway's own, when it has any: the host reads it each time it
    /// opens a gateway and hands it over as [`GatewayRequest::table`].
    pub table: Option<String>,
}

impl GatewayInfo {
    /// A gateway for sessions of accounts of `kind`, whose model is run by `vendor`'s software, with no table of its own.
    #[must_use]
    pub fn new(kind: impl Into<String>, vendor: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            vendor: vendor.into(),
            table: None,
        }
    }
}

/// What the host asks of a gateway for one session, from [`ModelGateway::open`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct GatewayRequest {
    /// The account's id. A gateway shares one server among the sessions that hold the same account.
    pub account: String,
    /// The account's settings, as an object; the account kind's schema says what it holds.
    pub settings: Value,
    /// The gateway's own table of canoa.cfg ([`GatewayInfo::table`]), as an object as the file has it now: `{}` when the gateway
    /// names no table or the file has none.
    pub table: Value,
    /// The host's home folder, where a gateway may keep its logs and the files it makes.
    pub home: Option<String>,
    /// The command line that starts one of the host's own helper commands: the program, then its arguments. A gateway that runs
    /// part of itself in a process of its own appends the name of its library and the arguments that library's helper reads, and
    /// starts the result through its spawner. Empty when this host has no such command.
    #[map(default)]
    pub helper: Vec<String>,
}

impl GatewayRequest {
    /// A request for a session of `account`, with no settings, no table, no home and no helper command.
    #[must_use]
    pub fn new(account: impl Into<String>) -> Self {
        Self {
            account: account.into(),
            settings: Value::from(Map::new()),
            table: Value::from(Map::new()),
            home: None,
            helper: Vec::new(),
        }
    }
}

/// Serves a model that runs on this computer to the sessions of accounts of one kind. A provider is the same on every call; what
/// it keeps for one account is in the lease it opens.
#[coqui_acp_api::guatiao::kind(name = "model_gateway", crate = ::coqui_acp_api::guatiao)]
pub trait ModelGateway: Send + Sync {
    /// Who it serves. The same answer on every call, with nothing run.
    fn describe(&self) -> Result<GatewayInfo, ProviderError>;

    /// A bearer on the gateway of `request.account`, started when no session holds one: the server, and what runs the model.
    /// `spawner` is the host's, for any process the gateway starts. A refusal says what the person can act on (the model is not
    /// loaded, the service is not running), in words that name no value of the account.
    fn open(
        &self,
        request: GatewayRequest,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<Object<dyn GatewayLease>, ProviderError>;
}

/// One session's access to a gateway. Destroying it ends the session's access, and the last one for an account ends that
/// account's gateway.
#[coqui_acp_api::guatiao::kind(object, name = "gateway_lease", crate = ::coqui_acp_api::guatiao)]
pub trait GatewayLease: Send + Sync {
    /// Where the model's API is: `http://127.0.0.1:PORT`, with no path. An agent adds the path of the API it speaks.
    fn address(&self) -> Result<String, ProviderError>;

    /// The bearer the agent shows. It is good for this session alone, and for no one but a program of this user.
    fn bearer(&self) -> Result<String, ProviderError>;

    /// The models this gateway could serve, as an account provider lists the models of an account of an ordinary kind, with the
    /// one the account serves marked as the default. A gateway that cannot list them answers [`ModelListing::unknown`] with its
    /// reason.
    fn models(&self) -> Result<ModelListing, ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_starts_with_empty_settings_and_no_helper() {
        let request = GatewayRequest::new("id-1");
        assert_eq!(request.settings, Value::from(Map::new()));
        assert_eq!(request.table, Value::from(Map::new()));
        assert!(request.home.is_none() && request.helper.is_empty());
        assert!(GatewayInfo::new("a-kind", "a-vendor").table.is_none());
    }
}
