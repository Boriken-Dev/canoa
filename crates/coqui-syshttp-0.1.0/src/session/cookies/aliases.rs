//! Hosts a user declares to be one for cookies.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::{is_ip, normal_host};
use crate::session::error::Error;

/// Hosts that share cookies: each host of a group maps to the group's first host, and a cookie a
/// member sets belongs to the first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostAliases {
    map: BTreeMap<String, String>,
}

fn is_host_name(host: &str) -> bool {
    !host.is_empty()
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || byte == b'_'
                        || byte == b'-'
                })
        })
}

impl HostAliases {
    /// Each host of `groups` mapped to its group's first host. A group of one host is no alias.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidRequest`] for a host that is not a name or an IP address (a port, a path, a
    /// blank), and for one that appears in two groups.
    pub fn new<G, H>(groups: impl IntoIterator<Item = G>) -> Result<Self, Error>
    where
        G: IntoIterator<Item = H>,
        H: AsRef<str>,
    {
        let mut map: BTreeMap<String, String> = BTreeMap::new();
        for group in groups {
            let mut hosts: Vec<String> = Vec::new();
            for raw in group {
                let host = normal_host(raw.as_ref());
                if !(is_ip(&host) || is_host_name(&host)) {
                    return Err(Error::InvalidRequest(
                        "a cookie host group holds something that is not a host name or an IP address"
                            .to_string(),
                    ));
                }
                if map.contains_key(&host) {
                    return Err(Error::InvalidRequest(
                        "a host is in two cookie host groups".to_string(),
                    ));
                }
                if !hosts.contains(&host) {
                    hosts.push(host);
                }
            }
            if hosts.len() > 1 {
                for host in &hosts {
                    map.insert(host.clone(), hosts[0].clone());
                }
            }
        }
        Ok(Self { map })
    }

    /// Reads the text of `SYSHTTP_COOKIE_HOSTS`: hosts split by `,`, groups by `;`.
    ///
    /// # Errors
    ///
    /// As [`HostAliases::new`].
    pub fn parse(text: &str) -> Result<Self, Error> {
        Self::new(
            text.split(';')
                .map(|group| group.split(',').filter(|host| !host.trim().is_empty())),
        )
    }

    /// Whether no host is an alias.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// The first host of the group `host` belongs to, `None` when it is in no group.
    #[must_use]
    pub fn first(&self, host: &str) -> Option<&str> {
        self.map.get(&normal_host(host)).map(String::as_str)
    }

    /// Every host of the group whose first host is `first`.
    #[must_use]
    pub fn members(&self, first: &str) -> Vec<&str> {
        self.map
            .iter()
            .filter(|(_, target)| target.as_str() == first)
            .map(|(host, _)| host.as_str())
            .collect()
    }
}
