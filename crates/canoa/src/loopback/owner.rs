//! The user that owns the client end of one loopback connection, asked of the kernel's socket diagnostics for that connection
//! alone: its cost does not grow with the sockets the machine holds, which reading `/proc/net/tcp` does.

use std::net::SocketAddrV4;

use rustix::net::netlink::{SOCK_DIAG, SocketAddrNetlink};
use rustix::net::{
    AddressFamily, RecvFlags, SendFlags, SocketFlags, SocketType, recv, sendto, socket_with,
};

/// `SOCK_DIAG_BY_FAMILY`: one socket, found by its two ends.
const BY_FAMILY: u16 = 20;
/// `NLM_F_REQUEST`.
const REQUEST: u16 = 1;
/// `NLMSG_ERROR`.
const FAILED: u16 = 2;
/// `ENOENT`: no socket with these ends.
const NOT_FOUND: i32 = 2;
/// `INET_DIAG_NOCOOKIE`: any socket with these ends.
const NO_COOKIE: u32 = u32::MAX;
/// The size of the request and of a socket's answer, after the 16 bytes of netlink header.
const REQUEST_LEN: usize = 72;
const REQUEST_SIZE: u32 = 72;
const HEADER: usize = 16;

/// What the kernel said of a connection.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Owner {
    /// The user of a live connection's socket at the client's end.
    User(u32),
    /// No such connection, or one that is closed: it reads user 0 and would pass for root.
    Gone,
    /// The kernel cannot be asked this way here.
    Unknown,
}

/// `addr` as the kernel takes it: the four bytes of an IPv4 address, or its IPv4-mapped form in IPv6's sixteen.
fn address(addr: &SocketAddrV4, mapped: bool) -> [u8; 16] {
    let mut out = [0_u8; 16];
    if mapped {
        out[10] = 0xff;
        out[11] = 0xff;
        out[12..].copy_from_slice(&addr.ip().octets());
    } else {
        out[..4].copy_from_slice(&addr.ip().octets());
    }
    out
}

/// The request for the TCP socket whose local end is `peer` and remote end `local`: the connecting side. `mapped` asks in
/// the IPv6 table, where an IPv4 peer of a dual-stack socket is kept.
fn request(peer: &SocketAddrV4, local: &SocketAddrV4, mapped: bool) -> [u8; REQUEST_LEN] {
    let mut out = [0_u8; REQUEST_LEN];
    out[0..4].copy_from_slice(&REQUEST_SIZE.to_ne_bytes());
    out[4..6].copy_from_slice(&BY_FAMILY.to_ne_bytes());
    out[6..8].copy_from_slice(&REQUEST.to_ne_bytes());
    out[8..12].copy_from_slice(&1_u32.to_ne_bytes());
    // inet_diag_req_v2: family, protocol, no extensions, any state.
    out[16] = if mapped { 10 } else { 2 };
    out[17] = 6;
    out[20..24].copy_from_slice(&u32::MAX.to_ne_bytes());
    // inet_diag_sockid: ports in network order, then both addresses, the interface and the cookie.
    out[24..26].copy_from_slice(&peer.port().to_be_bytes());
    out[26..28].copy_from_slice(&local.port().to_be_bytes());
    out[28..44].copy_from_slice(&address(peer, mapped));
    out[44..60].copy_from_slice(&address(local, mapped));
    out[64..68].copy_from_slice(&NO_COOKIE.to_ne_bytes());
    out[68..72].copy_from_slice(&NO_COOKIE.to_ne_bytes());
    out
}

