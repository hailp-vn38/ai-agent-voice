import socket
import struct
import threading
import unittest
from unittest.mock import patch

from proxy import Handler, Proxy, allowed_hosts, client_hello_sni, public_addresses


def hello(host):
    name = host.encode("ascii")
    server_name = b"\x00" + struct.pack("!H", len(name)) + name
    names = struct.pack("!H", len(server_name)) + server_name
    extension = struct.pack("!HH", 0, len(names)) + names
    body = b"\x03\x03" + bytes(32) + b"\x00" + b"\x00\x02\x13\x01" + b"\x01\x00"
    body += struct.pack("!H", len(extension)) + extension
    handshake = b"\x01" + len(body).to_bytes(3, "big") + body
    return b"\x16\x03\x01" + struct.pack("!H", len(handshake)) + handshake


class ProxyPolicyTests(unittest.TestCase):
    def test_exact_host_policy_and_tls_sni(self):
        self.assertEqual(allowed_hosts("example.com,api.example.com"), {"example.com", "api.example.com"})
        for invalid in ("", "EXAMPLE.com", "example.com,", "bad..example", ".example.com"):
            with self.assertRaises(ValueError):
                allowed_hosts(invalid)
        self.assertEqual(client_hello_sni(hello("example.com")), "example.com")
        with self.assertRaises(ValueError):
            client_hello_sni(hello("example.com")[:-1])

    def test_private_or_mixed_dns_answer_is_denied(self):
        with patch("proxy.socket.getaddrinfo", return_value=[
            (socket.AF_INET, socket.SOCK_STREAM, 6, "", ("8.8.8.8", 443)),
            (socket.AF_INET, socket.SOCK_STREAM, 6, "", ("169.254.169.254", 443)),
        ]):
            with self.assertRaisesRegex(ValueError, "non-public"):
                public_addresses("example.com")

    def test_proxy_denies_unapproved_connect_and_plain_http(self):
        with Proxy(("127.0.0.1", 0), Handler) as server:
            server.hosts = frozenset({"example.com"})
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            for request in (
                b"CONNECT metadata.google.internal:443 HTTP/1.1\r\nHost: metadata.google.internal:443\r\n\r\n",
                b"GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\n\r\n",
            ):
                with socket.create_connection(server.server_address, timeout=2) as client:
                    client.sendall(request)
                    self.assertIn(b"403 Forbidden", client.recv(100))
            server.shutdown()
            thread.join(timeout=2)


if __name__ == "__main__":
    unittest.main()
