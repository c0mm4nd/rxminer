#!/usr/bin/env python3
"""Minimal TLS Stratum pool for local miner integration tests.

This is intentionally a *test* pool: it never contacts a real network and it
never validates proof-of-work.  It exists so miners can be exercised end to end
with TLS enabled, without touching a production pool or spending real value.
"""
from __future__ import annotations

import argparse
import json
import os
import socket
import ssl
import subprocess
import sys
import threading
import time
from dataclasses import dataclass, field

HEX = "0123456789abcdef"


def logger(msg: str) -> None:
    print(f"[pool {time.strftime('%H:%M:%S')}] {msg}", flush=True)


def ensure_cert(cert: str, key: str) -> None:
    if os.path.exists(cert) and os.path.exists(key):
        valid = subprocess.run(
            ["openssl", "x509", "-checkend", "60", "-noout", "-in", cert],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        if valid.returncode == 0:
            return
    os.makedirs(os.path.dirname(os.path.abspath(cert)), exist_ok=True)
    subprocess.run(
        [
            "openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
            "-keyout", key, "-out", cert, "-days", "2",
            "-subj", "/CN=localhost",
            "-addext", "subjectAltName=DNS:localhost,IP:127.0.0.1",
        ],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


@dataclass
class Stats:
    connections: int = 0
    logins: int = 0
    submits: int = 0
    accepted: int = 0
    rejected: int = 0
    lock: threading.Lock = field(default_factory=threading.Lock)

    def bump(self, name: str) -> None:
        with self.lock:
            setattr(self, name, getattr(self, name) + 1)


class Pool:
    def __init__(self, host: str, port: int, seed: str, target: str, job_id: str,
                 deny_prefix: str = "", plain: bool = False, nonce: int = 0) -> None:
        self.host = host
        self.port = port
        self.seed = seed
        self.target = target
        self.job_id = job_id
        self.deny_prefix = deny_prefix
        self.plain = plain
        self.nonce = nonce
        self.stats = Stats()

    def make_job(self) -> dict:
        # 76-byte CryptoNote-style hashing blob.  Nonce lives at offset 39.
        blob = bytearray(os.urandom(76))
        blob[0] = 0x0C
        blob[39:43] = self.nonce.to_bytes(4, "little")
        return {
            "blob": blob.hex(),
            "job_id": self.job_id,
            "target": self.target,
            "height": 3_500_000,
            "seed_hash": self.seed,
            "algo": "rx/0",
        }

    def handle(self, conn: ssl.SSLSocket, addr) -> None:
        self.stats.bump("connections")
        logger(f"connection from {addr[0]}:{addr[1]} mode={'TCP' if self.plain else 'TLS'}")
        buf = b""
        job = self.make_job()
        login = "?"
        try:
            while True:
                data = conn.recv(65536)
                if not data:
                    break
                buf += data
                while b"\n" in buf:
                    line, buf = buf.split(b"\n", 1)
                    if not line.strip():
                        continue
                    try:
                        req = json.loads(line)
                    except json.JSONDecodeError:
                        logger(f"bad json: {line[:200]!r}")
                        continue
                    login = self.dispatch(conn, req, job, login)
        except (ConnectionError, ssl.SSLError, OSError) as exc:
            logger(f"connection closed: {exc}")
        finally:
            try:
                conn.close()
            except OSError:
                pass

    def send(self, conn: ssl.SSLSocket, obj: dict) -> None:
        conn.sendall((json.dumps(obj, separators=(",", ":")) + "\n").encode())

    def dispatch(self, conn: ssl.SSLSocket, req: dict, job: dict, login: str) -> str:
        method = req.get("method")
        rid = req.get("id")
        if method == "login":
            self.stats.bump("logins")
            params = req.get("params", {})
            login = params.get("login", "?")
            logger(f"login user={login!r} agent={params.get('agent', '?')!r}")
            if self.deny_prefix and str(login).startswith(self.deny_prefix):
                logger(f"rejecting login matching deny prefix {self.deny_prefix!r}")
                conn.close()
                return login
            self.send(conn, {
                "id": rid,
                "jsonrpc": "2.0",
                "error": None,
                "result": {
                    "id": "rxminer-test-rig",
                    "job": job,
                    "status": "OK",
                    "extensions": ["algo", "nicehash", "keepalive"],
                },
            })
            return login
        elif method == "keepalived":
            logger(f"keepalive user={login!r}")
            self.send(conn, {"id": rid, "jsonrpc": "2.0", "error": None,
                             "result": {"status": "KEEPALIVED"}})
            return login
        elif method == "submit":
            self.stats.bump("submits")
            params = req.get("params", {})
            if params.get("job_id") != self.job_id:
                self.stats.bump("rejected")
                self.send(conn, {"id": rid, "jsonrpc": "2.0",
                                 "error": {"code": -1, "message": "stale job"},
                                 "result": None})
                return login
            self.stats.bump("accepted")
            logger(f"share accepted wallet={login} job={self.job_id} nonce={params.get('nonce')}")
            self.send(conn, {"id": rid, "jsonrpc": "2.0", "error": None,
                             "result": {"status": "OK"}})
            return login
        else:
            self.send(conn, {"id": rid, "jsonrpc": "2.0", "error": None,
                             "result": {"status": "OK"}})
            return login

    def serve(self, duration: float | None) -> None:
        ctx = None
        if not self.plain:
            ensure_cert("tests/certs/pool.crt", "tests/certs/pool.key")
            ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            ctx.minimum_version = ssl.TLSVersion.TLSv1_2
            ctx.load_cert_chain("tests/certs/pool.crt", "tests/certs/pool.key")
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as srv:
            srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            srv.bind((self.host, self.port))
            srv.listen(64)
            srv.settimeout(1.0)
            logger(f"listening on {self.host}:{self.port} ({'TCP' if self.plain else 'TLS'}, target={self.target})")
            start = time.time()
            threads: list[threading.Thread] = []
            try:
                while duration is None or time.time() - start < duration:
                    try:
                        raw, addr = srv.accept()
                    except socket.timeout:
                        continue
                    if ctx is None:
                        conn = raw
                    else:
                        try:
                            conn = ctx.wrap_socket(raw, server_side=True)
                        except ssl.SSLError as exc:
                            logger(f"TLS handshake failed: {exc}")
                            raw.close()
                            continue
                    t = threading.Thread(target=self.handle, args=(conn, addr), daemon=True)
                    t.start()
                    threads.append(t)
            except KeyboardInterrupt:
                pass
            finally:
                for t in threads:
                    t.join(timeout=0.2)
                logger(
                    "stats: connections={connections} logins={logins} submits={submits} "
                    "accepted={accepted} rejected={rejected}".format(**self.stats.__dict__)
                )


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--host", default="127.0.0.1")
    p.add_argument("--port", type=int, default=34443)
    p.add_argument("--duration", type=float, default=None, help="seconds; default run forever")
    p.add_argument("--seed", default="52584d696e65722d746573742d736565642d30303030303030303030303030303030")
    p.add_argument("--target", default="ffffffffffff0000")
    p.add_argument("--job-id", default="rxminer-test-job")
    p.add_argument("--deny-prefix", default="")
    p.add_argument("--plain", action="store_true")
    p.add_argument("--nonce", type=lambda n: int(n, 0), default=0)
    a = p.parse_args()
    # Normalise/validate the seed used by RandomX (32 bytes).
    seed = a.seed.strip()
    if len(seed) != 64:
        seed = (seed.encode().hex() + "00" * 32)[:64]
    Pool(a.host, a.port, seed, a.target, a.job_id, a.deny_prefix, a.plain, a.nonce).serve(a.duration)
    return 0


if __name__ == "__main__":
    sys.exit(main())
