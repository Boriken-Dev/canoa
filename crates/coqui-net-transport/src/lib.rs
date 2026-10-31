//! Protocol-neutral ordered message framing and transport recovery policy.
//!
//! Networking drivers, certificate validation, coqui-peer handshakes and
//! application session reconciliation are intentionally separate.
#![forbid(unsafe_code)]

/// A reliable connection selected by a platform-specific dialer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Transport {
    WebTransport,
    WebSocket,
}

/// Failures a platform adapter can distinguish. A generic browser error
/// is not proof of MTU failure or certificate rejection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Failure {
    Unsupported,
    Timeout,
    PathLost,
    NoProgress,
    SuspectedMtu,
    Certificate,
    PeerProof,
    Protocol,
    Permission,
    Cancelled,
}

/// Whether application traffic has already crossed the link.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    /// Before any peer authentication or application message.
    Opening,
    /// A live authenticated connection may have unacknowledged operations.
    Active,
}

/// An alternate connection may be opened, never an unverified message sent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Recovery {
    Deny,
    /// Dial WSS, independently prove the configured peer, then start ACP.
    VerifyFallback,
    /// Dial WSS, verify the same peer, and reconcile the old application
    /// session without replaying commands of uncertain outcome.
    VerifyAndReconcile,
}

/// Pure selection policy. The WSS URI must already be advertised for this
/// peer and must be independently authenticated; it is never guessed.
/// A confirmed certificate/key rejection must not be treated as path loss.
#[must_use]
pub const fn recovery(
    failed: Transport,
    phase: Phase,
    failure: Failure,
    has_wss_fallback: bool,
) -> Recovery {
    if !has_wss_fallback || !matches!(failed, Transport::WebTransport) {
        return Recovery::Deny;
    }
    if !matches!(
        failure,
        Failure::Unsupported
            | Failure::Timeout
            | Failure::PathLost
            | Failure::NoProgress
            | Failure::SuspectedMtu
    ) {
        return Recovery::Deny;
    }
    match phase {
        Phase::Opening => Recovery::VerifyFallback,
        Phase::Active => Recovery::VerifyAndReconcile,
    }
}

/// One explicit set of endpoint locations for a single expected peer.
/// A caller validates URLs and certificate policy and never guesses WSS
/// from the WebTransport URL.
#[derive(Clone, Copy, Debug)]
pub struct EndpointSet<'a> {
    pub webtransport: Option<&'a str>,
    pub websocket: Option<&'a str>,
}

/// A native or browser adapter provides the actual connection and lifecycle.
pub trait Channel {
    fn close(&mut self);
}

pub trait Dialer {
    type Connection: Channel;

    fn dial(&mut self, transport: Transport, endpoint: &str) -> Result<Self::Connection, Failure>;
}

/// Optional transport liveness probe, performed before authentication or ACP
/// messages. A real probe requires protocol support at both endpoints.
pub trait Preflight<C: Channel> {
    fn check(&mut self, transport: Transport, channel: &mut C) -> Result<(), Failure>;
}

pub struct NoPreflight;

impl<C: Channel> Preflight<C> for NoPreflight {
    fn check(&mut self, _: Transport, _: &mut C) -> Result<(), Failure> {
        Ok(())
    }
}

/// The resulting channel is NOT trusted yet. The caller runs an independent
/// coqui-peer handshake with the expected peer identity on this channel.
pub struct UnverifiedLink<C: Channel> {
    pub transport: Transport,
    pub channel: C,
}

/// Open WebTransport first. A failed pre-auth probe can trigger fallback even
/// when the transport nominally opened, for instance due to QUIC PMTU stalls.
/// Never retry after returning a link; application recovery is explicit.
pub fn establish<D, P>(
    endpoints: EndpointSet<'_>,
    dialer: &mut D,
    probe: &mut P,
) -> Result<UnverifiedLink<D::Connection>, Failure>
where
    D: Dialer,
    P: Preflight<D::Connection>,
{
    if let Some(endpoint) = endpoints.webtransport {
        let attempted = dialer
            .dial(Transport::WebTransport, endpoint)
            .and_then(
                |mut channel| match probe.check(Transport::WebTransport, &mut channel) {
                    Ok(()) => Ok(channel),
                    Err(reason) => {
                        channel.close();
                        Err(reason)
                    }
                },
            );
        match attempted {
            Ok(channel) => {
                return Ok(UnverifiedLink {
                    transport: Transport::WebTransport,
                    channel,
                });
            }
            Err(reason)
                if recovery(
                    Transport::WebTransport,
                    Phase::Opening,
                    reason,
                    endpoints.websocket.is_some(),
                ) == Recovery::VerifyFallback => {}
            Err(reason) => return Err(reason),
        }
    }
    let Some(endpoint) = endpoints.websocket else {
        return Err(Failure::Unavailable);
    };
    let mut channel = dialer.dial(Transport::WebSocket, endpoint)?;
    if let Err(reason) = probe.check(Transport::WebSocket, &mut channel) {
        channel.close();
        return Err(reason);
    }
    Ok(UnverifiedLink {
        transport: Transport::WebSocket,
        channel,
    })
}

/// A whole reliable message. No guarantee is made about remote execution.
/// Stream connections use little-endian u32 body-length framing; WebSocket
/// adapters can deliver a complete body per message without framing.
pub const DEFAULT_MAX_MESSAGE: usize = 1024 * 1024;

