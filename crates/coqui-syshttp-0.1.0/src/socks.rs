//! The client side of SOCKS 4, 4a and 5 (RFC 1928, RFC 1929): a `CONNECT` to `host:port`.
//!
//! A host name is always sent to the proxy and resolved there (SOCKS 5's domain address, SOCKS
//! 4a), so a host that exists only behind the proxy needs no DNS where the client runs.

use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr};
use std::io::{Read, Write};

use crate::error::{Error, Reason};
use crate::proxy::{Credentials, ProxyScheme};

/// Asks the proxy on `stream` to connect to `host:port`, logging in with `credentials` where the
/// proxy asks for it. On `Ok` the stream is the tunnel.
///
/// # Errors
///
/// [`Error::Login`] when the proxy wants a login that was not given or refuses it,
/// [`Error::Refused`] for any other refusal or a reply that is not SOCKS, [`Error::Unsupported`]
/// for what the protocol cannot say (an IPv6 address in SOCKS 4), [`Error::Io`] for the stream.
pub(crate) fn connect(
    stream: &mut (impl Read + Write),
    scheme: ProxyScheme,
    credentials: Option<&Credentials>,
    host: &str,
    port: u16,
) -> Result<(), Error> {
    match scheme {
        ProxyScheme::Socks4 => socks4(stream, credentials, host, port),
        ProxyScheme::Socks5 => socks5(stream, credentials, host, port),
        _ => Err(Error::Unsupported(Reason::new("not a SOCKS proxy"))),
    }
}

fn address(host: &str) -> Option<IpAddr> {
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .split('%')
        .next()?
        .parse()
        .ok()
}

fn refused(text: &'static str) -> Error {
    Error::Refused(Reason::new(text))
}

fn socks4(
    stream: &mut (impl Read + Write),
    credentials: Option<&Credentials>,
    host: &str,
    port: u16,
) -> Result<(), Error> {
    let mut request = Vec::from([4u8, 1]);
    request.extend_from_slice(&port.to_be_bytes());
    let mut name = None;
    match address(host) {
        Some(IpAddr::V4(v4)) => request.extend_from_slice(&v4.octets()),
        Some(IpAddr::V6(_)) => {
            return Err(Error::Unsupported(Reason::new(
                "SOCKS 4 cannot name an IPv6 address",
            )));
        }
        None => {
            // SOCKS 4a: the address 0.0.0.x announces a name after the user id.
            request.extend_from_slice(&Ipv4Addr::new(0, 0, 0, 1).octets());
            name = Some(host);
        }
    }
    if let Some(credentials) = credentials {
        request.extend_from_slice(credentials.username().as_bytes());
    }
    request.push(0);
    if let Some(name) = name {
        request.extend_from_slice(name.as_bytes());
        request.push(0);
    }
    stream.write_all(&request)?;
    stream.flush()?;
    let mut reply = [0u8; 8];
    read_all(stream, &mut reply)?;
    if reply[0] != 0 {
        return Err(refused("the reply is not SOCKS 4"));
    }
    match reply[1] {
        0x5a => Ok(()),
        0x5b => Err(refused("the SOCKS 4 proxy refused or failed the request")),
        0x5c | 0x5d => Err(Error::Login(Reason::new(
            "the SOCKS 4 proxy could not verify the user id",
        ))),
        _ => Err(refused("the SOCKS 4 reply is unknown")),
    }
}

