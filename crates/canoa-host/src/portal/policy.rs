//! The sign-in policy: a `coqui_sign_in_policy` provider (the rules provider by default) decides which of the portal's methods a
//! connection may use, and whether the steps a sign-in has passed are enough.
//!
//! The portal asks it with the client's address as the proxy reported it (`None` when it could not be determined), never a
//! placeholder, and refuses whatever it cannot find out: an error from the policy offers nothing.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::net::IpAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use coqui_access::guatiao::library::Registry;
use coqui_access::{
    Factor, Person, SignInContext, SignInDecision, SignInPolicy, clients, purposes,
};
use coqui_access_signin::SignInRules;

use super::access::{check_library, new_registry, open_library, value_of};
use super::settings::Settings;
use super::signin::Method;

/// The provider id of the rules provider, which is built in and which `sign_in_policy` names when it names none.
pub const RULES: &str = "coqui_signin_rules";

/// What the policy is told the portal is.
const SERVICE: &str = "portal";

/// A method the policy offers and the providers it may be asked of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offered {
    /// The method, as the portal knows it.
    pub method: Method,
    /// The providers that may be asked for it, in order; empty is every provider that takes it.
    pub providers: Vec<String>,
}

/// What the policy decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    /// Whether the steps passed are enough.
    pub complete: bool,
    /// What may be presented next, in the policy's order. A method of the policy's the portal has no use for is left out.
    pub offers: Vec<Offered>,
    /// The rule that decided, for a log.
    pub rule: Option<String>,
}

impl Decision {
    /// What the policy offers for `method`.
    #[must_use]
    pub fn offer(&self, method: Method) -> Option<&Offered> {
        self.offers.iter().find(|offer| offer.method == method)
    }

    /// The offered methods, in order.
    #[must_use]
    pub fn methods(&self) -> Vec<Method> {
        self.offers.iter().map(|offer| offer.method).collect()
    }

    /// The rule that decided as a log shows it.
    #[must_use]
    pub fn rule_name(&self) -> &str {
        self.rule.as_deref().unwrap_or("none")
    }
}

/// The configured policy, built.
pub struct Policy {
    provider: Arc<dyn SignInPolicy>,
    /// What the log says is in force.
    summary: String,
    /// Dropped after the provider, whose instance points into what it loaded.
    _registry: Arc<Registry>,
}

impl fmt::Debug for Policy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Policy").finish_non_exhaustive()
    }
}

impl Policy {
    /// The policy `settings.sign_in_policy` names; with none, the portal's `methods` each alone, from anywhere.
    ///
    /// # Errors
    ///
    /// A message for a provider that is not there, does not load or refuses its configuration.
    pub fn new(settings: &Settings, methods: &[Method]) -> Result<Self, String> {
        let mut registry = new_registry()?;
        let Some(entry) = &settings.sign_in_policy else {
            let rules = SignInRules::single_factor(methods.iter().map(|method| method.id()))
                .map_err(|error| format!("sign_in_policy: {error}"))?;
            return Ok(Self {
                provider: Arc::new(rules),
                summary: "none: every method from anywhere".to_owned(),
                _registry: Arc::new(registry),
            });
        };
        let built = |why: String| format!("sign_in_policy: {}: {why}", entry.provider);
        match (&entry.library, entry.provider == RULES) {
            (Some(path), false) => {
                if !settings.libraries_any_owner {
                    check_library(path).map_err(built)?;
                }
                open_library(&mut registry, path).map_err(built)?;
            }
            (Some(_), true) => {
                return Err(built("it is built in; leave its `library` out".to_owned()));
            }
            (None, false) => {
                return Err(built(
                    "it is not built in; name the `library` that provides it".to_owned(),
                ));
            }
            (None, true) => {}
        }
        let config = value_of(&entry.config).map_err(built)?;
        let provider: Arc<dyn SignInPolicy> = Arc::new(
            registry
                .offer::<dyn SignInPolicy>(&entry.provider)
                .ok_or_else(|| built("no provider has that id".to_owned()))?
                .map_err(|_| built("it is not a sign-in policy".to_owned()))?
                .instantiate(&config)
                .map_err(|error| built(error.to_string()))?,
        );
        let summary = if entry.provider == RULES {
            let rules = entry
                .config
                .get("rule")
                .and_then(serde_json::Value::as_array)
                .map_or(0, Vec::len);
            format!("{rules} rule{}", if rules == 1 { "" } else { "s" })
        } else {
            format!("provider {}", entry.provider)
        };
        Ok(Self {
            provider,
            summary,
            _registry: Arc::new(registry),
        })
    }

    /// What is in force, for the log: how many rules, the provider's id, or that there is none.
    #[must_use]
    pub fn describe(&self) -> &str {
        &self.summary
    }

    /// A policy that is `provider`, for a test that needs one the rules provider cannot be.
    #[cfg(test)]
    pub fn from_provider(provider: Arc<dyn SignInPolicy>) -> Result<Self, String> {
        Ok(Self {
            provider,
            summary: "a provider of the test's own".to_owned(),
            _registry: Arc::new(new_registry()?),
        })
    }

    /// Asks the policy about `context`.
    ///
    /// # Errors
    ///
    /// The provider's reason, for a log: the caller offers nothing and refuses.
    pub fn decide(&self, context: SignInContext) -> Result<Decision, String> {
        let SignInDecision {
            complete,
            methods,
            rule,
            ..
        } = self
            .provider
            .decide(context)
            .map_err(|error| error.to_string())?;
        Ok(Decision {
            complete,
            offers: methods
                .into_iter()
                .filter_map(|offer| {
                    Some(Offered {
                        method: Method::from_id(&offer.method)?,
                        providers: offer.providers,
                    })
                })
                .collect(),
            rule,
        })
    }
}

/// The context of a sign-in from `address` (`None` when the proxy did not say), for `purpose`, with `person` once a step named
/// someone and the steps `done`.
#[must_use]
pub fn context(
    purpose: &str,
    address: Option<IpAddr>,
    person: Option<Person>,
    done: Vec<Factor>,
) -> SignInContext {
    SignInContext {
        purpose: purpose.to_owned(),
        done,
        address: address.map(|address| address.to_string()),
        client: Some(clients::BROWSER.to_owned()),
        service: Some(SERVICE.to_owned()),
        time_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|since| i64::try_from(since.as_secs()).ok()),
        person,
        ..SignInContext::default()
    }
}

/// The page's offer: nothing has been passed yet.
#[must_use]
pub fn offer_context(address: Option<IpAddr>) -> SignInContext {
    context(purposes::SIGN_IN, address, None, Vec::new())
}

/// The person an account is to the policy: its user id as text, its name and groups.
#[must_use]
pub fn person(user: &str, uid: u32, groups: &[String]) -> Person {
    Person {
        id: uid.to_string(),
        name: user.to_owned(),
        groups: groups.to_vec(),
        ..Person::default()
    }
}

/// The step a method passed, accepted by `provider`, that named the account `uid`.
#[must_use]
pub fn factor(method: Method, provider: &str, uid: u32) -> Factor {
    Factor {
        method: method.id().to_owned(),
        provider: Some(provider.to_owned()),
        person: Some(uid.to_string()),
    }
}
