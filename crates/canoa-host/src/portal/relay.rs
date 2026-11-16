//! A browser's WebSocket carried to one socket of the user's own host, message for message. The portal opens nothing of what
//! passes: the browser and the host prove their keys to each other through it, and what follows is sealed in their session.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use core::time::Duration;
use std::io::{self, Read, Write};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::Instant;

use coqui_transport::{Channel, Failure};
use tungstenite::protocol::{Role, WebSocketConfig};
use tungstenite::{Error, Message, WebSocket};

use super::conn::Conn;
use crate::pipe::PipeChannel;

/// How often the reader looks at whether the session still stands.
const TICK: Duration = Duration::from_millis(500);
/// How long the pump waits for the host before looking at whether the relay ended.
const PUMP_WAIT: Duration = Duration::from_millis(250);
/// How long a write to the browser may make no progress before the channel ends.
const WRITE_STALL: Duration = Duration::from_secs(30);

/// A socket read by one thread and written by another: this half only reads, and what the WebSocket would write through it
/// (a pong, a close) goes nowhere, since the writing half sends them under its own lock.
struct ReadHalf(Conn);

impl Read for ReadHalf {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer)
    }
}

impl Write for ReadHalf {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The answer that accepts a WebSocket upgrade whose `Sec-WebSocket-Key` is `key`.
#[must_use]
pub fn upgrade_answer(key: &str) -> String {
    format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
        tungstenite::handshake::derive_accept_key(key.as_bytes())
    )
}

/// Pings sent with nothing back from the browser, past which the link is closed.
const UNANSWERED: u32 = 2;

/// Carries messages, at most `max_message` bytes each, between the browser's WebSocket, already upgraded, and `channel` until
/// either ends, the session `alive` no longer stands, or `stopped` is set. Returns when both directions have ended.
/// Every `ping` (never when zero) the browser is pinged, so a proxy that cuts a silent connection keeps the link; one that
/// sends nothing in answer to `UNANSWERED` pings in a row is closed.
pub fn run(
    conn: &Conn,
    channel: PipeChannel,
    alive: &(dyn Fn() -> bool + Sync),
    stopped: &AtomicBool,
    max_message: usize,
    ping: Duration,
) {
    let config = WebSocketConfig::default()
        .max_message_size(Some(max_message))
        .max_frame_size(Some(max_message));
    // A browser that stops reading ends the channel instead of holding the writer's lock.
    if conn.set_read_timeout(Some(TICK)).is_err()
        || conn.set_write_timeout(Some(WRITE_STALL)).is_err()
    {
        return;
    }
    let writer = Arc::new(Mutex::new(WebSocket::from_raw_socket(
        conn.clone(),
        Role::Server,
        Some(config),
    )));
    let mut reader = WebSocket::from_raw_socket(ReadHalf(conn.clone()), Role::Server, Some(config));
    let channel = Arc::new(channel);
    let done = Arc::new(AtomicBool::new(false));
    // Pings sent since the browser last sent anything.
    let unanswered = Arc::new(AtomicU32::new(0));
    let pump = {
        let (writer, channel, done, unanswered) = (
            Arc::clone(&writer),
            Arc::clone(&channel),
            Arc::clone(&done),
            Arc::clone(&unanswered),
        );
        let conn = conn.clone();
        thread::Builder::new()
            .name("canoa-relay".to_owned())
            .spawn(move || {
                let mut last_ping = Instant::now();
                while !done.load(Ordering::SeqCst) {
                    if !ping.is_zero() && last_ping.elapsed() >= ping {
                        if unanswered.fetch_add(1, Ordering::SeqCst) >= UNANSWERED {
                            break;
                        }
                        last_ping = Instant::now();
                        let sent = writer
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .send(Message::Ping(Vec::new().into()));
                        if sent.is_err() {
                            break;
                        }
                    }
                    match channel.receive(Some(PUMP_WAIT)) {
                        Ok(Some(message)) => {
                            let sent = writer
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .send(Message::Binary(message.into()));
                            if sent.is_err() {
                                break;
                            }
                        }
                        Err(Failure::Timeout) => {}
                        Ok(None) | Err(_) => break,
                    }
                }
                done.store(true, Ordering::SeqCst);
                // Wakes the reader, which is waiting on the browser.
                conn.shutdown();
            })
    };
    let Ok(pump) = pump else {
        return;
    };
    while !done.load(Ordering::SeqCst) && !stopped.load(Ordering::SeqCst) && alive() {
        let read = reader.read();
        // Any frame from the browser, a pong most of all, shows the link carries traffic both ways.
        if read.is_ok() {
            unanswered.store(0, Ordering::SeqCst);
        }
        match read {
            Ok(Message::Binary(message)) => {
                if channel.send(&message).is_err() {
                    break;
                }
            }
            Ok(Message::Ping(payload)) => {
                if let Ok(mut writer) = writer.try_lock() {
                    let _ = writer.send(Message::Pong(payload));
                }
            }
            // The host's protocol is binary alone, and a close is the browser's end.
            Ok(Message::Text(_) | Message::Close(_)) => break,
            Ok(_) => {}
            Err(Error::Io(error))
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
    }
    done.store(true, Ordering::SeqCst);
    channel.close();
    // A polite close only when the writer is free, and the socket is shut either way, so a stuck writer ends and releases it.
    if let Ok(mut writer) = writer.try_lock() {
        let _ = writer.close(None);
    }
    conn.shutdown();
    let _ = pump.join();
}