fn socks5(
    stream: &mut (impl Read + Write),
    credentials: Option<&Credentials>,
    host: &str,
    port: u16,
) -> Result<(), Error> {
    // No login, and username/password when we have one to give.
    let mut greeting = Vec::from([5u8]);
    if credentials.is_some() {
        greeting.extend_from_slice(&[2, 0x00, 0x02]);
    } else {
        greeting.extend_from_slice(&[1, 0x00]);
    }
    stream.write_all(&greeting)?;
    stream.flush()?;
    let mut choice = [0u8; 2];
    read_all(stream, &mut choice)?;
    if choice[0] != 5 {
        return Err(refused("the reply is not SOCKS 5"));
    }
    match choice[1] {
        0x00 => {}
        0x02 => {
            let credentials = credentials.ok_or_else(|| {
                Error::Login(Reason::new(
                    "the SOCKS 5 proxy asks for a login that is not configured",
                ))
            })?;
            let (user, password) = (
                credentials.username().as_bytes(),
                credentials.password().as_bytes(),
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
            stream.write_all(&login)?;
            stream.flush()?;
            let mut verdict = [0u8; 2];
            read_all(stream, &mut verdict)?;
            if verdict[1] != 0 {
                return Err(Error::Login(Reason::new(
                    "the SOCKS 5 proxy refused the credentials",
                )));
            }
        }
        0xff => {
            return Err(Error::Login(Reason::new(
                "the SOCKS 5 proxy accepts none of the logins offered",
            )));
        }
        _ => return Err(refused("the SOCKS 5 proxy chose a login this client lacks")),
    }

    let mut request = Vec::from([5u8, 1, 0]);
    match address(host) {
        Some(IpAddr::V4(v4)) => {
            request.push(1);
            request.extend_from_slice(&v4.octets());
        }
        Some(IpAddr::V6(v6)) => {
            request.push(4);
            request.extend_from_slice(&v6.octets());
        }
        None => {
            let Ok(length) = u8::try_from(host.len()) else {
                return Err(Error::Parse("host: it is over 255 bytes"));
            };
            request.push(3);
            request.push(length);
            request.extend_from_slice(host.as_bytes());
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request)?;
    stream.flush()?;

    let mut head = [0u8; 4];
    read_all(stream, &mut head)?;
    if head[0] != 5 {
        return Err(refused("the reply is not SOCKS 5"));
    }
    if head[1] != 0 {
        return Err(Error::Refused(Reason::new(match head[1] {
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
    // The address the proxy bound: read and dropped, so the stream starts at the tunnel.
    let bound = match head[3] {
        1 => 4,
        4 => 16,
        3 => {
            let mut length = [0u8; 1];
            read_all(stream, &mut length)?;
            usize::from(length[0])
        }
        _ => return Err(refused("the SOCKS 5 reply names an unknown address type")),
    };
    let mut skipped = alloc::vec![0u8; bound + 2];
    read_all(stream, &mut skipped)
}

fn read_all(stream: &mut impl Read, buffer: &mut [u8]) -> Result<(), Error> {
    stream.read_exact(buffer).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            refused("the proxy closed the connection")
        } else {
            Error::Io(error)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A stream that reads canned bytes and records what was written.
    struct Canned {
        read: Cursor<Vec<u8>>,
        written: Vec<u8>,
    }

    impl Canned {
        fn new(reply: &[u8]) -> Self {
            Self {
                read: Cursor::new(reply.to_vec()),
                written: Vec::new(),
            }
        }
    }

    impl Read for Canned {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.read.read(buffer)
        }
    }

    impl Write for Canned {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.written.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn socks5_sends_the_name_for_the_proxy_to_resolve() {
        let mut stream = Canned::new(&[5, 0, 5, 0, 0, 1, 1, 2, 3, 4, 0, 80]);
        connect(
            &mut stream,
            ProxyScheme::Socks5,
            None,
            "mail.corp.example",
            443,
        )
        .unwrap();
        let mut expected = Vec::from([5u8, 1, 0, 5, 1, 0, 3, 17]);
        expected.extend_from_slice(b"mail.corp.example");
        expected.extend_from_slice(&443u16.to_be_bytes());
        assert_eq!(stream.written, expected);
    }

    #[test]
    fn socks5_sends_an_address_as_an_address() {
        let mut stream = Canned::new(&[5, 0, 5, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
        connect(&mut stream, ProxyScheme::Socks5, None, "10.1.2.3", 25).unwrap();
        assert_eq!(&stream.written[3..], [5, 1, 0, 1, 10, 1, 2, 3, 0, 25]);
        let mut stream = Canned::new(&[
            5, 0, 5, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]);
        connect(&mut stream, ProxyScheme::Socks5, None, "[::1]", 25).unwrap();
        assert_eq!(stream.written[6], 4);
    }

    #[test]
    fn socks5_logs_in_with_username_and_password_when_it_is_asked_for() {
        let mut stream = Canned::new(&[5, 2, 1, 0, 5, 0, 0, 3, 1, b'x', 0, 0]);
        let credentials = Credentials::new("ada", "s3cret");
        connect(&mut stream, ProxyScheme::Socks5, Some(&credentials), "h", 1).unwrap();
        assert_eq!(
            &stream.written[..13],
            [5, 2, 0, 2, 1, 3, b'a', b'd', b'a', 6, b's', b'3', b'c']
        );
    }

    #[test]
    fn socks5_refusals_are_told_apart() {
        let wanted = Canned::new(&[5, 2]);
        let mut stream = wanted;
        let error = connect(&mut stream, ProxyScheme::Socks5, None, "h", 1).unwrap_err();
        assert!(matches!(error, Error::Login(_)), "{error}");
        let mut stream = Canned::new(&[5, 0xff]);
        let error = connect(&mut stream, ProxyScheme::Socks5, None, "h", 1).unwrap_err();
        assert!(matches!(error, Error::Login(_)), "{error}");
        let mut stream = Canned::new(&[5, 0, 5, 5, 0, 1]);
        let error = connect(&mut stream, ProxyScheme::Socks5, None, "h", 1).unwrap_err();
        assert!(matches!(error, Error::Refused(_)), "{error}");
        let mut stream = Canned::new(&[5, 0]);
        let error = connect(&mut stream, ProxyScheme::Socks5, None, "h", 1).unwrap_err();
        assert!(matches!(error, Error::Refused(_)), "{error}");
    }

    #[test]
    fn socks4_sends_an_address_and_socks4a_a_name() {
        let mut stream = Canned::new(&[0, 0x5a, 0, 0, 0, 0, 0, 0]);
        connect(&mut stream, ProxyScheme::Socks4, None, "10.1.2.3", 80).unwrap();
        assert_eq!(stream.written, [4, 1, 0, 80, 10, 1, 2, 3, 0]);
        let mut stream = Canned::new(&[0, 0x5a, 0, 0, 0, 0, 0, 0]);
        let credentials = Credentials::new("ada", "ignored");
        connect(
            &mut stream,
            ProxyScheme::Socks4,
            Some(&credentials),
            "h.example",
            80,
        )
        .unwrap();
        let mut expected = Vec::from([4u8, 1, 0, 80, 0, 0, 0, 1]);
        expected.extend_from_slice(b"ada\0h.example\0");
        assert_eq!(stream.written, expected);
    }

    #[test]
    fn socks4_cannot_name_ipv6_and_reports_a_refusal() {
        let mut stream = Canned::new(&[]);
        let error = connect(&mut stream, ProxyScheme::Socks4, None, "::1", 80).unwrap_err();
        assert!(matches!(error, Error::Unsupported(_)), "{error}");
        let mut stream = Canned::new(&[0, 0x5b, 0, 0, 0, 0, 0, 0]);
        let error = connect(&mut stream, ProxyScheme::Socks4, None, "h", 80).unwrap_err();
        assert!(matches!(error, Error::Refused(_)), "{error}");
    }
}
