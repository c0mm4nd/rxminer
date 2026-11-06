#!/usr/bin/env python3
"""Local-only Rust miner regression: TLS, proxies, shares, API and shutdown.

Build with --features test-dev-fee. Each mining case allocates one 2080 MiB
dataset; allow at least 4 GiB free memory. No external pools are contacted.
"""
import argparse
import contextlib
import hashlib
import json
import os
from pathlib import Path
import signal
import socket
import ssl
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parent.parent


def port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def wait_until(predicate, timeout=90):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.1)
    raise AssertionError("timed out waiting for test condition")


@contextlib.contextmanager
def process(args, log):
    with open(log, "w") as output:
        child = subprocess.Popen(args, cwd=ROOT, stdout=output, stderr=subprocess.STDOUT)
        try:
            yield child
        except BaseException:
            # Temporary logs are deleted when the test exits. Preserve useful
            # startup and failure context in CI without dumping every share.
            output.flush()
            lines = log.read_text(errors="replace").splitlines()
            excerpt = lines if len(lines) <= 80 else lines[:40] + ["..."] + lines[-40:]
            print(f"Failure log ({log.name}):\n" + "\n".join(excerpt), flush=True)
            raise
        finally:
            if child.poll() is None:
                child.send_signal(signal.SIGINT)
                try:
                    child.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()


def ready_log(path, marker, child):
    if child.poll() is not None:
        raise AssertionError(f"process exited early:\n{path.read_text()}")
    return marker in path.read_text()


