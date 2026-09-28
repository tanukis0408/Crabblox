"""Local DNS forwarder for Roblox only.

The shim sends Roblox's lookups to 127.0.0.1:<port> (MACOBLOX_DNS), and this
forwards them over DNS-over-TLS (or plain UDP for a custom server). Some
Roblox image hosts do not resolve through ISP or system resolvers in some
regions, and plain UDP DNS to public resolvers is often tampered with there;
the rest of the system keeps its own DNS."""

import socket
import ssl
import struct
import threading
import time
import urllib.request
from concurrent.futures import ThreadPoolExecutor

PROVIDERS = {
    "quad9": ("9.9.9.9", "dns.quad9.net"),
    "cloudflare": ("1.1.1.1", "cloudflare-dns.com"),
    "google": ("8.8.8.8", "dns.google"),
}


ROBLOX_EDGE_IPS = ["128.116.5.3", "128.116.21.3", "128.116.115.3"]


def _extract_qname_and_type(query):
    try:
        offset = 12
        labels = []
        while offset < len(query):
            length = query[offset]
            if length == 0:
                break
            offset += 1
            labels.append(query[offset:offset + length].decode(errors="ignore"))
            offset += length
        qname = ".".join(labels).lower()
        offset += 1
        if offset + 4 <= len(query):
            qtype, _ = struct.unpack(">HH", query[offset:offset + 4])
            return qname, qtype
        return qname, 1
    except Exception:
        return "", 1


def _make_a_response(query, ips):
    tx_id = query[:2]
    flags = b"\x81\x80"
    counts = struct.pack(">HHHH", 1, len(ips), 0, 0)
    offset = 12
    while offset < len(query) and query[offset] != 0:
        offset += query[offset] + 1
    offset += 1 + 4
    question = query[12:offset]
    answers = b""
    for ip in ips:
        answers += b"\xc0\x0c" + struct.pack(">HHIH", 1, 1, 300, 4) + socket.inet_aton(ip)
    return tx_id + flags + counts + question + answers


def _make_empty_response(query):
    tx_id = query[:2]
    flags = b"\x81\x80"
    counts = b"\x00\x01\x00\x00\x00\x00\x00\x00"
    offset = 12
    while offset < len(query) and query[offset] != 0:
        offset += query[offset] + 1
    offset += 1 + 4
    return tx_id + flags + counts + query[12:offset]


def _sanitize_response(response):
    """Rewrite any blocked Cloudflare IP addresses in answers for Roblox domains to direct edge IPs."""
    try:
        answers = struct.unpack(">H", response[6:8])[0]
        offset = 12

        def skip_name(pos):
            while pos < len(response):
                length = response[pos]
                if length == 0:
                    return pos + 1
                if (length & 0xC0) == 0xC0:
                    return pos + 2
                pos += length + 1
            return pos

        offset = skip_name(offset) + 4
        resp_bytes = bytearray(response)
        modified = False
        for _ in range(answers):
            offset = skip_name(offset)
            if offset + 10 > len(resp_bytes):
                break
            _type, _class, ttl, length = struct.unpack(">HHIH", resp_bytes[offset:offset + 10])
            offset += 10
            if _type == 1 and length == 4 and offset + 4 <= len(resp_bytes):
                ip = socket.inet_ntoa(resp_bytes[offset:offset + 4])
                if ip.startswith("104.") or ip.startswith("172.6") or ip.startswith("172.7"):
                    resp_bytes[offset:offset + 4] = socket.inet_aton("128.116.5.3")
                    modified = True
            offset += length
        return bytes(resp_bytes) if modified else response
    except Exception:
        return response


def _min_ttl(response):
    """Smallest TTL in the answer section, or None."""
    try:
        answers = struct.unpack(">H", response[6:8])[0]
        offset = 12

        def skip_name(position):
            while True:
                length = response[position]
                if length == 0:
                    return position + 1
                if length & 0xC0 == 0xC0:
                    return position + 2
                position += length + 1

        offset = skip_name(offset) + 4
        ttls = []
        for _ in range(answers):
            offset = skip_name(offset)
            _type, _class, ttl, length = struct.unpack(">HHIH", response[offset:offset + 10])
            ttls.append(ttl)
            offset += 10 + length
        return min(ttls) if ttls else None
    except (IndexError, struct.error):
        return None


