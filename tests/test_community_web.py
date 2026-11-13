"""Gateway trust boundaries; no ALSA, Bonjour, HomePod or external service."""
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
from pathlib import Path
import threading
import unittest

spec = importlib.util.spec_from_file_location('web_gateway', Path(__file__).resolve().parents[1]/'scripts/serve-web.py')
gateway = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gateway)

class Upstream(BaseHTTPRequestHandler):
    calls = []
    def log_message(self, *_):
        pass
    def do_GET(self):
        self.calls.append((self.path, self.headers.get('Authorization')))
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b'{"running":false}')

class GatewayTests(unittest.TestCase):
    def setUp(self):
        Upstream.calls = []
        self.upstream = ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
        self.server = ThreadingHTTPServer(('127.0.0.1', 0), gateway.Handler)
        self.server.allowed_hosts = {f'127.0.0.1:{self.server.server_port}'}
        self.server.controller_port = self.upstream.server_port
        self.server.token = 'test-token-not-a-real-secret'
        self.threads = [threading.Thread(target=s.serve_forever, daemon=True) for s in (self.upstream, self.server)]
        for thread in self.threads:
            thread.start()

    def tearDown(self):
        for server in (self.server, self.upstream):
            server.shutdown()
            server.server_close()
        for thread in self.threads:
            thread.join(timeout=2)

    def request(self, path, method='GET', headers=None, body=None):
        connection = http.client.HTTPConnection('127.0.0.1', self.server.server_port, timeout=3)
        try:
            connection.request(method, path, body, headers or {})
            response = connection.getresponse()
            return response.status, response.read()
        finally:
            connection.close()

    def test_token_stays_on_server(self):
        code, body = self.request('/api/state')
        self.assertEqual(code, 200)
        self.assertNotIn(self.server.token.encode(), body)
        self.assertEqual(Upstream.calls, [('/api/state', 'Bearer '+self.server.token)])

    def test_dns_rebinding_host_is_rejected(self):
        self.assertEqual(self.request('/api/state', headers={'Host': 'attacker.example'})[0], 403)
        self.assertEqual(Upstream.calls, [])

    def test_only_known_routes_are_forwarded(self):
        for path in ('/api/video', '/api/anything', '/../../LICENSE'):
            self.assertEqual(self.request(path)[0], 404)
        self.assertEqual(Upstream.calls, [])

    def test_cross_origin_and_oversized_writes_are_rejected(self):
        self.assertEqual(self.request('/api/config', 'POST', body='{}')[0], 403)
        headers = {'X-Vinyl-Control': '1', 'Origin': 'http://attacker.example'}
        self.assertEqual(self.request('/api/config', 'POST', headers, '{}')[0], 403)
        self.assertEqual(self.request('/api/config', 'POST', {'X-Vinyl-Control': '1'}, 'x'*4097)[0], 400)
        self.assertEqual(Upstream.calls, [])

    def test_static_page_does_not_contain_controller_token(self):
        code, body = self.request('/')
        self.assertEqual(code, 200)
        self.assertNotIn(self.server.token.encode(), body)
        self.assertNotIn(b'videoTarget', body)

if __name__ == '__main__':
    unittest.main()