def test_cpus():
    seen = set()
    selected = []
    for cpu in sorted(os.sched_getaffinity(0)):
        siblings = Path(f"/sys/devices/system/cpu/cpu{cpu}/topology/thread_siblings_list")
        key = siblings.read_text().strip() if siblings.exists() else str(cpu)
        if key not in seen:
            selected.append(cpu)
            seen.add(key)
        if len(selected) == 8:
            break
    return ",".join(map(str, selected))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/test-dev/release/rxminer")
    args = parser.parse_args()
    binary = str((ROOT / args.binary).resolve())
    cpus = test_cpus()
    # Disable urllib's environment proxy handling for loopback API requests.
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with tempfile.TemporaryDirectory(prefix="rxminer-integration-") as temp:
        temp = Path(temp)
        for mode in ("direct-tls", "http-tls", "socks5h-tls", "plain"):
            pool_port, api_port = port(), port()
            pool_log, miner_log = temp / f"{mode}-pool.log", temp / f"{mode}-miner.log"
            pool_args = ["python3", "tests/tls_pool.py", "--port", str(pool_port), "--target", "ffffffffffffffff"]
            if mode == "plain":
                pool_args.append("--plain")
            with contextlib.ExitStack() as stack:
                pool = stack.enter_context(process(pool_args, pool_log))
                wait_until(lambda: ready_log(pool_log, "listening on", pool), 10)
                flags = ["--no-proxy"]
                if mode in ("http-tls", "socks5h-tls"):
                    proxy_port = port()
                    proxy_mode = "http" if mode == "http-tls" else "socks5"
                    proxy_log = temp / f"{mode}-proxy.log"
                    proxy = stack.enter_context(process(["python3", "tests/tls_proxy.py", "--port", str(proxy_port), "--mode", proxy_mode], proxy_log))
                    wait_until(lambda: ready_log(proxy_log, "listening on", proxy), 10)
                    flags += ["--proxy", f"http://127.0.0.1:{proxy_port}" if mode == "http-tls" else f"127.0.0.1:{proxy_port}"]
                scheme = "stratum+tcp" if mode == "plain" else "stratum+ssl"
                if mode != "plain":
                    cert = ssl.PEM_cert_to_DER_cert((ROOT / "tests/certs/pool.crt").read_text())
                    flags += ["--tls-ca", "tests/certs/pool.crt", "--tls-fingerprint", hashlib.sha256(cert).hexdigest()]
                command = ["taskset", "-c", cpus, binary, "-o", f"{scheme}://127.0.0.1:{pool_port}",
                           "-u", "test-wallet", "-t", "2", "--randomx-no-numa", "--http-port", str(api_port)] + flags
                api_token = "integration-token"
                # Exercise the canonical JSON layout and authenticated API through a real connection.
                if mode == "direct-tls":
                    configuration = temp / "config.json"
                    configuration.write_text(json.dumps({
                        "pools": [{"url": f"127.0.0.1:{pool_port}", "user": "test-wallet",
                                   "tls": True, "tls-ca": "tests/certs/pool.crt",
                                   "tls-fingerprint": hashlib.sha256(cert).hexdigest()}],
                        "cpu": {"rx": [int(cpu) for cpu in cpus.split(",")[:2]], "huge-pages": False},
                        "randomx": {"numa": False, "init": 2},
                        "http": {"enabled": True, "host": "127.0.0.1", "port": api_port,
                                 "access-token": api_token, "restricted": True},
                        "print-time": 0,
                    }))
                    command = ["taskset", "-c", cpus, binary, "-c", str(configuration), "--no-proxy"]
                else:
                    command += ["--no-huge-pages", "--http-access-token", api_token]
                api = urllib.request.build_opener(urllib.request.ProxyHandler({}))
                api.addheaders = [("Authorization", f"Bearer {api_token}")]
                miner = stack.enter_context(process(command, miner_log))

                def summary():
                    if miner.poll() is not None:
                        raise AssertionError(f"miner exited:\n{miner_log.read_text()}")
                    try:
                        with api.open(f"http://127.0.0.1:{api_port}/summary", timeout=2) as response:
                            return json.load(response)
                    except (OSError, urllib.error.URLError):
                        return None

                def shares_ready():
                    data = summary()
                    return data if data and data["shares"]["accepted"] > 0 and data["dev_shares"]["accepted"] > 0 else None

                data = wait_until(shares_ready)
                assert data["threads"] == 2 and data["nodes"] == 1, data
                assert data["dev_fee"]["percent"] == "<1%", data
                try:
                    opener.open(f"http://127.0.0.1:{api_port}/summary", timeout=2)
                    raise AssertionError("API must reject missing access token")
                except urllib.error.HTTPError as error:
                    assert error.code == 401
                if mode == "direct-tls":
                    # Cross a second fee window and verify that the returning
                    # user stream resumes nonce work instead of repeating it.
                    time.sleep(6)
                    data = summary()
                    for endpoint in ("/", "/hashrates", "/shares"):
                        with api.open(f"http://127.0.0.1:{api_port}{endpoint}", timeout=2) as response:
                            assert isinstance(json.load(response), dict)
                    try:
                        api.open(f"http://127.0.0.1:{api_port}/missing", timeout=2)
                        raise AssertionError("unknown endpoint must return 404")
                    except urllib.error.HTTPError as error:
                        assert error.code == 404
                    accepted = [line for line in pool_log.read_text().splitlines() if "share accepted wallet=" in line]
                    # User/developer shares use independent TCP connections.
                    # Their arrival order cannot establish which fee phase ran
                    # first; the scheduler unit test checks that invariant.
                    # Require both streams and unique nonces across fee windows.
                    wallets = {line.split("wallet=", 1)[1].split()[0] for line in accepted}
                    assert wallets == {"test-wallet", data["dev_fee"]["address"]}, wallets
                    unique = set()
                    for line in accepted:
                        payload = line.split("wallet=", 1)[1]
                        wallet = payload.split()[0]
                        nonce = payload.split("nonce=", 1)[1]
                        assert (wallet, nonce) not in unique, "duplicate nonce across fee windows"
                        unique.add((wallet, nonce))
                if mode == "plain":
                    # A dropped mandatory developer connection must be fatal,
                    # including when it happens during the user window.
                    pool.kill()
                    pool.wait(timeout=5)
                    assert miner.wait(timeout=10) != 0, miner_log.read_text()
                else:
                    miner.send_signal(signal.SIGINT)
                    assert miner.wait(timeout=15) == 0, miner_log.read_text()
                print(f"PASS {mode}: user={data['shares']['accepted']} dev={data['dev_shares']['accepted']}", flush=True)

        for name, flags, deny in (
            ("untrusted-cert", ["--tls-verify"], False),
            ("wrong-pin", ["--tls-fingerprint", "00" * 32], False),
            ("denied-dev", [], True),
        ):
            pool_port = port()
            pool_log, miner_log = temp / f"{name}-pool.log", temp / f"{name}-miner.log"
            pool_args = ["python3", "tests/tls_pool.py", "--port", str(pool_port)]
            if deny:
                pool_args += ["--deny-prefix", "46fca3"]
            with process(pool_args, pool_log) as pool:
                wait_until(lambda: ready_log(pool_log, "listening on", pool), 10)
                with process([binary, "-o", f"stratum+ssl://127.0.0.1:{pool_port}", "-u", "test-wallet", "-t", "2", "--no-proxy"] + flags, miner_log) as miner:
                    assert miner.wait(timeout=25) != 0, miner_log.read_text()
                    assert "developer connection unavailable" in miner_log.read_text(), miner_log.read_text()
            print(f"PASS {name}", flush=True)
    print("All local integration checks passed", flush=True)


if __name__ == "__main__":
    main()
