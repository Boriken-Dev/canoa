"""A stand-in for the proxy in front of `canoa portal`, for the real-browser run of the sign-in page.

    python3 portal_front.py PORT UNIX_SOCKET

Listens on 127.0.0.1:PORT and forwards each request to the portal's Unix socket, one connection a request. Like the real proxy
it sets `X-Forwarded-For` itself, from the request's `X-Test-Address` header (10.1.2.3 without one), and drops what the browser
sent in `X-Forwarded-For`, `X-Forwarded-Proto` and `Connection`. `X-Client-Cert` passes as the browser sets it, since the
browser stands in for the proxy's certificate header. It is a test aid, not a proxy to run anywhere else.
"""
import socket
import sys
import threading

DROPPED = {b"x-forwarded-for", b"x-forwarded-proto", b"connection", b"x-test-address"}


def read_request(client):
    buffer = b""
    while b"\r\n\r\n" not in buffer:
        chunk = client.recv(65536)
        if not chunk:
            return None
        buffer += chunk
    head, _, rest = buffer.partition(b"\r\n\r\n")
    lines = head.split(b"\r\n")
    length = 0
    address = b"10.1.2.3"
    kept = []
    for line in lines[1:]:
        name, _, value = line.partition(b":")
        key = name.strip().lower()
        if key == b"content-length":
            length = int(value)
        if key == b"x-test-address":
            address = value.strip()
        if key not in DROPPED:
            kept.append(line)
    body = rest
    while len(body) < length:
        chunk = client.recv(65536)
        if not chunk:
            break
        body += chunk
    kept += [b"X-Forwarded-For: " + address, b"Connection: close"]
    return lines[0], kept, body[:length]


def handle(client, unix_path):
    try:
        request = read_request(client)
        if request is None:
            return
        line, headers, body = request
        upstream = socket.socket(socket.AF_UNIX)
        upstream.connect(unix_path)
        upstream.sendall(line + b"\r\n" + b"\r\n".join(headers) + b"\r\n\r\n" + body)
        while True:
            chunk = upstream.recv(65536)
            if not chunk:
                break
            client.sendall(chunk)
        upstream.close()
    except OSError:
        pass
    finally:
        client.close()


def main():
    port, unix_path = int(sys.argv[1]), sys.argv[2]
    server = socket.socket()
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind(("127.0.0.1", port))
    server.listen(32)
    while True:
        client, _ = server.accept()
        threading.Thread(target=handle, args=(client, unix_path), daemon=True).start()


if __name__ == "__main__":
    main()
