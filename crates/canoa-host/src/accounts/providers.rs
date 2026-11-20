//! The account providers this host carries, each listed once with what it offers: every harness's own (an agent's accounts, signed
//! in the agent's way) and every maker (a proxy, a gateway, a model service), whose accounts a harness runs by the standards they
//! provide. An account is added through a provider, by one of that provider's own ways to sign in.

use std::sync::Arc;

use coqui_acp_api::{AccountOffer, AccountProvider};

use super::matching::Catalogs;
use crate::cli::Failed;
use crate::config::Config;
use crate::kind::Harness;
use crate::makers::{Maker, Makers};

/// One account provider and what it offers, with nothing of any other provider's.
pub(crate) struct Listed {
    /// The id of the account provider.
    pub(crate) id: String,
    /// The harness it belongs to when it is an agent's own; `None` for a maker.
    pub(crate) harness: Option<String>,
    /// Its name for people.
    pub(crate) title: String,
    /// What it is and what it keeps.
    pub(crate) description: String,
    /// The kinds it makes and the ways to sign in to them.
    pub(crate) offer: AccountOffer,
    /// The account provider, to sign in, renew and read usage with.
    pub(crate) provider: Arc<dyn AccountProvider>,
    /// The makers in force, which route a sign-in method the provider does not list itself.
    pub(crate) makers: Arc<Makers>,
    /// The harness the provider was reached through, which keeps the library the provider points into; dropped after it.
    _harness: Option<Box<Harness>>,
}

/// The account provider of the harness `harness`, when it has one this host can build.
fn of_harness(config: &Config, harness: &str) -> Option<Listed> {
    let built = crate::accounts::commands::provider_for(config, harness).ok()?;
    let id = built.account_provider_id()?.to_owned();
    let provider = Arc::clone(built.account_provider().ok()?);
    let about = provider.describe().ok()?;
    let offer = provider.offers().ok()?;
    Some(Listed {
        id,
        harness: Some(harness.to_owned()),
        title: about.title,
        description: about.description,
        offer,
        provider,
        makers: Arc::clone(built.makers()),
        _harness: Some(Box::new(built)),
    })
}

fn of_maker(maker: Maker, makers: &Arc<Makers>) -> Listed {
    let provider = maker.provider();
    let about = provider.describe().ok();
    Listed {
        id: maker.provider.clone(),
        harness: None,
        title: about
            .as_ref()
            .map_or_else(|| maker.provider.clone(), |about| about.title.clone()),
        description: about.map(|about| about.description).unwrap_or_default(),
        offer: maker.accounts.clone(),
        provider,
        makers: Arc::clone(makers),
        _harness: None,
    }
}

/// Every account provider this host can build: each harness's own, in the order the harnesses are listed, then the makers.
pub(crate) fn all(config: &Config) -> Vec<Listed> {
    let catalogs = Catalogs::new(config);
    let mut found: Vec<Listed> = catalogs
        .harnesses()
        .iter()
        .filter_map(|harness| of_harness(config, harness))
        .collect();
    let makers = Makers::of(config);
    found.extend(
        makers
            .all()
            .into_iter()
            .map(|maker| of_maker(maker, &makers)),
    );
    found
}

/// The account provider `id`.
///
/// # Errors
///
/// A usage failure naming the providers there are, when none has that id.
pub(crate) fn find(config: &Config, id: &str) -> Result<Listed, Failed> {
    let makers = Makers::of(config);
    if let Some(maker) = makers.all().into_iter().find(|maker| maker.provider == id) {
        return Ok(of_maker(maker, &makers));
    }
    for harness in Catalogs::new(config).harnesses() {
        if let Some(listed) = of_harness(config, &harness)
            && listed.id == id
        {
            return Ok(listed);
        }
    }
    let known: Vec<String> = all(config).into_iter().map(|listed| listed.id).collect();
    Err(Failed::usage(format!(
        "no account provider called {id}: this host has {}",
        known.join(", ")
    )))
}
