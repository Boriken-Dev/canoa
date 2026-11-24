//! The frames of SOCKS 4, 4a and 5 (RFC 1928, RFC 1929), without I/O.
//!
//! A host name is always sent to the proxy and resolved there (SOCKS 5's domain address, SOCKS
//! 4a), so a host that exists only behind the proxy needs no DNS where the client runs.
//! Every `decode` takes the bytes read so far and says whether it has a value, needs more, or
//! the bytes are wrong.

use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr};

use super::core::Progress;
use crate::error::{Error, Reason};
use crate::proxy::Credentials;

fn refused(text: &'static str) -> Error {
    Error::Refused(Reason::new(text))
}

fn address(host: &str) -> Option<IpAddr> {
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .split('%')
        .next()?
        .parse()
        .ok()
}

/// How the proxy chose to log the client in (SOCKS 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Method {
    /// No login.
    None,
    /// RFC 1929 user name and password.
    Password,
}

/// The first message of SOCKS 5: the logins on offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Greeting {
    password: bool,
}

impl Greeting {
    /// A greeting that offers a user name and password as well as no login when `password`.
    #[must_use]
    pub fn new(password: bool) -> Self {
        Self { password }
    }

    /// The greeting as bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        if self.password {
            Vec::from([5, 2, 0x00, 0x02])
        } else {
            Vec::from([5, 1, 0x00])
        }
    }

    /// Reads the proxy's choice.
    ///
    /// # Errors
    ///
    /// [`Error::Refused`] for a reply that is not SOCKS 5 or chooses a login this client lacks;
    /// [`Error::Login`] when the proxy accepts none of the logins offered.
    pub fn decode_choice(bytes: &[u8]) -> Result<Progress<Method>, Error> {
        let [version, choice, ..] = bytes else {
            return Ok(Progress::Incomplete(2 - bytes.len()));
        };
        if *version != 5 {
            return Err(refused("the reply is not SOCKS 5"));
        }
        let value = match choice {
            0x00 => Method::None,
            0x02 => Method::Password,
            0xff => {
                return Err(Error::Login(Reason::new(
                    "the SOCKS 5 proxy accepts none of the logins offered",
                )));
            }
            _ => return Err(refused("the SOCKS 5 proxy chose a login this client lacks")),
        };
        Ok(Progress::Complete { value, consumed: 2 })
    }
}

/// The user name and password message of RFC 1929.
#[derive(Debug, Clone, Copy)]
pub struct PasswordLogin<'a> {
    credentials: &'a Credentials,
}

impl<'a> PasswordLogin<'a> {
    /// A login with `credentials`.
    #[must_use]
    pub fn new(credentials: &'a Credentials) -> Self {
        Self { credentials }
    }

    /// The message as bytes.
    ///
    /// # Errors
    ///
    /// [`Error::Login`] when the user name or the password is over 255 bytes.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let (user, password) = (
            self.credentials.username().as_bytes(),
            self.credentials.password().as_bytes(),
        );
        let (Ok(user_length), Ok(password_length)) =
            (u8::try_from(user.len()), u8::try_from(password.len()))
        else {
            return Err(Error::Login(Reason::new(
                "a SOCKS 5 user name or password is over 255 bytes",
            )));
        };
        let mut login = Vec::from([1u8, user_length]);
        login.extend_from_slice(user);
        login.push(password_length);
        login.extend_from_slice(password);
        Ok(login)
    }

    /// Reads the proxy's verdict.
    ///
    /// # Errors
    ///
    /// [`Error::Login`] when the proxy refused the credentials.
    pub fn decode_verdict(bytes: &[u8]) -> Result<Progress<()>, Error> {
        let [_, status, ..] = bytes else {
            return Ok(Progress::Incomplete(2 - bytes.len()));
        };
        if *status != 0 {
            return Err(Error::Login(Reason::new(
                "the SOCKS 5 proxy refused the credentials",
            )));
        }
        Ok(Progress::Complete {
            value: (),
            consumed: 2,
        })
    }
}

/// Which SOCKS version a [`Request`] speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Version {
    /// SOCKS 4, and 4a for a host name.
    V4,
    /// SOCKS 5.
    V5,
}

/// A `CONNECT` to `host:port`.
#[derive(Debug, Clone)]
pub struct Request<'a> {
    version: Version,
    host: &'a str,
    port: u16,
    user: Option<&'a str>,
}

impl<'a> Request<'a> {
    /// A request for a tunnel to `host:port`. `user` is SOCKS 4's user id (it has no passwords).
    #[must_use]
    pub fn new(version: Version, host: &'a str, port: u16, user: Option<&'a str>) -> Self {
        Self {
            version,
            host,
            port,
            user,
        }
    }

