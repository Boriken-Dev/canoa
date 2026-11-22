//! The model gateways the libraries this host carries offer: a model that runs on this computer, served to an agent for the session
//! of an account the way a cloud endpoint is. Nothing here names a model's vendor. A library offers a `model_gateway` provider for
//! an account kind and, from the same library, the `account_provider` that makes accounts of the kind; the host joins that kind and
//! its sign-in method to the catalog of each harness that can be given an address and a bearer (its own account kind says so),
//! and when a session starts on such an account it opens the gateway for it and gives the harness what comes back.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and locks are std's"
)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use canoa::gateway::{GatewayInfo, GatewayLease, GatewayRequest, ModelGateway};
use coqui_acp_api::guatiao::Value;
use coqui_acp_api::guatiao::library::{Object, Registry};
use coqui_acp_api::{
    AccountGrant, AccountOffer, AccountProvider, Credentials, ModelListing, ProcessSpawner,
    UnknownKind,
};
use coqui_acp_harness::LocalSpawner;

use crate::cli::Program;
use crate::config::Settings;
use crate::provider::map_of;

/// One gateway and the account kind it serves, with what the library that offers it says of that kind.
#[derive(Clone)]
pub(crate) struct Offer {
    /// Who the gateway serves.
    pub(crate) info: GatewayInfo,
    gateway: Arc<dyn ModelGateway>,
    /// The id of the account provider that makes accounts of the kind.
    pub(crate) provider: String,
    /// What that provider offers: the kinds, and the sign-in methods.
    pub(crate) accounts: AccountOffer,
}

impl Offer {
    /// A gateway for the account kind its description names, whose accounts `maker` makes.
    pub(crate) fn new(
        gateway: Arc<dyn ModelGateway>,
        provider: impl Into<String>,
        maker: Arc<dyn AccountProvider>,
    ) -> Option<Self> {
        let info = gateway.describe().ok()?;
        let accounts = maker.offers().ok()?;
        accounts.kind(&info.kind)?;
        Some(Self {
            info,
            gateway,
            provider: provider.into(),
            accounts,
        })
    }

    /// The kind this gateway serves, as its account provider describes it.
    pub(crate) fn kind(&self) -> Option<&coqui_acp_api::AccountKindInfo> {
        self.accounts.kind(&self.info.kind)
    }
}

/// The gateways `registry` offers, each with the account provider of the same registry that makes the kind it serves.
fn offers_in(registry: &Registry) -> Vec<Offer> {
    let makers: Vec<(String, Arc<dyn AccountProvider>)> = registry
        .offers::<dyn AccountProvider>()
        .filter_map(|offer| Some((offer.id().to_owned(), offer.shared()?)))
        .collect();
    registry
        .offers::<dyn ModelGateway>()
        .filter_map(|offer| {
            let gateway = offer.shared()?;
            let kind = gateway.describe().ok()?.kind;
            let (provider, maker) = makers.iter().find(|(_, maker)| {
                maker
                    .offers()
                    .is_ok_and(|offered| offered.kind(&kind).is_some())
            })?;
            Offer::new(gateway, provider.clone(), Arc::clone(maker))
        })
        .collect()
}

