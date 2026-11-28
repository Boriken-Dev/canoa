//! `coqui-transport-pipe` as a provider of the `transport` kind, id `pipe`: a Windows named pipe or a Unix
//! domain socket, with the operating system's access control as the transport has it.
//!
//! Endpoints are `pipe:NAME`, `unix:PATH` (Unix) or a whole path, as [`PipeName`] reads them; text never
//! names another host. A connection is a byte stream, framed as a four-byte big-endian length and the message.
//! Both ends have to be this transport.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use core::time::Duration;

use coqui_builder::{Describe, Partial};
use coqui_transport::frame::LengthPrefix;
use coqui_transport::{Channel, StreamChannelBuilder, StreamChannelSettings};
use coqui_transport_pipe::{ConnectSettings, Error, Listener, ListenerSettings, PipeName, Stream};

use crate::driver::Accepted;
use crate::guatiao;
use crate::{Acceptor, Driver, DriverError, Refusal, TransportInfo};

/// The settings of the pipe transport: one group for each thing it does.
#[derive(Clone, Debug, PartialEq, Eq, Describe)]
#[map(deny_unknown_fields)]
pub struct PipeSettings {
    /// Making a connection, and the timeouts of the stream it makes.
    #[map(default = ConnectSettings::new())]
    pub connect: ConnectSettings,
    /// Listening: who may connect, and the timeouts of every stream it accepts.
    #[map(default = ListenerSettings::new())]
    pub listen: ListenerSettings,
    /// The channel on each stream: the largest message and the queues.
    #[map(default = StreamChannelSettings::new())]
    pub channel: StreamChannelSettings,
}

impl Default for PipeSettings {
    fn default() -> Self {
        Self {
            connect: ConnectSettings::new(),
            listen: ListenerSettings::new(),
            channel: StreamChannelSettings::new(),
        }
    }
}

/// The pipe transport as a [`Driver`].
#[derive(Debug, Clone, Copy)]
pub struct PipeDriver;

fn name(endpoint: &str) -> Result<PipeName, DriverError> {
    endpoint.parse().map_err(|error: Error| refusal(&error))
}

fn refusal(error: &Error) -> DriverError {
    let kind = match error {
        Error::InvalidName
        | Error::PathTooLong { .. }
        | Error::InvalidOptions
        | Error::NoRuntimeDirectory => Refusal::Endpoint,
        Error::Unsupported => Refusal::Unsupported,
        Error::NameTaken
        | Error::NotListening
        | Error::AccessDenied
        | Error::TimedOut
        | Error::GroupDenied
        | Error::SharedDirectory
        | Error::ForeignDirectory => Refusal::Unreachable,
        _ => Refusal::Failed,
    };
    // The error's own words name no address and hold no credential.
    DriverError::new(kind, error.to_string())
}

fn channel(
    stream: Stream,
    settings: StreamChannelSettings,
) -> Result<Box<dyn Channel>, DriverError> {
    StreamChannelBuilder::new()
        .codec(LengthPrefix::u32_be())
        .stream(stream)
        .fill(Partial::from(settings))
        .build()
        .map(|channel| Box::new(channel) as Box<dyn Channel>)
        .map_err(|error| {
            DriverError::new(
                Refusal::Failed,
                format!("the channel did not start: {error}"),
            )
        })
}

impl Driver for PipeDriver {
    type Settings = PipeSettings;

    fn describe(settings: &PipeSettings) -> TransportInfo {
        TransportInfo::new(
            "Pipe",
            "pipe:NAME, or unix:PATH on Unix: a named pipe or a Unix domain socket on this machine",
        )
        .max_message_bytes(settings.channel.max_message as u64)
    }

    fn dial(
        settings: &PipeSettings,
        endpoint: &str,
        timeout: Duration,
    ) -> Result<Box<dyn Channel>, DriverError> {
        let stream = Stream::builder()
            .name(name(endpoint)?)
            .fill(Partial::from(settings.connect.clone()))
            .timeout(timeout)
            .build()
            .map_err(|error| match error {
                coqui_transport_pipe::BuildError::Failed(error) => refusal(&error),
                other => DriverError::new(Refusal::Failed, other.to_string()),
            })?;
        channel(stream, settings.channel)
    }

    fn listen(settings: &PipeSettings, endpoint: &str) -> Result<Box<dyn Acceptor>, DriverError> {
        let listener = Listener::builder()
            .name(name(endpoint)?)
            .fill(Partial::from(settings.listen.clone()))
            .build()
            .map_err(|error| match error {
                coqui_transport_pipe::BuildError::Failed(error) => refusal(&error),
                other => DriverError::new(Refusal::Failed, other.to_string()),
            })?;
        Ok(Box::new(Accepting {
            listener,
            channel: settings.channel,
        }))
    }
}

struct Accepting {
    listener: Listener,
    channel: StreamChannelSettings,
}

impl Acceptor for Accepting {
    fn address(&self) -> String {
        self.listener.name().to_string()
    }

    fn accept(&self, timeout: Duration) -> Result<Option<Accepted>, DriverError> {
        match self.listener.accept_timeout(Some(timeout)) {
            Ok(stream) => Ok(Some((channel(stream, self.channel)?, String::new()))),
            Err(Error::TimedOut) => Ok(None),
            Err(error) => Err(refusal(&error)),
        }
    }
}

crate::transport_provider! {
    /// `coqui-transport-pipe` as a provider of the `transport` kind, id `pipe`.
    pub struct PipeProvider for PipeDriver, id = "pipe";
}

guatiao::local_providers!(
    id = "coqui-transport-pipe",
    version = env!("CARGO_PKG_VERSION"),
    providers = [PipeProvider]
);

/// Offers the `pipe` provider to `registry`, as a library linked into this binary named `coqui-transport-pipe`.
/// A second call is skipped, not an error.
///
/// # Errors
///
/// When the registry cannot take the library.
pub fn register(
    registry: &mut guatiao::library::Registry,
) -> Result<(), guatiao::library::LoadError> {
    registry
        .register_local("coqui-transport-pipe", library)
        .map(drop)
}
