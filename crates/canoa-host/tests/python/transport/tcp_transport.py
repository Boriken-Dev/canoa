"""A `transport` provider written in Python: connections over loopback TCP, so a client in another process can reach it.

An endpoint is `tcp://127.0.0.1:PORT` (port 0 takes a free one). A message is a four-byte big-endian length and the
message. The kind's rules are kept: `send` waits for room and answers `GONE` once either end has closed; one thread of its own
per connection is the only caller of the sink, and tells it the end exactly once; a listener's own thread is the only caller
of its acceptor, and releases the endpoint before it says it has closed.
"""

from __future__ import annotations

import socket
import struct
import threading
from dataclasses import dataclass
from typing import Any, Dict, List, Optional, Tuple

from coqui_transport_guatiao_kinds import Channel, ChannelSink, Transport, TransportAcceptor, TransportListener

from guatiao import Ref, provide
from guatiao.errors import BadValue, Gone

__version__ = "1.0.0"
__all__ = ["Tcp"]

OK = 0
BAD_VALUE = 7
GONE = 12
INTERNAL = 100

DIAL_KEYS = {"endpoint", "timeout_ms"}
LISTEN_KEYS = {"endpoint"}


def _exact(sock: socket.socket, count: int) -> Optional[bytes]:
    """`count` bytes, or None when the peer ended first."""
    data = b""
    while len(data) < count:
        piece = sock.recv(count - len(data))
        if not piece:
            return None
        data += piece
    return data


class _State:
    """How one connection ended, as the host or the provider tells it."""

    def __init__(self) -> None:
        self.lock = threading.Lock()
        self.closed: Optional[Tuple[int, str]] = None

    def close(self, status: int, reason: str, sock: socket.socket) -> None:
        with self.lock:
            if self.closed is not None:
                return
            self.closed = (status, reason)
        try:
            sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass


class _Connection(Channel):
    """The host's hold on one connection. Dropping it closes."""

    def __init__(self, sock: socket.socket, state: _State, max_message: int) -> None:
        self._sock = sock
        self._state = state
        self._max = max_message
        self._writing = threading.Lock()

    def send(self, bytes: bytes) -> None:
        if len(bytes) > self._max:
            raise BadValue("the message is larger than this transport takes")
        with self._writing:
            if self._state.closed is not None:
                raise Gone("the connection is closed")
            try:
                self._sock.sendall(struct.pack(">I", len(bytes)) + bytes)
            except OSError:
                raise Gone("the connection is closed")

    def close(self, status: int, reason: str) -> None:
        self._state.close(status, reason, self._sock)

    def __del__(self) -> None:
        self._state.close(OK, "", self._sock)


def _read(sock: socket.socket, state: _State, sink: ChannelSink, max_message: int) -> None:
    """The one thread that calls the sink: messages in order, then the end, once."""
    status: Tuple[int, str] = (OK, "")
    try:
        while True:
            head = _exact(sock, 4)
            if head is None:
                break
            (size,) = struct.unpack(">I", head)
            if size > max_message:
                status = (BAD_VALUE, "a message over the limit")
                break
            body = _exact(sock, size)
            if body is None:
                status = (GONE, "the peer left in the middle of a message")
                break
            try:
                sink.receive(body)  # type: ignore[arg-type]
            except Exception as e:  # a sink that refuses ends the connection
                status = (int(getattr(e, "status", INTERNAL)), str(e))
                break
    except OSError:
        status = (GONE, "the connection failed")
    with state.lock:
        if state.closed is not None:
            status = state.closed
        else:
            state.closed = status
    try:
        sock.close()
    except OSError:
        pass
    sink.closed(status[0], status[1])


def _serve(sock: socket.socket, sink: ChannelSink, max_message: int) -> _State:
    state = _State()
    threading.Thread(
        target=_read, args=(sock, state, sink, max_message), name="tcp_transport_reader", daemon=True
    ).start()
    return state


