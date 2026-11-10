import json
import os
import pathlib
import socket
import ssl
import subprocess
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
from yamichain_rpc import Handler, Server, Transport, endpoint, secret


class Echo(Handler):
    def handle_request(self, value):
        raw = json.dumps({'value': value, 'ok': True}).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)


class RpcTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.tmp.name)
        self.key, self.cert = self.root/'key.pem', self.root/'cert.pem'
        subprocess.run(['openssl','req','-x509','-newkey','rsa:2048','-nodes','-days','1',
                        '-keyout',str(self.key),'-out',str(self.cert),'-subj','/CN=localhost',
                        '-addext','subjectAltName=DNS:localhost,IP:127.0.0.1'],check=True,capture_output=True)
        self.key.chmod(0o600)
        self.tokens = {}
        for role in ['gateway','executor','validator']:
            path = self.root/(role+'.token');path.write_text(os.urandom(32).hex());path.chmod(0o600)
            self.tokens[role+'_file'] = str(path)
        self.config = dict(ca_file=str(self.cert),certificate_file=str(self.cert),key_file=str(self.key),tokens=self.tokens)
        # Reserve the actual listening socket first; no racy free-port probing.
        self.server = Server(('127.0.0.1',0), Echo)
        self.url = 'https://127.0.0.1:'+str(self.server.server_port)
        self.transport = Transport(self.config,{'gateway':[self.url]})
        self.server.context=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        self.server.context.load_cert_chain(self.cert,self.key)
        self.server.token=secret(self.tokens['gateway_file'])
        self.thread=threading.Thread(target=self.server.serve_forever,daemon=True);self.thread.start()

    def tearDown(self):
        self.server.shutdown();self.server.server_close();self.thread.join();self.tmp.cleanup()

    def test_tls_authenticated_request_and_invalid_identity(self):
        self.assertEqual(self.transport.request(self.url,{'encrypted':'opaque'}),{'ok':True,'value':{'encrypted':'opaque'}})
        # A valid executor credential must not authenticate to the gateway.
        req=urllib.request.Request(self.url,headers={'Authorization':'Bearer '+secret(self.tokens['executor_file'])})
        with self.assertRaises(urllib.error.HTTPError) as e:
            urllib.request.urlopen(req,context=ssl.create_default_context(cafile=str(self.cert)))
        self.assertEqual(e.exception.code,401)
        with self.assertRaises(urllib.error.HTTPError) as e:
            urllib.request.urlopen(self.url,context=ssl.create_default_context(cafile=str(self.cert)))
        self.assertEqual(e.exception.code,401)
        with self.assertRaises(urllib.error.URLError):
            urllib.request.urlopen(self.url,timeout=3)

    def test_remote_plaintext_unknown_origins_and_private_secret_permissions(self):
        for url in ['http://10.1.2.3:8000','http://example.com:80','https://user:password@localhost:80']:
            with self.assertRaises(ValueError):endpoint(url)
        with self.assertRaises(ValueError):self.transport.request('https://localhost:1')
        token=pathlib.Path(self.tokens['gateway_file']);token.chmod(0o644)
        with self.assertRaises(ValueError):self.transport.request(self.url)
        token.chmod(0o600)
        link=self.root/'link';link.symlink_to(token)
        with self.assertRaises(OSError):secret(link)

    def test_bounded_admission_and_duplicate_length_rejection(self):
        for i in range(2000):self.server.admitted(str(i))
        self.assertEqual(len(self.server.clients),1024)
        self.assertFalse(all(self.server.admitted('same') for _ in range(121)))
        ctx=ssl.create_default_context(cafile=str(self.cert))
        with socket.create_connection(('127.0.0.1',self.server.server_port)) as raw:
            with ctx.wrap_socket(raw,server_hostname='localhost') as s:
                token=secret(self.tokens['gateway_file'])
                s.sendall(('POST / HTTP/1.0\r\nAuthorization: Bearer '+token+'\r\nContent-Length: 2\r\nContent-Length: 3\r\n\r\n{}').encode())
                self.assertIn(b'400',s.recv(1024).split(b'\r\n')[0])

    def test_wallet_config_needs_only_its_own_gateway_token(self):
        cfg=dict(ca_file=str(self.cert),tokens={'gateway_file':self.tokens['gateway_file']})
        client=Transport(cfg,{'gateway':[self.url],'executor':['https://127.0.0.1:1']})
        self.assertTrue(client.request(self.url)['ok'])

    def test_handshake_flood_is_bounded_and_does_not_starve_authenticated_client(self):
        connections=[socket.create_connection(('127.0.0.1',self.server.server_port)) for _ in range(3)]
        try:self.assertTrue(self.transport.request(self.url,timeout=3)['ok'])
        finally:
            for c in connections:c.close()

    def test_redirects_do_not_forward_credentials(self):
        class Redirect(Echo):
            def handle_request(self,value):
                self.send_response(302);self.send_header('Location','https://localhost:1');self.end_headers()
        old=self.server.RequestHandlerClass;self.server.RequestHandlerClass=Redirect
        try:
            with self.assertRaises(urllib.error.HTTPError) as e:self.transport.request(self.url)
            self.assertEqual(e.exception.code,302)
        finally:self.server.RequestHandlerClass=old


if __name__=='__main__':unittest.main()