/// The gateways the bundled libraries offer, found once.
fn bundled_offered() -> &'static [(Offer, &'static str)] {
    static OFFERED: OnceLock<Vec<(Offer, &'static str)>> = OnceLock::new();
    OFFERED.get_or_init(|| {
        crate::vendors::bundled_providers()
            .map(offers_in)
            .unwrap_or_default()
            .into_iter()
            .map(|offer| {
                let vendor: &'static str = Box::leak(offer.info.vendor.clone().into_boxed_str());
                (offer, vendor)
            })
            .collect()
    })
}

/// Gateway routes offered by extension agent vendors noted in this process, with the agent id each came from.
static EXTRA_OFFERS: std::sync::Mutex<Vec<(Offer, &'static str)>> =
    std::sync::Mutex::new(Vec::new());

/// Keeps any gateways `registry` offers that are not already kept.
pub(crate) fn note(registry: &Registry) {
    let bundled = bundled_offered();
    let mut extra = EXTRA_OFFERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for offer in offers_in(registry) {
        if bundled
            .iter()
            .chain(extra.iter())
            .all(|(existing, _)| existing.info.kind != offer.info.kind)
        {
            let vendor: &'static str = Box::leak(offer.info.vendor.clone().into_boxed_str());
            extra.push((offer, vendor));
        }
    }
}

/// The gateways offered across `registries`, each kind once (first registry wins).
pub(crate) fn offered_in(registries: &[(&str, &Registry)]) -> Vec<Offer> {
    let mut offers = Vec::new();
    for (_, registry) in registries {
        for offer in offers_in(registry) {
            if offers
                .iter()
                .all(|existing: &Offer| existing.info.kind != offer.info.kind)
            {
                offers.push(offer);
            }
        }
    }
    offers
}

/// The gateways the bundled libraries and any noted extension libraries offer.
pub(crate) fn offered() -> Vec<Offer> {
    let bundled = bundled_offered();
    let extra = EXTRA_OFFERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    bundled
        .iter()
        .chain(extra.iter())
        .map(|(offer, _)| offer.clone())
        .collect()
}

/// The gateway that serves sessions of accounts of `kind`, when a bundled or noted library offers one.
pub(crate) fn for_kind(kind: &str) -> Option<Offer> {
    offered().into_iter().find(|offer| offer.info.kind == kind)
}

/// The vendor of what runs the model of accounts of `kind`, in the words a session's identity variables use, when a gateway
/// serves the kind.
pub(crate) fn vendor_of(kind: &str) -> Option<&'static str> {
    bundled_offered()
        .iter()
        .find(|(offer, _)| offer.info.kind == kind)
        .map(|(_, vendor)| *vendor)
        .or_else(|| {
            EXTRA_OFFERS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .find(|(offer, _)| offer.info.kind == kind)
                .map(|(_, vendor)| *vendor)
        })
}

/// What the gateways need of the host: where its home is, which canoa.cfg to read each gateway's table from, and how to start the
/// host's own helper command.
#[derive(Default)]
pub(crate) struct Gateways {
    home: Option<PathBuf>,
    config: OnceLock<PathBuf>,
    helper: OnceLock<Program>,
    /// What a test serves in place of the libraries' gateways.
    #[cfg(test)]
    stand_ins: std::sync::Mutex<Vec<Offer>>,
}

impl core::fmt::Debug for Gateways {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Gateways").finish_non_exhaustive()
    }
}

impl Gateways {
    /// Gateways that keep their files in the host's `home`.
    pub(crate) fn in_home(home: Option<&Path>) -> Self {
        Self {
            home: home.map(Path::to_path_buf),
            ..Self::default()
        }
    }

    /// Reads each gateway's own table from the canoa.cfg at `file` whenever it opens one, once.
    pub(crate) fn table_from(&self, file: PathBuf) {
        let _ = self.config.set(file);
    }

    /// Names the program that starts the host's own helper command, once.
    pub(crate) fn helper(&self, program: Program) {
        let _ = self.helper.set(program);
    }

    /// Serves `offers` in place of the libraries' gateways.
    #[cfg(test)]
    pub(crate) fn serving(&self, offers: Vec<Offer>) {
        *self
            .stand_ins
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = offers;
    }

    /// The gateway that serves sessions of accounts of `kind`.
    pub(crate) fn offer(&self, kind: &str) -> Option<Offer> {
        #[cfg(test)]
        if let Some(found) = self
            .stand_ins
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .find(|offer| offer.info.kind == kind)
        {
            return Some(found.clone());
        }
        for_kind(kind)
    }

    /// Whether a gateway serves accounts of `kind`.
    pub(crate) fn serves(&self, kind: &str) -> bool {
        self.offer(kind).is_some()
    }

    /// The command line that starts the host's helper command, when the host has a program of its own.
    fn helper_line(&self) -> Vec<String> {
        self.helper
            .get()
            .map(|program| {
                let mut line = vec![program.path.to_string_lossy().into_owned()];
                line.extend(
                    program
                        .args
                        .iter()
                        .map(|arg| arg.to_string_lossy().into_owned()),
                );
                line.push(crate::cli::HELPER.to_owned());
                line
            })
            .unwrap_or_default()
    }

