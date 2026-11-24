//! The PAC builtins that need no JavaScript engine: network arithmetic and string tests.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr};
use core::str::FromStr;

use coqui_net_acl::IpNet;

/// `isInNet(host, pattern, mask)` for an already resolved `address`: the IPv4 network
/// `pattern/mask`, mask written as a netmask. An IPv6 address is never in an IPv4 network.
pub(crate) fn is_in_net(address: IpAddr, pattern: &str, mask: &str) -> bool {
    let (Ok(pattern), Ok(mask)) = (
        Ipv4Addr::from_str(pattern.trim()),
        Ipv4Addr::from_str(mask.trim()),
    ) else {
        return false;
    };
    let IpAddr::V4(address) = address else {
        return false;
    };
    let mask = u32::from(mask);
    (u32::from(address) & mask) == (u32::from(pattern) & mask)
}

/// `isInNetEx(address, prefix)`: a CIDR network, IPv4 or IPv6.
pub(crate) fn is_in_net_ex(address: &str, prefix: &str) -> bool {
    match (IpAddr::from_str(address.trim()), IpNet::from_str(prefix)) {
        (Ok(address), Ok(network)) => network.contains(address),
        _ => false,
    }
}

/// `sortIpAddressList`: a `;`-separated address list in numeric order, IPv4 first.
pub(crate) fn sort_ip_address_list(list: &str) -> String {
    let mut addresses: Vec<&str> = list
        .split(';')
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .collect();
    if addresses.iter().all(|a| IpAddr::from_str(a).is_ok()) {
        addresses.sort_by_key(|a| IpAddr::from_str(a).ok());
    } else {
        addresses.sort_unstable();
    }
    addresses.join("; ")
}

/// `convert_addr`: an address as a number (IPv4 as 32 bits, IPv6 as its low 53 bits' float).
pub(crate) fn convert_addr(address: &str) -> Option<f64> {
    match IpAddr::from_str(address.trim()).ok()? {
        IpAddr::V4(v4) => Some(f64::from(u32::from(v4))),
        IpAddr::V6(v6) => Some(u128::from(v6) as f64),
    }
}

/// `localHostOrDomainIs(host, hostdom)`: the bare host name, or the full host and domain.
pub(crate) fn local_host_or_domain_is(host: &str, hostdom: &str) -> bool {
    if host.contains('.') {
        hostdom == host
    } else {
        hostdom.split('.').next() == Some(host)
    }
}

/// `dnsDomainLevels`: the number of dots, as the PAC specification defines it.
pub(crate) fn dns_domain_levels(host: &str) -> usize {
    host.matches('.').count()
}

/// The name of this PAC engine's version, for `getClientVersion`.
pub(crate) fn client_version() -> String {
    "1.0".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_in_net_compares_under_the_mask() {
        let address: IpAddr = "10.1.2.3".parse().unwrap();
        assert!(is_in_net(address, "10.0.0.0", "255.0.0.0"));
        assert!(!is_in_net(address, "10.2.0.0", "255.255.0.0"));
        assert!(!is_in_net("::1".parse().unwrap(), "10.0.0.0", "255.0.0.0"));
        assert!(!is_in_net(address, "garbage", "255.0.0.0"));
    }

    #[test]
    fn is_in_net_ex_takes_cidr_for_both_families() {
        assert!(is_in_net_ex("10.1.2.3", "10.0.0.0/8"));
        assert!(is_in_net_ex("2001:db8::5", "2001:db8::/32"));
        assert!(!is_in_net_ex("11.0.0.1", "10.0.0.0/8"));
        assert!(!is_in_net_ex("nonsense", "10.0.0.0/8"));
    }

    #[test]
    fn an_address_list_sorts_numerically_not_as_text() {
        assert_eq!(
            sort_ip_address_list("10.0.0.9; 9.0.0.1; 10.0.0.10"),
            "9.0.0.1; 10.0.0.9; 10.0.0.10"
        );
    }

    #[test]
    fn local_host_or_domain_is_compares_the_host_part_exactly() {
        assert!(local_host_or_domain_is("www", "www.example.com"));
        assert!(!local_host_or_domain_is("ww", "www.example.com"));
        assert!(local_host_or_domain_is(
            "www.example.com",
            "www.example.com"
        ));
        assert!(!local_host_or_domain_is(
            "www.example.org",
            "www.example.com"
        ));
    }

    #[test]
    fn domain_levels_count_dots() {
        assert_eq!(dns_domain_levels("sub.example.com"), 2);
        assert_eq!(dns_domain_levels("plain"), 0);
    }
}
