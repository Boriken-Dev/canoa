//! The page's side of a peer proof, for tests on the Dart VM, which has no WebCrypto of its own: the calls of the app's
//! `PeerBridge` (`app/lib/auth/peer_proof.dart`) answered by the Rust peer crates, so the app's connection code meets a
//! peer that proves and pairs the way a browser does.
//!
//! One line of JSON per call on standard input, `{"id", "op", "args"}`, one answer per line on standard output, `{"id",
//! "result"}` or `{"id", "error"}`; bytes are base64. Each method's key is made for this process and kept in memory. A proof
//! or a pairing is a token that `step` advances; a proof ends in a session token that `seal`, `open` and `closeRecord` use.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "an example names what it uses from std"
)]

use std::collections::BTreeMap;
use std::io::{self, BufRead as _, Write as _};
use std::sync::mpsc::{Receiver, Sender, channel};

use coqui_peer::pair::{CodeAlphabet, Pairing, Pin, PinRole};
use coqui_peer::{Handshake, MessageTransport, Method as _, PeerId};
use data_encoding::BASE64;
use serde_json::{Value, json};

type Failure = String;

fn failed(error: impl core::fmt::Display) -> Failure {
    error.to_string()
}

/// A session either method ends a proof in.
enum Session {
    WebCrypto(coqui_peer_webcrypto::Session),
    Noise(coqui_peer_noise::Session),
}

/// What a joiner's thread tells the call waiting on it.
enum Event {
    Send(Vec<u8>),
    Waiting,
    Done(Result<String, Failure>),
}

/// The pairing exchange's transport, ends of two channels: what the thread sends reaches the call, and a call hands the
/// thread the peer's next message.
struct Link {
    events: Sender<Event>,
    deliver: Receiver<Vec<u8>>,
}

impl MessageTransport for Link {
    fn send(&mut self, message: &[u8]) -> io::Result<()> {
        self.events
            .send(Event::Send(message.to_vec()))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "the call has gone"))
    }

    fn recv(&mut self) -> io::Result<Vec<u8>> {
        self.events
            .send(Event::Waiting)
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "the call has gone"))?;
        self.deliver
            .recv()
            .map_err(|_| io::Error::new(io::ErrorKind::ConnectionAborted, "the connection ended"))
    }
}

enum Run {
    WebCrypto(Option<coqui_peer_webcrypto::WebCryptoHandshake>),
    Noise(Option<coqui_peer_noise::NoiseHandshake>),
    Pair(Pair),
}

struct Pair {
    stage: u8,
    handshake: Option<coqui_peer_noise::PairingHandshake>,
    pin: String,
    events: Option<Receiver<Event>>,
    deliver: Option<Sender<Vec<u8>>>,
}

/// What a step gives back.
struct Step {
    send: Vec<Vec<u8>>,
    peer: Option<String>,
    session: Option<String>,
}

fn drive<H: Handshake>(
    handshake: &mut H,
    received: Option<&[u8]>,
) -> Result<Vec<Vec<u8>>, Failure> {
    if let Some(received) = received {
        handshake.receive(received).map_err(failed)?;
    }
    let mut out = Vec::new();
    while let Some(message) = handshake.transmit().map_err(failed)? {
        out.push(message);
    }
    Ok(out)
}

struct Peer {
    web: coqui_peer_webcrypto::Identity,
    noise: coqui_peer_noise::Identity,
    runs: BTreeMap<String, Run>,
    sessions: BTreeMap<String, Session>,
    next: u64,
}

impl Peer {
    fn token(&mut self, kind: &str) -> String {
        self.next += 1;
        format!("{kind}{}", self.next)
    }

    fn local_id(&self, method: &str) -> Result<String, Failure> {
        match method {
            "webcrypto" => Ok(self.web.peer_id().to_string()),
            "noise" => Ok(self.noise.peer_id().to_string()),
            other => Err(format!("{other} is not a proof method")),
        }
    }

