"""Connects to the Unix socket named by argv[1], sends one line, and prints what came back: `refused` when the system turned the
connection away, `empty` when it was let in and closed with no answer, else the answer. Run as one of the test accounts."""
import socket
import sys

path, line = sys.argv[1], sys.argv[2]
try:
    sock = socket.socket(socket.AF_UNIX)
    sock.settimeout(5)
    sock.connect(path)
except OSError as error:
    print("refused", error.errno)
    sys.exit(0)
try:
    sock.sendall(line.encode() + b"\n")
    data = sock.recv(65536)
except OSError:
    data = b""
print("empty" if not data else data.decode(errors="replace"))