class DnsForwarder:
    def __init__(self, provider, custom=""):
        if provider in PROVIDERS:
            self.server, self.tls_name = PROVIDERS[provider]
            self.port = 853
        else:
            host, _, port = custom.strip().partition(":")
            self.server, self.tls_name = host, None
            self.port = int(port or 53)
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.socket.bind(("127.0.0.1", 0))
        self.socket.settimeout(0.5)
        self.address = "127.0.0.1:%d" % self.socket.getsockname()[1]
        self.cache = {}
        self.cache_lock = threading.Lock()
        self.pool = []
        self.pool_lock = threading.Lock()
        self.context = ssl.create_default_context()
        self.executor = ThreadPoolExecutor(max_workers=8)
        self.running = True
        threading.Thread(target=self._serve, daemon=True).start()

    def stop(self):
        self.running = False
        self.executor.shutdown(wait=False)
        with self.pool_lock:
            for connection in self.pool:
                connection.close()
            self.pool.clear()

    def _serve(self):
        while self.running:
            try:
                query, client = self.socket.recvfrom(4096)
            except socket.timeout:
                continue
            except OSError:
                break
            if len(query) > 12:
                self.executor.submit(self._answer, query, client)
        self.socket.close()

    def _answer(self, query, client):
        qname, qtype = _extract_qname_and_type(query)
        if qname == "auth.roblox.com":
            if qtype == 1:
                response = _make_a_response(query, ROBLOX_EDGE_IPS)
            else:
                response = _make_empty_response(query)
            try:
                self.socket.sendto(response, client)
            except OSError:
                pass
            return

        key = query[2:]
        now = time.time()
        with self.cache_lock:
            cached = self.cache.get(key)
        if cached and cached[0] > now:
            response = query[:2] + cached[1]
        else:
            response = self._resolve(query)
            if not response:
                return
            response = _sanitize_response(response)
            ttl = _min_ttl(response)
            ttl = 30 if ttl is None else max(30, min(ttl, 600))
            with self.cache_lock:
                self.cache[key] = (now + ttl, response[2:])
        try:
            self.socket.sendto(response, client)
        except OSError:
            pass

    def _resolve_doh(self, query):
        if not self.tls_name:
            return None
        url = f"https://{self.tls_name}/dns-query"
        req = urllib.request.Request(
            url,
            data=query,
            headers={"Content-Type": "application/dns-message", "Accept": "application/dns-message", "User-Agent": "MacOBlox"}
        )
        try:
            with urllib.request.urlopen(req, timeout=3) as resp:
                data = resp.read()
                if len(data) >= 12 and data[:2] == query[:2]:
                    return data
        except Exception:
            pass
        return None

    def _resolve(self, query):
        if self.tls_name:
            res = self._resolve_tls(query)
            if res:
                return res
            return self._resolve_doh(query)
        return self._resolve_udp(query)

    def _resolve_udp(self, query):
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as upstream:
            upstream.settimeout(2)
            for _ in range(2):
                try:
                    upstream.sendto(query, (self.server, self.port))
                    response = upstream.recv(4096)
                    if response[:2] == query[:2]:
                        return response
                except OSError:
                    continue
        return None

    def _connect(self):
        raw = socket.create_connection((self.server, self.port), timeout=2)
        return self.context.wrap_socket(raw, server_hostname=self.tls_name)

    @staticmethod
    def _read_exact(connection, length):
        data = b""
        while len(data) < length:
            chunk = connection.recv(length - len(data))
            if not chunk:
                raise OSError("connection closed")
            data += chunk
        return data

    def _resolve_tls(self, query):
        for _ in range(2):
            with self.pool_lock:
                connection = self.pool.pop() if self.pool else None
            try:
                if connection is None:
                    connection = self._connect()
                connection.sendall(struct.pack(">H", len(query)) + query)
                length = struct.unpack(">H", self._read_exact(connection, 2))[0]
                response = self._read_exact(connection, length)
                with self.pool_lock:
                    if self.running and len(self.pool) < 4:
                        self.pool.append(connection)
                        connection = None
                if connection:
                    connection.close()
                return response
            except (OSError, ssl.SSLError, struct.error):
                if connection:
                    connection.close()
        return None


def resolve_a(host, provider="quad9", timeout=3):
    """IPv4 addresses of `host` from one DNS-over-TLS query (with DoH fallback), for the
    launcher's own downloads when the system resolver fails."""
    if host.lower() == "auth.roblox.com":
        return list(ROBLOX_EDGE_IPS)
    server, tls_name = PROVIDERS.get(provider, PROVIDERS["quad9"])
    query = struct.pack(">HHHHHH", 0x4d42, 0x0100, 1, 0, 0, 0)
    for label in host.rstrip(".").split("."):
        query += bytes([len(label)]) + label.encode()
    query += b"\0" + struct.pack(">HH", 1, 1)
    response = None
    try:
        raw = socket.create_connection((server, 853), timeout=timeout)
        with ssl.create_default_context().wrap_socket(raw, server_hostname=tls_name) as connection:
            connection.sendall(struct.pack(">H", len(query)) + query)
            length = struct.unpack(">H", DnsForwarder._read_exact(connection, 2))[0]
            response = DnsForwarder._read_exact(connection, length)
    except Exception:
        # Fall back to DNS-over-HTTPS (port 443) if port 853 is blocked
        url = f"https://{tls_name}/dns-query"
        req = urllib.request.Request(
            url,
            data=query,
            headers={"Content-Type": "application/dns-message", "Accept": "application/dns-message", "User-Agent": "MacOBlox"}
        )
        try:
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                data = resp.read()
                if len(data) >= 12 and data[:2] == query[:2]:
                    response = data
        except Exception:
            pass
    if not response:
        return []

    def skip_name(position):
        while True:
            length = response[position]
            if length == 0:
                return position + 1
            if length & 0xC0 == 0xC0:
                return position + 2
            position += length + 1

    answers = struct.unpack(">H", response[6:8])[0]
    offset = skip_name(12) + 4
    addresses = []
    for _ in range(answers):
        offset = skip_name(offset)
        kind, _class, _ttl, length = struct.unpack(">HHIH", response[offset:offset + 10])
        offset += 10
        if kind == 1 and length == 4:
            addresses.append(socket.inet_ntoa(response[offset:offset + 4]))
        offset += length

    if host.lower().endswith(".roblox.com") or host.lower() == "roblox.com":
        fixed = []
        for a in addresses:
            if a.startswith("104.") or a.startswith("172.6") or a.startswith("172.7"):
                fixed.append("128.116.5.3")
            else:
                fixed.append(a)
        if fixed:
            addresses = fixed
    return addresses