/// Prefix a nonempty bounded body with its size for a reliable stream.
pub fn encode_frame(body: &[u8], maximum: usize) -> Result<Vec<u8>, FrameError> {
    if body.is_empty() || body.len() > maximum {
        return Err(FrameError::Size);
    }
    let size = u32::try_from(body.len()).map_err(|_| FrameError::Size)?;
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(body);
    Ok(out)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameError {
    Size,
}

/// Incremental bounded decoder for fragments or coalesced stream messages.
pub struct Decoder {
    buffered: Vec<u8>,
    maximum: usize,
}

impl Decoder {
    #[must_use]
    pub fn new(maximum: usize) -> Self {
        Self {
            buffered: Vec::new(),
            maximum,
        }
    }

    pub fn push(&mut self, part: &[u8]) -> Result<Vec<Vec<u8>>, FrameError> {
        self.buffered.extend_from_slice(part);
        let mut messages = Vec::new();
        let mut cursor = 0;
        while self.buffered.len() - cursor >= 4 {
            let size = u32::from_le_bytes(
                self.buffered[cursor..cursor + 4]
                    .try_into()
                    .map_err(|_| FrameError::Size)?,
            ) as usize;
            if size == 0 || size > self.maximum {
                self.buffered.clear();
                return Err(FrameError::Size);
            }
            if self.buffered.len() - cursor < size + 4 {
                break;
            }
            messages.push(self.buffered[cursor + 4..cursor + size + 4].to_vec());
            cursor += size + 4;
        }
        self.buffered.drain(..cursor);
        if self.buffered.len() > self.maximum.saturating_add(4) {
            self.buffered.clear();
            return Err(FrameError::Size);
        }
        Ok(messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_failure_falls_back_with_identity_verification() {
        for failure in [
            Failure::Unsupported,
            Failure::Timeout,
            Failure::PathLost,
            Failure::NoProgress,
            Failure::SuspectedMtu,
        ] {
            assert_eq!(
                recovery(Transport::WebTransport, Phase::Opening, failure, true),
                Recovery::VerifyFallback
            );
            assert_eq!(
                recovery(Transport::WebTransport, Phase::Active, failure, true),
                Recovery::VerifyAndReconcile
            );
        }
    }

    #[test]
    fn never_treat_peer_or_tls_rejection_as_mtu() {
        for failure in [
            Failure::Certificate,
            Failure::PeerProof,
            Failure::Protocol,
            Failure::Permission,
            Failure::Cancelled,
        ] {
            assert_eq!(
                recovery(Transport::WebTransport, Phase::Active, failure, true),
                Recovery::Deny
            );
        }
        assert_eq!(
            recovery(Transport::WebSocket, Phase::Active, Failure::PathLost, true),
            Recovery::Deny
        );
        assert_eq!(
            recovery(
                Transport::WebTransport,
                Phase::Active,
                Failure::PathLost,
                false
            ),
            Recovery::Deny
        );
    }

    struct TestChannel {
        log: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    }

    impl Channel for TestChannel {
        fn close(&mut self) {
            self.log.borrow_mut().push("close".to_string());
        }
    }

    struct TestDialer {
        log: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    }

    impl Dialer for TestDialer {
        type Connection = TestChannel;

        fn dial(&mut self, transport: Transport, _: &str) -> Result<TestChannel, Failure> {
            self.log.borrow_mut().push(format!("dial:{transport:?}"));
            Ok(TestChannel {
                log: self.log.clone(),
            })
        }
    }

    struct TestProbe {
        log: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
        problem: Failure,
    }

    impl Preflight<TestChannel> for TestProbe {
        fn check(&mut self, transport: Transport, _: &mut TestChannel) -> Result<(), Failure> {
            self.log.borrow_mut().push(format!("probe:{transport:?}"));
            if transport == Transport::WebTransport {
                return Err(self.problem);
            }
            Ok(())
        }
    }

    #[test]
    fn establishment_retries_only_pre_auth_path_failures() {
        for problem in [
            Failure::SuspectedMtu,
            Failure::Timeout,
            Failure::Certificate,
        ] {
            let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let mut dial = TestDialer { log: log.clone() };
            let mut probe = TestProbe {
                log: log.clone(),
                problem,
            };
            let result = establish(
                EndpointSet {
                    webtransport: Some("https://peer.example:4433/"),
                    websocket: Some("wss://peer.example/"),
                },
                &mut dial,
                &mut probe,
            );
            if problem == Failure::Certificate {
                assert!(matches!(result, Err(Failure::Certificate)));
                assert_eq!(
                    *log.borrow(),
                    ["dial:WebTransport", "probe:WebTransport", "close"]
                );
            } else {
                assert_eq!(result.unwrap().transport, Transport::WebSocket);
                assert_eq!(
                    *log.borrow(),
                    [
                        "dial:WebTransport",
                        "probe:WebTransport",
                        "close",
                        "dial:WebSocket",
                        "probe:WebSocket"
                    ]
                );
            }
        }
    }

    #[test]
    fn frames_survive_fragmentation_and_coalescing() {
        let mut bytes = encode_frame(b"hello", 10).unwrap();
        bytes.extend(encode_frame(b"world", 10).unwrap());
        let mut decoder = Decoder::new(10);
        assert!(decoder.push(&bytes[..3]).unwrap().is_empty());
        assert_eq!(
            decoder.push(&bytes[3..]).unwrap(),
            vec![b"hello".to_vec(), b"world".to_vec()]
        );
        assert_eq!(encode_frame(b"", 10), Err(FrameError::Size));
        assert_eq!(decoder.push(&[11, 0, 0, 0]), Err(FrameError::Size));
    }
}
