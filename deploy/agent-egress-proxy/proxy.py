"""Single-attempt HTTPS CONNECT proxy for an isolated worker network.

The container has no published ports. Its policy is an exact host list supplied
by the Rust server after Admin review. The worker network has no external route.
"""

import ipaddress
import os
import selectors
import socket
import socketserver
import struct
import threading
import time

MAX_HEADER = 8192
MAX_TLS_RECORD = 16384
MAX_TUNNEL_BYTES = 20 * 1024 * 1024
MAX_TUNNEL_SECONDS = 300


def allowed_hosts(value):
    hosts = value.split(",") if value else []
    if not hosts or len(hosts) > 16:
        raise ValueError("empty or oversized host policy")
    for host in hosts:
        if (
            len(host) > 128
            or "." not in host
            or host != host.lower()
            or not all(c.isascii() and (c.isalnum() or c in ".-_") for c in host)
            or host.startswith(".")
            or host.endswith(".")
            or ".." in host
        ):
            raise ValueError("invalid host policy")
    return frozenset(hosts)


def public_addresses(host):
    addresses = []
    for family, _, _, _, endpoint in socket.getaddrinfo(host, 443, type=socket.SOCK_STREAM):
        if len(addresses) >= 32:
            raise ValueError("oversized DNS answer")
        if family not in (socket.AF_INET, socket.AF_INET6):
            raise ValueError("unsupported address family")
        ip = ipaddress.ip_address(endpoint[0])
        mapped = getattr(ip, "ipv4_mapped", None)
        if not ip.is_global or (mapped is not None and not mapped.is_global):
            raise ValueError("non-public DNS answer")
        addresses.append((family, str(ip)))
    if not addresses:
        raise ValueError("empty DNS answer")
    return addresses


def client_hello_sni(record):
    """Return the sole SNI host from one bounded TLS ClientHello record."""
    if len(record) < 9 or record[0] != 22 or record[1] != 3:
        raise ValueError("not TLS ClientHello")
    size = struct.unpack_from("!H", record, 3)[0]
    if size > MAX_TLS_RECORD or len(record) != size + 5:
        raise ValueError("invalid TLS record size")
    if record[5] != 1 or int.from_bytes(record[6:9], "big") != size - 4:
        raise ValueError("invalid ClientHello")
    pos = 9
    end = len(record)
    if pos + 35 > end:
        raise ValueError("short ClientHello")
    pos += 34  # legacy version and random
    session_length = record[pos]
    pos += 1 + session_length
    if pos + 2 > end:
        raise ValueError("short cipher list")
    cipher_length = struct.unpack_from("!H", record, pos)[0]
    pos += 2 + cipher_length
    if pos + 1 > end:
        raise ValueError("short compression list")
    compression_length = record[pos]
    pos += 1 + compression_length
    if pos + 2 > end:
        raise ValueError("missing extensions")
    extension_length = struct.unpack_from("!H", record, pos)[0]
    pos += 2
    if pos + extension_length != end:
        raise ValueError("extension length mismatch")
    while pos < end:
        if pos + 4 > end:
            raise ValueError("short extension")
        kind, length = struct.unpack_from("!HH", record, pos)
        pos += 4
        next_pos = pos + length
        if next_pos > end:
            raise ValueError("extension overrun")
        if kind == 0:
            if length < 5:
                raise ValueError("short SNI")
            names_length = struct.unpack_from("!H", record, pos)[0]
            if names_length != length - 2 or record[pos + 2] != 0:
                raise ValueError("invalid SNI list")
            name_length = struct.unpack_from("!H", record, pos + 3)[0]
            if name_length != names_length - 3:
                raise ValueError("multiple or malformed SNI names")
            try:
                return record[pos + 5 : next_pos].decode("ascii")
            except UnicodeDecodeError as error:
                raise ValueError("non-ASCII SNI") from error
        pos = next_pos
    raise ValueError("missing SNI")


def read_exact(connection, count):
    result = bytearray()
    while len(result) < count:
        part = connection.recv(count - len(result))
        if not part:
            raise ValueError("truncated TLS record")
        result.extend(part)
    return bytes(result)


def tunnel(client, remote):
    total = 0
    deadline = time.monotonic() + MAX_TUNNEL_SECONDS
    with selectors.DefaultSelector() as selector:
        client.setblocking(False)
        remote.setblocking(False)
        selector.register(client, selectors.EVENT_READ, remote)
        selector.register(remote, selectors.EVENT_READ, client)
        while time.monotonic() < deadline:
            for key, _ in selector.select(timeout=1):
                source = key.fileobj
                target = key.data
                data = source.recv(65536)
                if not data:
                    return
                total += len(data)
                if total > MAX_TUNNEL_BYTES:
                    return
                target.settimeout(10)
                try:
                    target.sendall(data)
                finally:
                    target.setblocking(False)


class Proxy(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True
    daemon_threads = True
    request_queue_size = 8
    slots = threading.BoundedSemaphore(8)

    def process_request(self, request, client_address):
        if not self.slots.acquire(blocking=False):
            request.close()
            return
        try:
            super().process_request(request, client_address)
        except Exception:
            self.slots.release()
            raise

    def process_request_thread(self, request, client_address):
        try:
            super().process_request_thread(request, client_address)
        finally:
            self.slots.release()


class Handler(socketserver.BaseRequestHandler):
    def handle(self):
        client = self.request
        client.settimeout(10)
        try:
            header = bytearray()
            while not header.endswith(b"\r\n\r\n"):
                if len(header) >= MAX_HEADER:
                    raise ValueError("oversized proxy request")
                byte = client.recv(1)
                if not byte:
                    return
                header.extend(byte)
            lines = header.decode("ascii").split("\r\n")
            parts = lines[0].split(" ")
            if len(parts) != 3 or parts[0] != "CONNECT" or parts[2] != "HTTP/1.1":
                raise ValueError("HTTPS CONNECT required")
            host, separator, port = parts[1].partition(":")
            if separator != ":" or port != "443" or host not in self.server.hosts:
                raise ValueError("destination denied")
            if not any(line.lower() == "host: " + parts[1].lower() for line in lines[1:-2]):
                raise ValueError("Host mismatch")
            addresses = public_addresses(host)
            remote = None
            for family, address in addresses:
                try:
                    remote = socket.socket(family, socket.SOCK_STREAM)
                    remote.settimeout(5)
                    remote.connect((address, 443))
                    break
                except OSError:
                    if remote is not None:
                        remote.close()
                    remote = None
            if remote is None:
                raise ValueError("destination unavailable")
            with remote:
                client.sendall(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                prefix = read_exact(client, 5)
                size = struct.unpack_from("!H", prefix, 3)[0]
                if size > MAX_TLS_RECORD:
                    raise ValueError("oversized TLS record")
                hello = prefix + read_exact(client, size)
                if client_hello_sni(hello) != host:
                    raise ValueError("SNI mismatch")
                remote.sendall(hello)
                tunnel(client, remote)
        except (OSError, UnicodeError, ValueError):
            try:
                client.sendall(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
            except OSError:
                pass


if __name__ == "__main__":
    policy = allowed_hosts(os.environ.get("AGENT_ALLOWED_HOSTS", ""))
    with Proxy(("0.0.0.0", 8080), Handler) as server:
        server.hosts = policy
        server.serve_forever(poll_interval=0.5)
