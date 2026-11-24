//! Downloads a PAC script: a plain `GET`, never through a proxy.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::net::SocketAddr;
use core::time::Duration;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Instant;

use crate::error::{Error, Reason};
use crate::settings::PacSettings;

fn pac_error(text: &'static str) -> Error {
    Error::Pac(Reason::new(text))
}

/// The body of `url` (`http` or, with the `tls` feature, `https`), within `download_timeout` for each step, following `max_redirects` redirects.
pub(crate) fn get(url: &str, settings: &PacSettings) -> Result<Vec<u8>, Error> {
    let mut url = url.to_string();
    for _ in 0..=settings.max_redirects {
        let (status, location, body) = get_once(&url, settings)?;
        match status {
            200 => return Ok(body),
            301 | 302 | 307 | 308 => {
                let location = location.ok_or_else(|| pac_error("redirect without a location"))?;
                url = absolute(&url, &location);
            }
            _ => return Err(pac_error("download refused")),
        }
    }
    Err(pac_error("too many redirects"))
}

fn absolute(base: &str, location: &str) -> String {
    if location.contains("://") {
        return location.to_string();
    }
    let scheme_end = base.find("://").map_or(0, |index| index + 3);
    let origin_end = base[scheme_end..]
        .find('/')
        .map_or(base.len(), |index| scheme_end + index);
    if location.starts_with('/') {
        alloc::format!("{}{location}", &base[..origin_end])
    } else {
        let directory_end = base
            .rfind('/')
            .filter(|index| *index >= origin_end)
            .unwrap_or(origin_end);
        alloc::format!("{}/{location}", &base[..directory_end])
    }
}

struct Location {
    tls: bool,
    host: String,
    port: u16,
    path: String,
}

fn split(url: &str) -> Result<Location, Error> {
    let (tls, rest) = if let Some(rest) = url.strip_prefix("http://") {
        (false, rest)
    } else if let Some(rest) = url.strip_prefix("https://") {
        (true, rest)
    } else {
        return Err(pac_error("unsupported location"));
    };
    let (authority, path) = rest
        .find('/')
        .map_or((rest, "/"), |slash| (&rest[..slash], &rest[slash..]));
    let authority: http::uri::Authority = authority.parse().map_err(|_| pac_error("location"))?;
    let host = authority.host();
    let host = host
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(host)
        .to_string();
    Ok(Location {
        tls,
        host,
        port: authority.port_u16().unwrap_or(if tls { 443 } else { 80 }),
        path: path.to_string(),
    })
}

fn connect(location: &Location, timeout: Duration) -> Result<TcpStream, Error> {
    let addresses: Vec<SocketAddr> = (location.host.as_str(), location.port)
        .to_socket_addrs()?
        .collect();
    let mut last = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => {
                stream.set_read_timeout(Some(timeout))?;
                stream.set_write_timeout(Some(timeout))?;
                return Ok(stream);
            }
            Err(error) => last = Some(error),
        }
    }
    Err(last.map_or_else(|| pac_error("no address"), Error::Io))
}

fn get_once(url: &str, settings: &PacSettings) -> Result<(u16, Option<String>, Vec<u8>), Error> {
    let location = split(url)?;
    let timeout = settings.download_timeout;
    let stream = connect(&location, timeout)?;
    let default_port = if location.tls { 443 } else { 80 };
    let host_header = if location.port == default_port {
        location.host.clone()
    } else {
        crate::proxy::join_host_port(&location.host, location.port)
    };
    let request = alloc::format!(
        "GET {} HTTP/1.1\r\nHost: {host_header}\r\nUser-Agent: coqui-proxy\r\nAccept: */*\r\nConnection: close\r\n\r\n",
        location.path,
    );
    let deadline = Instant::now() + timeout * 3;
    let raw = if location.tls {
        read_tls(
            stream,
            &location.host,
            request.as_bytes(),
            deadline,
            settings,
        )?
    } else {
        let mut stream = stream;
        stream.write_all(request.as_bytes())?;
        read_all(&mut stream, deadline, settings.max_script_bytes)?
    };
    parse_response(&raw, settings.max_script_bytes)
}

