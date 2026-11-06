#!/usr/bin/env python3
"""Local light-mode regression for failover, reload, pause, TLS API and shutdown.
Build the miner with --features test-dev-fee. No public pool is contacted.
"""
import argparse
import contextlib
import json
import os
import subprocess
from pathlib import Path
import signal
import ssl
import tempfile
import time
import urllib.error
import urllib.request
from integration import ROOT, port, process, ready_log, wait_until
from tls_pool import ensure_cert


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default='target/release/rxminer')
    args = parser.parse_args()
    binary = str((ROOT / args.binary).resolve())
    with tempfile.TemporaryDirectory(prefix='rxminer-runtime-') as directory:
        root = Path(directory)
        pool_port, dead_port, api_port = port(), port(), port()
        config = root / 'config.json'
        output, pool_log, log_file = [root / name for name in ['miner.log', 'pool.log', 'configured.log']]
        cert, key = root / 'server.crt', root / 'server.key'
        ensure_cert(str(cert), str(key))
        data = {
            'pools': [{'url': f'127.0.0.1:{p}', 'user': 'runtime-test', 'nicehash': True, 'keepalive': 1}
                      for p in [dead_port, pool_port]],
            'randomx': {'mode': 'light', 'numa': False, 'wrmsr': False, 'scratchpad_prefetch_mode': 2},
            'cpu': {'rx': [-1], 'huge-pages': False, 'memory-pool': True, 'priority': 1},
            'http': {'enabled': True, 'port': api_port, 'access-token': 'local-token', 'restricted': False},
            'tls': {'enabled': True, 'cert': str(cert), 'cert_key': str(key), 'protocols': 'TLSv1.2 TLSv1.3'},
            'retries': 2, 'retry-pause': 0, 'user-agent': 'runtime-agent', 'print-time': 0,
            'health-print-time': 1, 'autosave': False, 'watch': True, 'log-file': str(log_file),
            'dns': {'ip_version': 4, 'ttl': 1},
        }
        config.write_text(json.dumps(data))
        tls = ssl.create_default_context(cafile=str(cert))
        api = urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPSHandler(context=tls))

        def request(path='/summary', method='GET', body=None, token=True):
            headers = {'Authorization': 'Bearer local-token'} if token else {}
            content = None if body is None else json.dumps(body).encode()
            with api.open(urllib.request.Request(f'https://localhost:{api_port}{path}', data=content, headers=headers, method=method), timeout=3) as response:
                return json.load(response)

        def summary():
            try:
                return request()
            except (OSError, urllib.error.URLError):
                return None

        def ready(threads):
            value = summary()
            return value if value and value['threads'] == threads and value['shares']['accepted'] and value['dev_shares']['accepted'] else None

        with contextlib.ExitStack() as stack:
            pool = stack.enter_context(process(['python3', 'tests/tls_pool.py', '--plain', '--port', str(pool_port),
                '--target', 'ffffffffffffffff', '--nonce', '0xabfffffe'], pool_log))
            wait_until(lambda: ready_log(pool_log, 'listening on', pool), 10)
            miner = stack.enter_context(process([binary, '-c', str(config), '--no-proxy'], output))
            wait_until(lambda: ready(1), 30)
            assert 'switching to pool 2' in output.read_text()
            wait_until(lambda: 'keepalive user=' in pool_log.read_text(), 5)
            assert 'runtime-agent' in pool_log.read_text()
            wait_until(lambda: '[health]' in log_file.read_text(), 5)
            for line in pool_log.read_text().splitlines():
                if 'share accepted' in line:
                    assert line.rsplit('nonce=', 1)[1].endswith('ab'), line
            print('PASS backup pool, NiceHash wrap, keepalive, user-agent, light mining, logging', flush=True)

            for method, path, body in [('POST', '/json_rpc', {'id': 1, 'method': 'stop'}), ('GET', '/1/config', None)]:
                try:
                    request(path, method, body, token=False)
                    raise AssertionError('authentication bypass')
                except urllib.error.HTTPError as error:
                    assert error.code == 401
            before = config.read_bytes()
            try:
                request('/1/config', 'PUT', {'cpu': {'rx': [-9]}})
                raise AssertionError('invalid configuration accepted')
            except urllib.error.HTTPError as error:
                assert error.code == 400
            assert config.read_bytes() == before
            assert request('/json_rpc', 'POST', {'id': 2, 'method': 'pause'})['result']['status'] == 'OK'
            time.sleep(0.5)
            frozen = request()['hashes']
            time.sleep(0.5)
            assert request()['hashes'] == frozen
            assert request('/2/summary')['paused']
            assert request('/2/summary')['hashrate']['total'] == [0, 0, 0]
            request('/json_rpc', 'POST', {'id': 3, 'method': 'resume'})
            wait_until(lambda: request()['hashes'] > frozen, 5)
            print('PASS authenticated HTTPS API, invalid PUT, pause/resume', flush=True)

            config.write_text('{ invalid JSON')
            wait_until(lambda: 'reload rejected' in output.read_text(), 5)
            assert ready(1)
            data['cpu']['rx'] = [-1, -1]
            data['watch'] = False
            config.write_text(json.dumps(data))
            wait_until(lambda: ready(2), 30)
            assert request('/1/config')['cpu']['rx'] == [-1, -1]
            data['cpu']['rx'] = [-1]
            request('/1/config', 'PUT', data)
            wait_until(lambda: ready(1), 30)
            request('/json_rpc', 'POST', {'id': 4, 'method': 'stop'})
            assert miner.wait(timeout=10) == 0, output.read_text()
            print('PASS invalid reload keeps workers, valid reload replaces workers, API reload with watch disabled, stop', flush=True)
        generated = root / 'generated.json'
        background_log = root / 'background.log'
        generated.write_text(json.dumps({
            'cpu': {'max-threads-hint': 1, 'huge-pages': False},
            'randomx': {'mode': 'light', 'numa': False, 'wrmsr': False},
            'benchmark': {'seconds': 1}, 'background': True,
            'log-file': str(background_log), 'autosave': True,
        }))
        cpus = ','.join(str(cpu) for cpu in sorted(os.sched_getaffinity(0))[:2])
        subprocess.run(['taskset', '-c', cpus, binary, '-c', str(generated)], cwd=ROOT, check=True, timeout=10)
        wait_until(lambda: background_log.exists() and ' [final]' in background_log.read_text(), 30)
        assert len(json.loads(generated.read_text())['cpu']['rx']) == 1
        assert 'restarting workers' not in background_log.read_text()
        print('PASS background completion and automatic profile persistence', flush=True)
    print('All local runtime checks passed', flush=True)


if __name__ == '__main__':
    main()