def _address(endpoint: str) -> Tuple[str, int]:
    if not endpoint.startswith("tcp://"):
        raise BadValue("the endpoint is not tcp://HOST:PORT")
    host, _, port = endpoint[len("tcp://") :].rpartition(":")
    if host != "127.0.0.1" or not port.isdigit() or int(port) > 65535:
        raise BadValue("the endpoint is not tcp://127.0.0.1:PORT")
    return host, int(port)


class _Stop:
    def __init__(self) -> None:
        self.lock = threading.Lock()
        self.value: Optional[Tuple[int, str]] = None

    def set(self, status: int, reason: str) -> None:
        with self.lock:
            if self.value is None:
                self.value = (status, reason)


class _Listening(TransportListener):
    def __init__(self, address: str, stop: _Stop) -> None:
        self._address = address
        self._stop = stop

    def address(self) -> str:
        return self._address

    def close(self, status: int, reason: str) -> None:
        self._stop.set(status, reason)

    def __del__(self) -> None:
        self._stop.set(OK, "")


def _accept(server: socket.socket, acceptor: TransportAcceptor, stop: _Stop, max_message: int) -> None:
    """The listener's thread, the only caller of the acceptor."""
    server.settimeout(0.05)
    while stop.value is None:
        try:
            sock, peer = server.accept()
        except socket.timeout:
            continue
        except OSError:
            break
        sock.settimeout(None)
        state = _State()
        try:
            sink = acceptor.accepted(f"{peer[0]}:{peer[1]}", _Connection(sock, state, max_message))
        except Exception:
            state.close(OK, "", sock)
            sock.close()
            continue
        threading.Thread(
            target=_read, args=(sock, state, sink, max_message), name="tcp_transport_reader", daemon=True
        ).start()
    status = stop.value or (GONE, "the listening socket failed")
    # The endpoint is free before the host is told it is closed.
    server.close()
    acceptor.closed(status[0], status[1])


@dataclass(frozen=True)
class TcpConfig:
    """How the TCP transport is configured."""

    #: The largest message, in bytes.
    max_message: int = 1048576


class Tcp(provide.Provider, Transport, display_name="TCP, in Python", default=True):
    """Connections over loopback TCP."""

    def __init__(self, config: TcpConfig = TcpConfig()) -> None:
        if config.max_message < 1:
            raise BadValue("`max_message` is at least 1")
        self.config = config

    def _endpoint(self, request: Ref, keys: set) -> str:
        asked = request.to_python()
        if not isinstance(asked, dict) or not set(asked) <= keys or not isinstance(asked.get("endpoint"), str):
            raise BadValue("the request is not one this transport reads")
        return asked["endpoint"]

    def describe(self) -> Dict[str, Any]:
        return {
            "title": "TCP, in Python",
            "endpoint": "tcp://127.0.0.1:PORT",
            "dials": True,
            "listens": True,
            "max_message_bytes": self.config.max_message,
        }

    def dial(self, request: Ref, sink: ChannelSink) -> Channel:
        asked = request.to_python()
        host, port = _address(self._endpoint(request, DIAL_KEYS))
        try:
            sock = socket.create_connection((host, port), timeout=float(asked.get("timeout_ms", 5000)) / 1000)
        except OSError:
            raise Gone("nobody answered")
        sock.settimeout(None)
        return _Connection(sock, _serve(sock, sink, self.config.max_message), self.config.max_message)

    def listen(self, request: Ref, acceptor: TransportAcceptor) -> TransportListener:
        host, port = _address(self._endpoint(request, LISTEN_KEYS))
        server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            server.bind((host, port))
            server.listen(8)
        except OSError:
            server.close()
            raise Gone("that endpoint is taken")
        stop = _Stop()
        bound = server.getsockname()
        threading.Thread(
            target=_accept,
            args=(server, acceptor, stop, self.config.max_message),
            name="tcp_transport_accept",
            daemon=True,
        ).start()
        return _Listening(f"tcp://{bound[0]}:{bound[1]}", stop)

    def non_defaults(self) -> List[Dict[str, str]]:
        defaults = TcpConfig()
        if self.config.max_message == defaults.max_message:
            return []
        return [
            {
                "setting": "max_message",
                "value": f"usize({self.config.max_message})",
                "default": f"usize({defaults.max_message})",
            }
        ]