/// What the answer to a request says. An established connection or one its client has begun to close is a live client's; a
/// socket with no inode is a closed connection's remnant.
fn read(answer: &[u8]) -> Owner {
    let field = |at: usize| {
        answer
            .get(at..at + 4)
            .and_then(|b| <[u8; 4]>::try_from(b).ok())
    };
    let Some(kind) = answer.get(4..6).and_then(|b| <[u8; 2]>::try_from(b).ok()) else {
        return Owner::Unknown;
    };
    match u16::from_ne_bytes(kind) {
        FAILED => match field(HEADER).map(i32::from_ne_bytes) {
            Some(code) if code == -NOT_FOUND => Owner::Gone,
            _ => Owner::Unknown,
        },
        BY_FAMILY => {
            // inet_diag_msg: state at 1, then the 48-byte id, expiry and two queues, the user and the inode.
            let state = answer.get(HEADER + 1).copied();
            match (field(HEADER + 64), field(HEADER + 68)) {
                (Some(user), Some(inode))
                    if matches!(state, Some(1 | 4 | 5)) && u32::from_ne_bytes(inode) != 0 =>
                {
                    Owner::User(u32::from_ne_bytes(user))
                }
                (Some(_), Some(_)) => Owner::Gone,
                _ => Owner::Unknown,
            }
        }
        _ => Owner::Unknown,
    }
}

/// Asks the kernel one question; `mapped` as in [`request`].
fn ask(peer: &SocketAddrV4, local: &SocketAddrV4, mapped: bool) -> Owner {
    let Ok(socket) = socket_with(
        AddressFamily::NETLINK,
        SocketType::RAW,
        SocketFlags::CLOEXEC,
        Some(SOCK_DIAG),
    ) else {
        return Owner::Unknown;
    };
    let sent = sendto(
        &socket,
        &request(peer, local, mapped),
        SendFlags::empty(),
        &SocketAddrNetlink::new(0, 0),
    );
    if sent.is_err() {
        return Owner::Unknown;
    }
    let mut answer = [0_u8; 256];
    match recv(&socket, &mut answer[..], RecvFlags::empty()) {
        Ok((_, len)) if len <= answer.len() => read(&answer[..len]),
        _ => Owner::Unknown,
    }
}

/// The user that owns the socket whose local end is `peer` and remote end `local`, in either of the kernel's TCP tables.
pub(super) fn owner(peer: &SocketAddrV4, local: &SocketAddrV4) -> Owner {
    match ask(peer, local, false) {
        Owner::Gone => ask(peer, local, true),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};

    fn v4(addr: SocketAddr) -> SocketAddrV4 {
        match addr {
            SocketAddr::V4(addr) => addr,
            SocketAddr::V6(addr) => panic!("{addr}"),
        }
    }

    #[test]
    fn a_connection_is_owned_by_the_user_that_made_it() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let local = v4(listener.local_addr().unwrap());
        let client = TcpStream::connect(local).unwrap();
        let (_server, peer) = listener.accept().unwrap();
        assert_eq!(v4(peer), v4(client.local_addr().unwrap()));
        assert_eq!(
            owner(&v4(peer), &local),
            Owner::User(rustix::process::geteuid().as_raw())
        );
    }

    #[test]
    fn a_dual_stack_client_is_found_in_the_ipv6_table() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let local = v4(listener.local_addr().unwrap());
        let mapped = Ipv4Addr::LOCALHOST.to_ipv6_mapped();
        let _client = TcpStream::connect((mapped, local.port())).unwrap();
        let (_server, peer) = listener.accept().unwrap();
        assert_eq!(
            owner(&v4(peer), &local),
            Owner::User(rustix::process::geteuid().as_raw())
        );
    }

    #[test]
    fn a_connection_that_is_not_there_has_no_owner() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let local = v4(listener.local_addr().unwrap());
        let nowhere = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 9);
        assert_eq!(owner(&nowhere, &local), Owner::Gone);
    }

    #[test]
    fn an_answer_that_is_short_or_of_another_kind_is_unknown() {
        assert_eq!(read(&[]), Owner::Unknown);
        assert_eq!(read(&[0; 12]), Owner::Unknown);
        let mut other = [0_u8; 100];
        other[4..6].copy_from_slice(&7_u16.to_ne_bytes());
        assert_eq!(read(&other), Owner::Unknown);
    }
}
