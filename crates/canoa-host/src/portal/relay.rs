//! A browser's WebSocket carried to one socket of the user's own host, message for message. The portal opens nothing of what
//! passes: the browser and the host prove their keys to each other through it, and what follows is sealed in their session.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::{Mutex, PoisonError};
use std::thread;

use coqui_transport::{Channel, Failure};
use tungstenite::protocol::{Role, WebSocketConfig};
use tungstenite::{Error, Message, WebSocket};

use crate::pipe::PipeChannel;

/// The largest message either way: the sealed session's largest record and a little over.
pub const MAX_MESSAGE: usize = 17 * 1024 * 1024;
/// How often the reader looks at whether the session still stands.
const TICK: Duration = Duration::from_millis(500);
/// How long the pump waits for the host before looking at whether the relay ended.
const PUMP_WAIT: Duration = Duration::from_millis(250);

/// A socket read by one thread and written by another: this half only reads, and what the WebSocket would write through it
/// (a pong, a close) goes nowhere, since the writing half sends them under its own lock.
struct ReadHalf(TcpStream);

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

/// Carries messages between the browser's WebSocket, already upgraded, and `channel` until either ends, the session `alive`
/// no longer stands, or `stopped` is set. Returns when both directions have ended.
pub fn run(
    stream: &TcpStream,
    channel: PipeChannel,
    alive: &(dyn Fn() -> bool + Sync),
    stopped: &AtomicBool,
) {
    let Ok(write_side) = stream.try_clone() else {
        return;
    };
    let Ok(read_side) = stream.try_clone() else {
        return;
    };
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE));
    if stream.set_read_timeout(Some(TICK)).is_err() {
        return;
    }
    let writer = Arc::new(Mutex::new(WebSocket::from_raw_socket(
        write_side,
        Role::Server,
        Some(config),
    )));
    let mut reader = WebSocket::from_raw_socket(ReadHalf(read_side), Role::Server, Some(config));
    let channel = Arc::new(channel);
    let done = Arc::new(AtomicBool::new(false));
    let pump = {
        let (writer, channel, done) =
            (Arc::clone(&writer), Arc::clone(&channel), Arc::clone(&done));
        let stream = stream.try_clone();
        thread::Builder::new()
            .name("canoa-relay".to_owned())
            .spawn(move || {
                while !done.load(Ordering::SeqCst) {
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
                if let Ok(stream) = stream {
                    let _ = stream.shutdown(Shutdown::Both);
                }
            })
    };
    let Ok(pump) = pump else {
        return;
    };
    while !done.load(Ordering::SeqCst) && !stopped.load(Ordering::SeqCst) && alive() {
        match reader.read() {
            Ok(Message::Binary(message)) => {
                if channel.send(&message).is_err() {
                    break;
                }
            }
            Ok(Message::Ping(payload)) => {
                let _ = writer
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .send(Message::Pong(payload));
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
    let _ = writer
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .close(None);
    let _ = stream.shutdown(Shutdown::Both);
    let _ = pump.join();
}