    fn begin(&mut self, method: &str, expected: &str) -> Result<String, Failure> {
        let expected: PeerId = expected.parse().map_err(failed)?;
        let run = match method {
            "webcrypto" => {
                let method = coqui_peer_webcrypto::WebCrypto::builder()
                    .identity(self.web.clone())
                    .responders(coqui_peer_webcrypto::Trust::pinned([expected]))
                    .build()
                    .map_err(failed)?;
                Run::WebCrypto(Some(method.initiate().map_err(failed)?))
            }
            "noise" => {
                let method = coqui_peer_noise::Noise::builder()
                    .identity(self.noise.clone())
                    .responders(coqui_peer_noise::Trust::pinned([expected]))
                    .build()
                    .map_err(failed)?;
                Run::Noise(Some(method.initiate().map_err(failed)?))
            }
            other => return Err(format!("{other} is not a proof method")),
        };
        let token = self.token("p");
        self.runs.insert(token.clone(), run);
        Ok(token)
    }

    fn pair(&mut self, pin: &str) -> Result<String, Failure> {
        let handshake = coqui_peer_noise::PairingMethod::builder()
            .identity(self.noise.clone())
            .build()
            .map_err(failed)?
            .initiate()
            .map_err(failed)?;
        let token = self.token("p");
        self.runs.insert(
            token.clone(),
            Run::Pair(Pair {
                stage: 0,
                handshake: Some(handshake),
                pin: pin.to_owned(),
                events: None,
                deliver: None,
            }),
        );
        Ok(token)
    }

    fn step(&mut self, token: &str, received: Option<Vec<u8>>) -> Result<Step, Failure> {
        let result = self.advance(token, received.as_deref());
        // A step that failed, or that ended a proof, leaves nothing to advance.
        if result.is_err() || matches!(&result, Ok(step) if step.session.is_some()) {
            self.runs.remove(token);
        }
        result
    }

    fn advance(&mut self, token: &str, received: Option<&[u8]>) -> Result<Step, Failure> {
        let mut proved = None;
        let step = match self
            .runs
            .get_mut(token)
            .ok_or("Peer proof expired or unknown")?
        {
            Run::WebCrypto(slot) => {
                let handshake = slot.as_mut().ok_or("Unexpected message after the proof")?;
                let send = drive(handshake, received)?;
                if handshake.is_complete() {
                    let established = slot.take().expect("held above").finish().map_err(failed)?;
                    proved = Some((
                        established.peer.to_string(),
                        Session::WebCrypto(established.session),
                    ));
                }
                Step {
                    send,
                    peer: None,
                    session: None,
                }
            }
            Run::Noise(slot) => {
                let handshake = slot.as_mut().ok_or("Unexpected message after the proof")?;
                let send = drive(handshake, received)?;
                if handshake.is_complete() {
                    let established = slot.take().expect("held above").finish().map_err(failed)?;
                    proved = Some((
                        established.peer.to_string(),
                        Session::Noise(established.session),
                    ));
                }
                Step {
                    send,
                    peer: None,
                    session: None,
                }
            }
            Run::Pair(pair) => return step_pair(pair, received),
        };
        match proved {
            Some((peer, session)) => {
                let sealing = self.token("s");
                self.sessions.insert(sealing.clone(), session);
                Ok(Step {
                    peer: Some(peer),
                    session: Some(sealing),
                    ..step
                })
            }
            None => Ok(step),
        }
    }

    fn session(&mut self, token: &str) -> Result<&mut Session, Failure> {
        self.sessions
            .get_mut(token)
            .ok_or_else(|| "Session closed or unknown".to_owned())
    }
}

/// A pairing: the Noise pairing handshake, then the exchange inside it on a thread of its own, run a message at a time.
fn step_pair(pair: &mut Pair, received: Option<&[u8]>) -> Result<Step, Failure> {
    let mut send = Vec::new();
    match pair.stage {
        0 => {
            pair.stage = 1;
            let handshake = pair.handshake.as_mut().ok_or("Unexpected step")?;
            return Ok(Step {
                send: drive(handshake, None)?,
                peer: None,
                session: None,
            });
        }
        1 => {
            pair.stage = 2;
            let mut handshake = pair.handshake.take().ok_or("Unexpected step")?;
            send = drive(
                &mut handshake,
                Some(received.ok_or("Unexpected step without a message")?),
            )?;
            let session = handshake.finish().map_err(failed)?.session;
            let pin = Pin::parse(&CodeAlphabet::pin(), &pair.pin).map_err(failed)?;
            let (events, events_in) = channel();
            let (deliver_out, deliver) = channel();
            let mut link = Link {
                events: events.clone(),
                deliver,
            };
            std::thread::spawn(move || {
                let paired = session
                    .join(Pairing::pin(pin, PinRole::Typed), &mut link)
                    .and_then(coqui_peer_noise::PairDial::finish)
                    .map(|paired| paired.peer.id().to_string())
                    .map_err(failed);
                let _ = events.send(Event::Done(paired));
            });
            pair.events = Some(events_in);
            pair.deliver = Some(deliver_out);
        }
        _ => {
            let message = received.ok_or("Unexpected step without a message")?;
            let deliver = pair
                .deliver
                .as_ref()
                .ok_or("Unexpected message after the pairing")?;
            deliver
                .send(message.to_vec())
                .map_err(|_| "Unexpected message after the pairing".to_owned())?;
        }
    }
    // Everything the thread sends until it next waits for the peer, or ends.
    let events = pair.events.as_ref().ok_or("Unexpected step")?;
    loop {
        match events.recv() {
            Ok(Event::Send(message)) => send.push(message),
            Ok(Event::Waiting) => {
                return Ok(Step {
                    send,
                    peer: None,
                    session: None,
                });
            }
            Ok(Event::Done(Ok(peer))) => {
                return Ok(Step {
                    send,
                    peer: Some(peer),
                    session: None,
                });
            }
            Ok(Event::Done(Err(error))) => return Err(format!("The pairing was refused: {error}")),
            Err(_) => return Err("The pairing ended".to_owned()),
        }
    }
}