    /// The request as bytes.
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] for an IPv6 address in SOCKS 4; [`Error::Parse`] for a host name
    /// over 255 bytes in SOCKS 5.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        match self.version {
            Version::V4 => self.encode4(),
            Version::V5 => self.encode5(),
        }
    }

    fn encode4(&self) -> Result<Vec<u8>, Error> {
        let mut request = Vec::from([4u8, 1]);
        request.extend_from_slice(&self.port.to_be_bytes());
        let mut name = None;
        match address(self.host) {
            Some(IpAddr::V4(v4)) => request.extend_from_slice(&v4.octets()),
            Some(IpAddr::V6(_)) => {
                return Err(Error::Unsupported(Reason::new(
                    "SOCKS 4 cannot name an IPv6 address",
                )));
            }
            None => {
                // SOCKS 4a: the address 0.0.0.x announces a name after the user id.
                request.extend_from_slice(&Ipv4Addr::new(0, 0, 0, 1).octets());
                name = Some(self.host);
            }
        }
        if let Some(user) = self.user {
            request.extend_from_slice(user.as_bytes());
        }
        request.push(0);
        if let Some(name) = name {
            request.extend_from_slice(name.as_bytes());
            request.push(0);
        }
        Ok(request)
    }

    fn encode5(&self) -> Result<Vec<u8>, Error> {
        let mut request = Vec::from([5u8, 1, 0]);
        match address(self.host) {
            Some(IpAddr::V4(v4)) => {
                request.push(1);
                request.extend_from_slice(&v4.octets());
            }
            Some(IpAddr::V6(v6)) => {
                request.push(4);
                request.extend_from_slice(&v6.octets());
            }
            None => {
                let Ok(length) = u8::try_from(self.host.len()) else {
                    return Err(Error::Parse("host: it is over 255 bytes"));
                };
                request.push(3);
                request.push(length);
                request.extend_from_slice(self.host.as_bytes());
            }
        }
        request.extend_from_slice(&self.port.to_be_bytes());
        Ok(request)
    }
}

/// The proxy's answer to a [`Request`]; the address it bound is read and dropped, so the stream
/// starts at the tunnel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reply;

impl Reply {
    /// Reads a SOCKS 4 reply (8 bytes).
    ///
    /// # Errors
    ///
    /// [`Error::Refused`] for a refusal or a reply that is not SOCKS 4; [`Error::Login`] when the
    /// proxy could not verify the user id.
    pub fn decode4(bytes: &[u8]) -> Result<Progress<Self>, Error> {
        if bytes.len() < 8 {
            return Ok(Progress::Incomplete(8 - bytes.len()));
        }
        if bytes[0] != 0 {
            return Err(refused("the reply is not SOCKS 4"));
        }
        match bytes[1] {
            0x5a => Ok(Progress::Complete {
                value: Self,
                consumed: 8,
            }),
            0x5b => Err(refused("the SOCKS 4 proxy refused or failed the request")),
            0x5c | 0x5d => Err(Error::Login(Reason::new(
                "the SOCKS 4 proxy could not verify the user id",
            ))),
            _ => Err(refused("the SOCKS 4 reply is unknown")),
        }
    }

    /// Reads a SOCKS 5 reply (a 4-byte head, the bound address and the port).
    ///
    /// # Errors
    ///
    /// [`Error::Refused`] for a refusal or a reply that is not SOCKS 5.
    pub fn decode5(bytes: &[u8]) -> Result<Progress<Self>, Error> {
        if bytes.len() < 4 {
            return Ok(Progress::Incomplete(4 - bytes.len()));
        }
        if bytes[0] != 5 {
            return Err(refused("the reply is not SOCKS 5"));
        }
        if bytes[1] != 0 {
            return Err(Error::Refused(Reason::new(match bytes[1] {
                1 => "the SOCKS 5 proxy failed",
                2 => "the SOCKS 5 proxy's rules do not allow the connection",
                3 => "the SOCKS 5 proxy cannot reach the network",
                4 => "the SOCKS 5 proxy cannot reach the host",
                5 => "the host refused the connection",
                6 => "the SOCKS 5 request timed out",
                7 => "the SOCKS 5 proxy lacks the command",
                8 => "the SOCKS 5 proxy lacks the address type",
                _ => "the SOCKS 5 reply is unknown",
            })));
        }
        let bound = match bytes[3] {
            1 => 4,
            4 => 16,
            3 => match bytes.get(4) {
                Some(length) => 1 + usize::from(*length),
                None => return Ok(Progress::Incomplete(1)),
            },
            _ => return Err(refused("the SOCKS 5 reply names an unknown address type")),
        };
        let total = 4 + bound + 2;
        if bytes.len() < total {
            return Ok(Progress::Incomplete(total - bytes.len()));
        }
        Ok(Progress::Complete {
            value: Self,
            consumed: total,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_socks5_name_is_sent_for_the_proxy_to_resolve() {
        let bytes = Request::new(Version::V5, "mail.corp.example", 443, None)
            .encode()
            .unwrap();
        let mut expected = Vec::from([5u8, 1, 0, 3, 17]);
        expected.extend_from_slice(b"mail.corp.example");
        expected.extend_from_slice(&443u16.to_be_bytes());
        assert_eq!(bytes, expected);
    }

    #[test]
    fn replies_ask_for_the_bytes_they_lack() {
        assert_eq!(Reply::decode5(&[5, 0]).unwrap(), Progress::Incomplete(2));
        assert_eq!(
            Reply::decode5(&[5, 0, 0, 3, 2, b'a']).unwrap(),
            Progress::Incomplete(3)
        );
        assert_eq!(Reply::decode4(&[0, 0x5a]).unwrap(), Progress::Incomplete(6));
        assert_eq!(
            Greeting::decode_choice(&[5]).unwrap(),
            Progress::Incomplete(1)
        );
    }
}
