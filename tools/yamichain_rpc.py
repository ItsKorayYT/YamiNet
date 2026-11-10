"""Bounded TLS/authenticated RPC for the disposable native-chain runner.

No financial authority is granted by RPC access. Signed private operations and
native finality continue to be checked by the existing Rust implementation.
"""
import collections
import hmac
import ipaddress
import json
import os
import pathlib
import socket
import ssl
import stat
import threading
import time
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer
from socketserver import ThreadingMixIn


def secret(path):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_mode & 0o077:
            raise ValueError('RPC secret must be a private regular file (mode 0600)')
        value = os.read(fd, 4097)
        if not 32 <= len(value.strip()) <= 4096 or len(value) > 4096:
            raise ValueError('RPC token must contain 32..4096 ASCII characters')
        value = value.decode('ascii').strip()
        if any(not 33 <= ord(c) <= 126 for c in value):
            raise ValueError('RPC token cannot contain whitespace')
        return value
    finally:
        os.close(fd)


def endpoint(url):
    u = urllib.parse.urlsplit(url)
    if (u.scheme not in ('http', 'https') or not u.hostname or not u.port
            or u.username or u.password or u.query or u.fragment):
        raise ValueError('RPC endpoint requires an explicit port and no URL credentials')
    # Plaintext compatibility is strictly loopback, including resolution-free names.
    local = u.hostname == 'localhost'
    try:
        local |= ipaddress.ip_address(u.hostname).is_loopback
    except ValueError:
        pass
    if u.scheme == 'http' and not local:
        raise ValueError('non-loopback RPC requires HTTPS and authentication')
    return u


class Transport:
    def __init__(self, config, services):
        self.config = config or {}
        self.services = {}
        for role, urls in services.items():
            for url in urls:
                u = endpoint(url)
                origin = (u.scheme, u.hostname, u.port)
                if origin in self.services:
                    raise ValueError('RPC service origins must be distinct')
                self.services[origin] = role
                if u.scheme == 'https':
                    if not self.config.get('ca_file'):
                        raise ValueError('HTTPS RPC requires an explicitly configured CA file')

    def token(self, role):
        return secret(self.config['tokens'][role + '_file'])

    def request(self, url, value=None, timeout=90):
        u = endpoint(url)
        role = self.services.get((u.scheme, u.hostname, u.port))
        if role is None:
            raise ValueError('RPC target is not a configured service origin')
        headers = {'Content-Type': 'application/json'}
        context = None
        if u.scheme == 'https':
            context = ssl.create_default_context(cafile=self.config['ca_file'])
            context.minimum_version = ssl.TLSVersion.TLSv1_2
            headers['Authorization'] = 'Bearer ' + self.token(role)
        req = urllib.request.Request(url, data=None if value is None else json.dumps(value).encode(), headers=headers)
        # Redirects could disclose a role credential to an unrelated service.
        class NoRedirect(urllib.request.HTTPRedirectHandler):
            def redirect_request(self, *args, **kwargs):
                return None
        opener = urllib.request.build_opener(NoRedirect(), urllib.request.HTTPSHandler(context=context))
        with opener.open(req, timeout=timeout) as response:
            raw = response.read(16 * 1024 * 1024 + 1)
            if len(raw) > 16 * 1024 * 1024:
                raise ValueError('RPC response byte limit')
            return json.loads(raw)

    def server(self, url, handler):
        u = endpoint(url)
        role = self.services[(u.scheme, u.hostname, u.port)]
        context = None
        token = None
        if u.scheme == 'https':
            token = self.token(role)
            key = self.config['key_file']
            # Validate key permissions without reading/logging private key material.
            info = pathlib.Path(key).lstat()
            if not stat.S_ISREG(info.st_mode) or info.st_mode & 0o077:
                raise ValueError('RPC TLS key must be a private regular file (mode 0600)')
            context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            context.minimum_version = ssl.TLSVersion.TLSv1_2
            context.load_cert_chain(self.config['certificate_file'], key)
        server = Server((u.hostname, u.port), handler)
        server.context, server.token = context, token
        return server


class Server(ThreadingMixIn, HTTPServer):
    daemon_threads = True
    request_queue_size = 16

    def __init__(self, *args):
        self.slots = threading.BoundedSemaphore(8)
        self.service_lock = threading.Lock()
        self.admission_lock = threading.Lock()
        self.clients = collections.OrderedDict()
        self.context = self.token = None
        super().__init__(*args)

    def get_request(self):
        connection, address = super().get_request()
        connection.settimeout(5)
        if self.context:
            try:
                connection = self.context.wrap_socket(connection, server_side=True, do_handshake_on_connect=False)
            except Exception:
                connection.close()
                raise
        return connection, address

    def process_request(self, request, address):
        if not self.slots.acquire(blocking=False):
            request.close()
            return
        try:
            super().process_request(request, address)
        except Exception:
            self.slots.release()
            request.close()
            raise

    def process_request_thread(self, request, address):
        try:
            if isinstance(request, ssl.SSLSocket):
                request.do_handshake()
            super().process_request_thread(request, address)
        except (OSError, ssl.SSLError):
            request.close()
        finally:
            self.slots.release()

    def admitted(self, address):
        now = time.monotonic()
        with self.admission_lock:
            # Bounded IP state. Evict oldest entries, never grow with attacker identities.
            last, tokens = self.clients.pop(address, (now, 120.0))
            tokens = min(120.0, tokens + (now - last) * 60)
            allowed = tokens >= 1
            self.clients[address] = (now, max(0, tokens - 1))
            while len(self.clients) > 1024:
                self.clients.popitem(last=False)
            return allowed

    def handle_error(self, request, address):
        # Never log request bodies, Authorization headers or peer-supplied exceptions.
        pass


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def admit(self):
        if not self.server.admitted(self.client_address[0]):
            self.send_error(429)
            return False
        if len(self.path) > 2048 or sum(len(k) + len(v) for k, v in self.headers.items()) > 8192:
            self.send_error(431)
            return False
        if self.server.token:
            supplied = self.headers.get_all('Authorization', [])
            expected = 'Bearer ' + self.server.token
            if len(supplied) != 1 or not hmac.compare_digest(supplied[0].encode(), expected.encode()):
                self.send_error(401)
                return False
        return True

    def do_GET(self):
        if self.admit():
            with self.server.service_lock:
                self.handle_request(None)

    def do_POST(self):
        if not self.admit():
            return
        lengths = self.headers.get_all('Content-Length', [])
        if len(lengths) != 1 or self.headers.get('Transfer-Encoding') is not None:
            self.send_error(400)
            return
        try:
            n = int(lengths[0])
            if not 0 < n <= 1024 * 1024:
                self.send_error(413)
                return
            raw = self.rfile.read(n)
            if len(raw) != n:
                raise ValueError('truncated body')
            value = json.loads(raw)
        except (ValueError, OSError):
            self.send_error(400)
            return
        with self.server.service_lock:
            self.handle_request(value)