#[cfg(feature = "tls")]
fn read_tls(
    stream: TcpStream,
    host: &str,
    request: &[u8],
    deadline: Instant,
    settings: &PacSettings,
) -> Result<Vec<u8>, Error> {
    let config = match &settings.tls {
        Some(config) => alloc::sync::Arc::clone(config),
        None => crate::tls::platform_config()?,
    };
    let name =
        rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|_| pac_error("host"))?;
    let connection = rustls::ClientConnection::new(config, name).map_err(|_| pac_error("tls"))?;
    let mut tls = rustls::StreamOwned::new(connection, stream);
    tls.write_all(request)?;
    read_all(&mut tls, deadline, settings.max_script_bytes)
}

#[cfg(not(feature = "tls"))]
fn read_tls(
    _stream: TcpStream,
    _host: &str,
    _request: &[u8],
    _deadline: Instant,
    _settings: &PacSettings,
) -> Result<Vec<u8>, Error> {
    Err(pac_error("an https location needs the `tls` feature"))
}

fn read_all(reader: &mut impl Read, deadline: Instant, max: usize) -> Result<Vec<u8>, Error> {
    let mut data = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        if Instant::now() > deadline {
            return Err(pac_error("download timed out"));
        }
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => data.extend_from_slice(&chunk[..count]),
            // A server that closes TLS without close_notify still delivered what it sent.
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(Error::Io(error)),
        }
        if data.len() > max.saturating_add(16 * 1024) {
            return Err(pac_error("script too large"));
        }
    }
    Ok(data)
}

fn parse_response(raw: &[u8], max: usize) -> Result<(u16, Option<String>, Vec<u8>), Error> {
    let end = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| pac_error("malformed reply"))?;
    let head = String::from_utf8_lossy(&raw[..end]);
    let mut lines = head.lines();
    let status: u16 = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| pac_error("malformed reply"))?;
    let mut location = None;
    let mut chunked = false;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            let value = value.trim();
            if name.eq_ignore_ascii_case("location") {
                location = Some(value.to_string());
            } else if name.eq_ignore_ascii_case("transfer-encoding")
                && value.to_ascii_lowercase().contains("chunked")
            {
                chunked = true;
            }
        }
    }
    let body = &raw[end + 4..];
    let body = if chunked {
        dechunk(body)?
    } else {
        body.to_vec()
    };
    if body.len() > max {
        return Err(pac_error("script too large"));
    }
    Ok((status, location, body))
}

fn dechunk(mut body: &[u8]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    loop {
        let line_end = body
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| pac_error("malformed reply"))?;
        let size = core::str::from_utf8(&body[..line_end])
            .ok()
            .and_then(|line| line.split(';').next())
            .and_then(|size| usize::from_str_radix(size.trim(), 16).ok())
            .ok_or_else(|| pac_error("malformed reply"))?;
        body = &body[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if body.len() < size + 2 {
            return Err(pac_error("malformed reply"));
        }
        out.extend_from_slice(&body[..size]);
        body = &body[size + 2..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chunked_body_is_joined() {
        let body = b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n";
        assert_eq!(dechunk(body).unwrap(), b"Wikipedia");
    }

    #[test]
    fn a_relative_redirect_is_resolved_against_the_request() {
        assert_eq!(
            absolute("http://h:81/a/b.pac", "/c.pac"),
            "http://h:81/c.pac"
        );
        assert_eq!(absolute("http://h/a/b.pac", "c.pac"), "http://h/a/c.pac");
        assert_eq!(absolute("http://h/a", "https://x/y"), "https://x/y");
    }

    #[test]
    fn a_response_head_gives_the_status_location_and_body() {
        let raw = b"HTTP/1.1 302 Found\r\nLocation: /next\r\n\r\n";
        let (status, location, body) =
            parse_response(raw, PacSettings::DEFAULT_MAX_SCRIPT_BYTES).unwrap();
        assert_eq!(
            (status, location.as_deref(), body.len()),
            (302, Some("/next"), 0)
        );
    }
}
