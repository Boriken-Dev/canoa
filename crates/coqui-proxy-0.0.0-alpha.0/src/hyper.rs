//! The asynchronous connector as a `tower` service for hyper's client.
//!
//! [`Connector`] answers a request's URI with a tunnel to its server over the first route of a
//! [`Resolver`] that connects: straight, through a `CONNECT` tunnel or through a
//! SOCKS proxy. It leaves TLS to the origin to the caller (a layer above it, as `hyper-util`'s
//! client expects), and an `http:` origin is tunnelled with `CONNECT` too, so the client sends the
//! request in origin form.

use alloc::boxed::Box;
use alloc::string::ToString;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use ::hyper::rt::{Read, ReadBufCursor, Write};
use hyper_util::client::legacy::connect::{Connected, Connection as HyperConnection};
use hyper_util::rt::TokioIo;

use crate::connect::asynch::{self, ProxyConnector};
use crate::error::Error;
use crate::resolver::Resolver;

/// A tunnel made by a [`Connector`], as hyper reads and writes it.
#[derive(Debug)]
pub struct Tunnel {
    io: TokioIo<asynch::Connection>,
}

impl Tunnel {
    /// The connection underneath.
    #[must_use]
    pub fn into_inner(self) -> asynch::Connection {
        self.io.into_inner()
    }
}

impl Read for Tunnel {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: ReadBufCursor<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_read(context, buffer)
    }
}

impl Write for Tunnel {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.io).poll_write(context, buffer)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(context)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(context)
    }
}

impl HyperConnection for Tunnel {
    fn connected(&self) -> Connected {
        Connected::new()
    }
}

/// Connects hyper's client through the routes of a [`Resolver`].
#[derive(Debug, Clone)]
pub struct Connector {
    connector: ProxyConnector,
    resolver: Resolver,
}

impl Connector {
    /// A connector asking `resolver` for each URI's routes and connecting with `connector`.
    #[must_use]
    pub fn new(connector: ProxyConnector, resolver: Resolver) -> Self {
        Self {
            connector,
            resolver,
        }
    }

    /// The resolver that decides the routes.
    #[must_use]
    pub fn resolver(&self) -> &Resolver {
        &self.resolver
    }

    /// The connector that makes the connections.
    #[must_use]
    pub fn connector(&self) -> &ProxyConnector {
        &self.connector
    }
}

impl tower_service::Service<::http::Uri> for Connector {
    type Response = Tunnel;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Tunnel, Error>> + Send>>;

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: ::http::Uri) -> Self::Future {
        let connector = self.connector.clone();
        let resolver = self.resolver.clone();
        Box::pin(async move {
            let connection = connector.connect_url(&resolver, &uri.to_string()).await?;
            Ok(Tunnel {
                io: TokioIo::new(connection),
            })
        })
    }
}
