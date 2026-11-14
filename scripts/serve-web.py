#!/usr/bin/env python3
"""Local web gateway and controller launcher. No third-party Python packages."""
import argparse
import http.client
import ipaddress
import os
from pathlib import Path
import secrets
import signal
import socket
import subprocess
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
READ_ROUTES = {'/api/state', '/api/meter', '/api/artwork', '/api/inputs'}
WRITE_ROUTES = {'/api/config', '/api/volume'}
STATIC = {'/': ('index.html', 'text/html; charset=utf-8'), '/app.js': ('app.js', 'text/javascript; charset=utf-8'), '/style.css': ('style.css', 'text/css; charset=utf-8')}

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, code, data=b'', content_type='application/json'):
        self.send_response(code)
        self.send_header('Content-Type', content_type)
        self.send_header('Content-Length', str(len(data)))
        self.send_header('Cache-Control', 'no-store')
        self.send_header('X-Content-Type-Options', 'nosniff')
        self.send_header('Referrer-Policy', 'no-referrer')
        self.send_header('Content-Security-Policy', "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' https://statici.icloud.com; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'")
        self.end_headers()
        self.wfile.write(data)

    def handle_request(self, write=False):
        self.connection.settimeout(20)
        if self.headers.get('Host') not in self.server.allowed_hosts:
            self.reply(403, b'{"error":"Host not allowed"}')
            return
        if not ipaddress.ip_address(self.client_address[0]).is_private:
            self.reply(403, b'{"error":"Local network only"}')
            return
        path = urlsplit(self.path).path
        if len(self.path) > 1024:
            self.reply(414)
            return
        if not write and path in STATIC:
            filename, mime = STATIC[path]
            self.reply(200, (ROOT/'web'/filename).read_bytes(), mime)
            return
        if path not in (WRITE_ROUTES if write else READ_ROUTES):
            self.reply(404, b'{"error":"Not found"}')
            return
        body = None
        if write:
            origin = self.headers.get('Origin')
            if self.headers.get('X-Vinyl-Control') != '1' or (origin and origin != 'http://'+self.headers.get('Host', '')):
                self.reply(403, b'{"error":"Invalid control request"}')
                return
            try:
                size = int(self.headers.get('Content-Length', '0'))
                if not 0 < size <= 4096 or self.headers.get('Transfer-Encoding'):
                    raise ValueError()
            except ValueError:
                self.reply(400, b'{"error":"Invalid body size"}')
                return
            body = self.rfile.read(size)
            if len(body) != size:
                self.reply(400)
                return
        connection = http.client.HTTPConnection('127.0.0.1', self.server.controller_port, timeout=15)
        try:
            connection.request('POST' if write else 'GET', self.path, body=body, headers={
                'Authorization': 'Bearer '+self.server.token,
                'X-Vinyl-Control': '1', 'Content-Type': 'application/json'})
            response = connection.getresponse()
            data = response.read(1024*1024+1)
            if len(data) > 1024*1024:
                raise ValueError('Upstream response too large')
            self.reply(response.status, data, response.getheader('Content-Type', 'application/json'))
        except (OSError, ValueError, http.client.HTTPException):
            self.reply(502, b'{"error":"Controller unavailable"}')
        finally:
            connection.close()

    def do_GET(self):
        self.handle_request()

    def do_POST(self):
        self.handle_request(True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    inputs = parser.add_mutually_exclusive_group()
    inputs.add_argument('--input-device', default=None, help='Native input ID from audio-inputs; default input if omitted')
    inputs.add_argument('--alsa-device', help='Legacy Linux ALSA input, as listed by arecord -L')
    parser.add_argument('--listen', default='127.0.0.1', help='Private IPv4 to listen on; loopback by default')
    parser.add_argument('--port', type=int, default=8787)
    parser.add_argument('--state', type=Path, default=Path.home()/'.local/state/vinyl-airplay')
    parser.add_argument('--controller', type=Path, default=ROOT/('target/release/vinyl-control'+('.exe' if os.name == 'nt' else '')))
    parser.add_argument('--sender', type=Path, default=ROOT/('target/release/vinyl-airplay'+('.exe' if os.name == 'nt' else '')))
    args = parser.parse_args()
    try:
        address = ipaddress.IPv4Address(args.listen)
        if not (address.is_private and not address.is_unspecified) or not 1024 <= args.port <= 65535:
            raise ValueError()
    except ValueError:
        parser.error('Use a specific private IPv4 and a port from 1024 to 65535.')
    for binary in (args.controller, args.sender):
        if not binary.is_file() or not os.access(binary, os.X_OK):
            parser.error(f'Build the binaries first: cargo build --release --locked ({binary})')
    os.umask(0o077)
    state = args.state.expanduser().resolve()
    (state/'control').mkdir(parents=True, exist_ok=True)
    token_path = state/'control/token'
    if not token_path.exists():
        token_path.write_text(secrets.token_hex(32))
    token_path.chmod(0o600)
    token = token_path.read_text().strip()
    if len(token) < 32:
        parser.error('The existing controller token is invalid.')
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        controller_port = sock.getsockname()[1]
    server = ThreadingHTTPServer((args.listen, args.port), Handler)
    server.daemon_threads = True
    server.controller_port = controller_port
    server.token = token
    server.allowed_hosts = {f'{args.listen}:{args.port}'}
    if address.is_loopback:
        server.allowed_hosts.add(f'localhost:{args.port}')
    env = {**os.environ, 'VINYL_ROOT': str(state), 'VINYL_TOKEN_FILE': str(token_path),
           'VINYL_BIND': f'127.0.0.1:{controller_port}',
           'VINYL_SENDER_BIN': str(args.sender.resolve())}
    env.pop('VINYL_ALSA_DEVICE', None)
    env.pop('VINYL_INPUT_DEVICE', None)
    if args.input_device:
        env['VINYL_INPUT_DEVICE'] = args.input_device
    if args.alsa_device:
        env['VINYL_ALSA_DEVICE'] = args.alsa_device
    child = subprocess.Popen([str(args.controller.resolve())], env=env,
                             creationflags=subprocess.CREATE_NEW_PROCESS_GROUP if os.name == 'nt' else 0)
    signal.signal(signal.SIGTERM, lambda *_: (_ for _ in ()).throw(KeyboardInterrupt()))
    print(f'Open http://{args.listen}:{args.port}/ — playback starts disabled.', flush=True)
    if not address.is_loopback:
        print('Trusted LAN users can control playback. Do not expose this port to the Internet.', flush=True)
    try:
        server.serve_forever(poll_interval=.25)
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
        child.send_signal(signal.CTRL_BREAK_EVENT if os.name == 'nt' else signal.SIGINT)
        try:
            child.wait(timeout=15)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait()

if __name__ == '__main__':
    main()
