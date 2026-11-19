//! Name lookups with a time limit, for the PAC builtins and WPAD.

use alloc::string::String;
use alloc::vec::Vec;
use core::net::IpAddr;
use core::time::Duration;
use std::net::ToSocketAddrs;
use std::sync::mpsc;
use std::thread;

/// Resolves `host` to its first address within `timeout`; the lookup itself cannot be
/// interrupted, so a slow one is left to finish on its own thread.
pub(crate) fn resolve(host: &str, timeout: Duration) -> Option<IpAddr> {
    resolve_all(host, timeout).into_iter().next()
}

/// Every address of `host`, IPv4 first, within `timeout`.
pub(crate) fn resolve_all(host: &str, timeout: Duration) -> Vec<IpAddr> {
    if let Ok(address) = host.parse::<IpAddr>() {
        return alloc::vec![address];
    }
    let (sender, receiver) = mpsc::channel();
    let name = String::from(host);
    // The thread is detached on purpose: a stuck resolver must not hold the caller.
    drop(thread::spawn(move || {
        let found: Vec<IpAddr> = (name.as_str(), 0)
            .to_socket_addrs()
            .map(|addresses| addresses.map(|a| a.ip()).collect())
            .unwrap_or_default();
        let _ = sender.send(found);
    }));
    let mut found = receiver.recv_timeout(timeout).unwrap_or_default();
    found.sort_by_key(|address| address.is_ipv6());
    found.dedup();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_literal_resolves_without_a_lookup() {
        assert_eq!(
            resolve("127.0.0.1", Duration::from_millis(1)),
            Some("127.0.0.1".parse().unwrap())
        );
    }
}
