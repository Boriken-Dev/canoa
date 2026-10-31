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
            recovery(Transport::WebTransport, Phase::Active, Failure::PathLost, false),
            Recovery::Deny
        );
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
