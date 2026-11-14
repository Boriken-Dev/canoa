//! A browser's connection: a loopback TCP stream, or one on the Unix socket a proxy reaches the portal at.

use alloc::sync::Arc;
use core::net::IpAddr;
use core::time::Duration;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};

use coqui_transport_pipe::{ReadTimeout, Stream, WriteTimeout};

/// One connection, cloneable so that one thread can read while another writes.
#[derive(Clone, Debug)]
pub enum Conn {
    /// On the loopback TCP address, which any local user can reach: what it says of the browser's address is not believed.
    Tcp(Arc<TcpStream>),
    /// On the socket only the proxy's group can reach: the proxy says who the browser is.
    Pipe(Arc<Stream>),
}

impl Conn {
    /// Whether what this connection says about the browser's address and scheme is believed.
    pub fn from_proxy(&self) -> bool {
        matches!(self, Self::Pipe(_))
    }

    /// The address of the peer, for a TCP connection.
    pub fn peer_ip(&self) -> Option<IpAddr> {
        match self {
            Self::Tcp(stream) => stream.peer_addr().ok().map(|address| address.ip()),
            Self::Pipe(_) => None,
        }
    }

    /// Bounds a read.
    pub fn set_read_timeout(&self, wait: Option<Duration>) -> io::Result<()> {
        match self {
            Self::Tcp(stream) => stream.set_read_timeout(wait),
            Self::Pipe(stream) => {
                let stream = &**stream;
                stream.set_read_timeout(wait)
            }
        }
    }

    /// Bounds how long a write may make no progress.
    pub fn set_write_timeout(&self, wait: Option<Duration>) -> io::Result<()> {
        match self {
            Self::Tcp(stream) => stream.set_write_timeout(wait),
            Self::Pipe(stream) => {
                let stream = &**stream;
                stream.set_write_timeout(wait)
            }
        }
    }

    /// Ends both directions, which wakes whoever waits on either.
    pub fn shutdown(&self) {
        match self {
            Self::Tcp(stream) => {
                let _ = stream.shutdown(Shutdown::Both);
            }
            Self::Pipe(stream) => {
                let _ = stream.shutdown();
            }
        }
    }
}

impl Read for Conn {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(stream) => (&**stream).read(buffer),
            Self::Pipe(stream) => (&**stream).read(buffer),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(stream) => (&**stream).write(buffer),
            Self::Pipe(stream) => (&**stream).write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Tcp(stream) => (&**stream).flush(),
            Self::Pipe(stream) => (&**stream).flush(),
        }
    }
}