fn bytes(value: &Value) -> Result<Option<Vec<u8>>, Failure> {
    match value.as_str() {
        None => Ok(None),
        Some(text) => BASE64.decode(text.as_bytes()).map(Some).map_err(failed),
    }
}

fn text(data: &[u8]) -> Value {
    json!(BASE64.encode(data))
}

fn call(peer: &mut Peer, op: &str, args: &[Value]) -> Result<Value, Failure> {
    let string = |index: usize| args.get(index).and_then(Value::as_str).unwrap_or("");
    let message = |index: usize| -> Result<Vec<u8>, Failure> {
        bytes(args.get(index).unwrap_or(&Value::Null))?
            .ok_or_else(|| "A message is needed".to_owned())
    };
    match op {
        "unreachable" => Ok(Value::Null),
        "localId" => Ok(json!(peer.local_id(string(0))?)),
        "begin" => Ok(json!(peer.begin(string(0), string(1))?)),
        "pair" => Ok(json!(peer.pair(string(0))?)),
        "step" => {
            let received = bytes(args.get(1).unwrap_or(&Value::Null))?;
            let step = peer.step(string(0), received)?;
            Ok(json!({
                "send": step.send.iter().map(|message| text(message)).collect::<Vec<_>>(),
                "peerId": step.peer,
                "session": step.session,
            }))
        }
        "abort" => {
            peer.runs.remove(string(0));
            Ok(Value::Null)
        }
        "seal" => {
            let data = message(1)?;
            Ok(text(&match peer.session(string(0))? {
                Session::WebCrypto(session) => session.seal(&data).map_err(failed)?,
                Session::Noise(session) => session.seal(&data).map_err(failed)?,
            }))
        }
        "open" => {
            let record = message(1)?;
            let opened = match peer.session(string(0))? {
                Session::WebCrypto(session) => session.open(&record).map_err(failed)?,
                Session::Noise(session) => session.open(&record).map_err(failed)?,
            };
            Ok(opened.map_or(Value::Null, |data| text(&data)))
        }
        "closeRecord" => Ok(text(&match peer.session(string(0))? {
            Session::WebCrypto(session) => session.close().map_err(failed)?,
            Session::Noise(session) => session.close().map_err(failed)?,
        })),
        "closeSession" => {
            peer.sessions.remove(string(0));
            Ok(Value::Null)
        }
        other => Err(format!("{other} is not a call")),
    }
}

fn main() {
    let mut peer = Peer {
        web: coqui_peer_webcrypto::Identity::generate().expect("a WebCrypto key"),
        noise: coqui_peer_noise::Identity::generate().expect("a Noise key"),
        runs: BTreeMap::new(),
        sessions: BTreeMap::new(),
        next: 0,
    };
    let mut stdout = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { return };
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let args = request["args"].as_array().cloned().unwrap_or_default();
        let answer = match call(&mut peer, request["op"].as_str().unwrap_or(""), &args) {
            Ok(result) => json!({"id": request["id"], "result": result}),
            Err(error) => json!({"id": request["id"], "error": error}),
        };
        let _ = writeln!(stdout, "{answer}");
        let _ = stdout.flush();
    }
}