    /// A lease on the gateway of the account `grant` is, started when no session holds one.
    ///
    /// # Errors
    ///
    /// No gateway serves the kind, the settings are not a table, or the gateway refuses: its words are the person's to read.
    pub(crate) fn open(&self, grant: &AccountGrant) -> Result<Lease, String> {
        let offer = self
            .offer(&grant.kind)
            .ok_or_else(|| format!("no gateway serves accounts of kind {}", grant.kind))?;
        let settings =
            guatiao_serde::adapt::from_value::<serde_json::Map<String, serde_json::Value>>(
                &grant.settings,
            )
            .map_err(|_| "the account's settings are not a table".to_owned())?;
        let table = match (&offer.info.table, self.config.get()) {
            (Some(name), Some(file)) => crate::control::table_in(file, name),
            _ => serde_json::Value::Object(serde_json::Map::new()),
        };
        let mut request = GatewayRequest::new(grant.id.clone());
        request.settings = guatiao_serde::from_serde(serde_json::Value::Object(settings))
            .map_err(|why| why.to_string())?;
        request.table = guatiao_serde::from_serde(table).map_err(|why| why.to_string())?;
        request.home = self
            .home
            .as_ref()
            .map(|home| home.to_string_lossy().into_owned());
        request.helper = self.helper_line();
        let spawner: Object<dyn ProcessSpawner> = LocalSpawner::new().into_object();
        let object = offer.gateway.open(request, spawner).map_err(|error| {
            let why = error.message();
            if why.is_empty() {
                "the gateway could not be started".to_owned()
            } else {
                why.to_owned()
            }
        })?;
        let (address, bearer) = object
            .address()
            .and_then(|address| Ok((address, object.bearer()?)))
            .map_err(|error| error.message().to_owned())?;
        Ok(Lease {
            object,
            address,
            bearer,
        })
    }
}

/// A session's bearer on a gateway; dropping it ends the session's access.
pub(crate) struct Lease {
    object: Object<dyn GatewayLease>,
    address: String,
    bearer: String,
}

impl Lease {
    /// The address its agent is pointed at, without a path.
    pub(crate) fn address(&self) -> &str {
        &self.address
    }

    /// The bearer an agent shows.
    pub(crate) fn bearer(&self) -> &str {
        &self.bearer
    }

    /// The models the gateway could serve for the account, as the gateway lists them.
    pub(crate) fn models(&self) -> ModelListing {
        self.object
            .models()
            .unwrap_or_else(|error| ModelListing::unavailable(UnknownKind::Failed, error.message()))
    }
}

impl core::fmt::Debug for Lease {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // The bearer is not shown.
        f.debug_struct("Lease").finish_non_exhaustive()
    }
}

/// Why a session could not be given its gateway.
pub(crate) type Why = String;

/// The grant a harness is given in place of `grant`, a gateway-served account's: the harness's own kind, the gateway's address and
/// the session's bearer; and the files its folder needs. The lease is held for as long as the session lives.
///
/// # Errors
///
/// A harness no gateway can be given to, or a gateway that could not start.
pub(crate) fn grant(
    gateways: &Gateways,
    harness: &str,
    grant: &AccountGrant,
) -> Result<(AccountGrant, Lease), Why> {
    let route = crate::vendors::gateway_route(harness)
        .ok_or_else(|| format!("{harness} cannot be given a gateway"))?;
    let lease = gateways.open(grant)?;
    let address = format!("{}{}", lease.address(), route.suffix);
    let mut credentials = Credentials::new();
    credentials.insert(route.credential.clone(), lease.bearer().to_owned());
    let mut kept = serde_json::Map::new();
    if let Some(key) = &route.address {
        kept.insert(key.clone(), serde_json::Value::String(address.clone()));
    }
    for fixed in &route.fixed {
        kept.insert(
            fixed.name.clone(),
            serde_json::Value::String(fixed.value.clone()),
        );
    }
    let mut mapped = AccountGrant::new(grant.id.clone(), route.kind.clone())
        .settings(Value::from(map_of(&Settings::new(kept))?))
        .credentials(credentials)
        .revision(grant.revision);
    if let Some(dir) = &grant.state_dir {
        mapped = mapped.state_dir(dir.clone());
        prepare(harness, &address, Path::new(dir))?;
    }
    Ok((mapped, lease))
}

/// Writes what the harness reads from its folder to use the gateway at `address`, as its description says: its configuration
/// that turns off what a gateway cannot serve, or the file that points a provider at the gateway.
fn prepare(harness: &str, address: &str, dir: &Path) -> Result<(), Why> {
    let files = crate::vendors::behave(harness, |vendor| vendor.gateway_files(address.to_owned()))
        .and_then(Result::ok)
        .unwrap_or_default();
    for each in files {
        let file = dir.join(&each.path);
        std::fs::write(&file, each.text).map_err(|error| format!("{}: {error}", file.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
