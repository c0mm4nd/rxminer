#!/usr/bin/env python3
"""Tiny local HTTP CONNECT / SOCKS5 proxy for RxMiner TLS tests."""
from __future__ import annotations

import argparse
import selectors
import socket
import socketserver
import struct
import sys
import threading


def pump(a: socket.socket, b: socket.socket) -> None:
    sel = selectors.DefaultSelector()
    sel.register(a, selectors.EVENT_READ, b)
    sel.register(b, selectors.EVENT_READ, a)
    try:
        while True:
            events = sel.select(timeout=30)
            if not events:
                break
            for key, _ in events:
                src: socket.socket = key.fileobj
                dst: socket.socket = key.data
                data = src.recv(65536)
                if not data:
                    return
                dst.sendall(data)
    finally:
        sel.close()
        for s in (a, b):
            try:
                s.close()
            except OSError:
                pass


def read_until(sock: socket.socket, marker: bytes, limit: int = 65536) -> bytes:
    buf = b""
    while marker not in buf:
        chunk = sock.recv(4096)
        if not chunk:
            raise ConnectionError("eof")
        buf += chunk
        if len(buf) > limit:
            raise ConnectionError("header too large")
    return buf


class Handler(socketserver.BaseRequestHandler):
    mode = "http"

    def handle(self) -> None:
        conn: socket.socket = self.request
        try:
            if self.mode == "http":
                data = read_until(conn, b"\r\n\r\n")
                first = data.split(b"\r\n", 1)[0].decode(errors="replace")
                parts = first.split()
                if len(parts) < 2 or parts[0].upper() != "CONNECT":
                    conn.sendall(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n")
                    return
                host, port = parts[1].rsplit(":", 1)
                port = int(port)
            else:
                head = conn.recv(2)
                if len(head) != 2 or head[0] != 5:
                    return
                nmethods = head[1]
                methods = conn.recv(nmethods)
                if 0 not in methods:
                    conn.sendall(b"\x05\xff")
                    return
                conn.sendall(b"\x05\x00")
                req = conn.recv(4)
                if len(req) != 4 or req[0] != 5 or req[1] != 1:
                    return
                atyp = req[3]
                if atyp == 1:
                    host = socket.inet_ntoa(conn.recv(4))
                elif atyp == 4:
                    host = socket.inet_ntop(socket.AF_INET6, conn.recv(16))
                elif atyp == 3:
                    n = conn.recv(1)[0]
                    host = conn.recv(n).decode()
                else:
                    return
                port = struct.unpack("!H", conn.recv(2))[0]

            upstream = socket.create_connection((host, port), timeout=10)
            if self.mode == "http":
                conn.sendall(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            else:
                conn.sendall(b"\x05\x00\x00\x01\x00\x00\x00\x00\x00\x00")
            upstream.settimeout(None)
            conn.settimeout(None)
            pump(conn, upstream)
        except (OSError, ValueError, ConnectionError):
            try:
                conn.sendall(b"HTTP/1.1 502 Bad Gateway\r\n\r\n")
            except OSError:
                pass


class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--port", type=int, required=True)
    p.add_argument("--mode", choices=["http", "socks5"], default="http")
    a = p.parse_args()
    Handler.mode = a.mode
    with Server(("127.0.0.1", a.port), Handler) as srv:
        print(f"proxy {a.mode} listening on 127.0.0.1:{a.port}", flush=True)
        srv.serve_forever()
    return 0


if __name__ == "__main__":
    sys.exit(main())
