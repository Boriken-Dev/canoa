//! Which servers Autodiscover may send the account's credential to.
//!
//! An Autodiscover answer is the server's word about where the mailbox lives, so an address in it (a redirect or the EWS
//! address) is trusted with the credential only when the account itself names that server: the host of the Autodiscover
//! address the account gives, or the domain of its mailbox address, with the names beneath either. Anything else is not asked.

use core::net::IpAddr;

use url::{Host, Url};

use canoa_mail::mail_backend::MailError;

/// The names an account gives for its own servers.
#[derive(Clone, Debug)]
pub(super) struct Hosts {
    names: Vec<String>,
}

impl Hosts {
    /// The domain of `email` and the host of the Autodiscover address the account names, when it names one.
    pub(super) fn of(email: &str, autodiscover: Option<&str>) -> Self {
        let mut names = vec![normal(email.rsplit('@').next().unwrap_or(email))];
        if let Some(host) = autodiscover
            .and_then(|url| Url::parse(url).ok())
            .and_then(|url| url.host().map(|host| normal(&host.to_string())))
        {
            names.push(host);
        }
        Self { names }
    }

    /// Whether the credential may be sent to `url`.
    ///
    /// # Errors
    ///
    /// The address is not a web address, is plain `http` to a server other than this computer, or names a server the account
    /// does not.
    pub(super) fn admit(&self, url: &str) -> Result<(), MailError> {
        let parsed = Url::parse(url).map_err(|_| {
            MailError::Backend("Autodiscover named an address that is not a web address".to_owned())
        })?;
        let host = parsed.host().ok_or_else(|| {
            MailError::Backend("Autodiscover named an address with no server".to_owned())
        })?;
        let shown = host.to_string();
        if !self.names_server(&host) {
            return Err(MailError::Backend(format!(
                "Autodiscover pointed at {shown}, which is neither the mailbox address's domain nor the Autodiscover address the account names, so the account's sign-in was not sent there"
            )));
        }
        match parsed.scheme() {
            "https" => Ok(()),
            "http" if loopback(&host) => Ok(()),
            _ => Err(MailError::Backend(format!(
                "Autodiscover pointed at {shown} without https, so the account's sign-in was not sent there"
            ))),
        }
    }

    fn names_server(&self, host: &Host<&str>) -> bool {
        let shown = normal(&host.to_string());
        let named = |name: &String| {
            shown == *name
                || (matches!(host, Host::Domain(_))
                    && shown
                        .strip_suffix(name.as_str())
                        .is_some_and(|rest| rest.ends_with('.')))
        };
        self.names.iter().any(named)
    }
}

/// A name as compared: lower case, without the root dot.
fn normal(name: &str) -> String {
    name.trim_end_matches('.').to_ascii_lowercase()
}

/// This computer's own address, the only place plain `http` is accepted.
fn loopback(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(name) => {
            let name = normal(name);
            name == "localhost" || name.ends_with(".localhost")
        }
        Host::Ipv4(ip) => IpAddr::V4(*ip).is_loopback(),
        Host::Ipv6(ip) => IpAddr::V6(*ip).is_loopback(),
    }
}

#[cfg(test)]
mod tests {
    use super::Hosts;

    fn admitted(hosts: &Hosts, url: &str) -> bool {
        hosts.admit(url).is_ok()
    }

    #[test]
    fn the_mailbox_domain_and_the_names_beneath_it_are_named() {
        let hosts = Hosts::of("me@Example.test", None);
        assert!(admitted(&hosts, "https://example.test/EWS/Exchange.asmx"));
        assert!(admitted(
            &hosts,
            "https://mail.example.test/EWS/Exchange.asmx"
        ));
        assert!(admitted(
            &hosts,
            "https://MAIL.example.test./EWS/Exchange.asmx"
        ));
    }

    #[test]
    fn another_domain_is_not_named_however_it_is_written() {
        let hosts = Hosts::of("me@example.test", None);
        for url in [
            "https://other.test/EWS/Exchange.asmx",
            "https://notexample.test/",
            "https://example.test.other.test/",
            "https://example.test@other.test/",
            "https://other.test/example.test",
            "https://other.test/?example.test",
            "https://10.1.2.3/",
            "https://[::2]/",
        ] {
            assert!(!admitted(&hosts, url), "{url}");
        }
    }

    #[test]
    fn plain_http_is_for_this_computer_only() {
        let named = Hosts::of("me@example.test", None);
        assert!(!admitted(
            &named,
            "http://mail.example.test/EWS/Exchange.asmx"
        ));
        assert!(!admitted(&named, "ftp://mail.example.test/"));
        assert!(!admitted(&named, "mail.example.test"));
        let here = Hosts::of(
            "me@example.test",
            Some("http://127.0.0.1:8080/autodiscover/autodiscover.xml"),
        );
        assert!(admitted(&here, "http://127.0.0.1:9090/EWS/Exchange.asmx"));
        assert!(!admitted(&here, "http://127.0.0.2:9090/"));
        let local = Hosts::of("me@localhost", None);
        assert!(admitted(&local, "http://localhost:8080/"));
        assert!(admitted(&local, "http://app.localhost/"));
    }

    #[test]
    fn the_autodiscover_address_the_account_gives_is_named() {
        let hosts = Hosts::of(
            "me@example.test",
            Some("https://exchange.hosting.test/autodiscover/autodiscover.xml"),
        );
        assert!(admitted(
            &hosts,
            "https://exchange.hosting.test/EWS/Exchange.asmx"
        ));
        assert!(admitted(&hosts, "https://east.exchange.hosting.test/"));
        assert!(!admitted(&hosts, "https://hosting.test/"));
        assert!(!admitted(&hosts, "https://other.hosting.test/"));
    }
}
