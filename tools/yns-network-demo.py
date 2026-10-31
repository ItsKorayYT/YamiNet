#!/usr/bin/env python3
"""Independent process YNS demonstration. Loopback simulation, not internet proof."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def run_demo(node, browser):
    with tempfile.TemporaryDirectory(prefix='Yami YNS network demo ') as directory:
        root = Path(directory)
        processes = []
        logs = []

        def profile(name):
            env = os.environ.copy()
            env['YAMINET_PEER_DIR'] = str(root / name)
            env['YAMINET_BROWSER_DHT_NETWORK'] = 'local'
            env['YAMINET_BROWSER_DATA_DIR'] = str(root / (name + '-library'))
            env['YAMINET_BROWSER_RELAYS'] = '0'
            return env

        def browser_run(name, *args, succeeds=True):
            result = subprocess.run([str(browser), *map(str, args)], env=profile(name),
                                    capture_output=True, text=True, encoding='utf-8',
                                    errors='replace', timeout=20)
            if (result.returncode == 0) != succeeds:
                raise RuntimeError(f'browser command failed: {args[0]}: {result.stderr}')
            return result.stdout.strip(), result.stderr

        site = root / 'site'
        site.mkdir()
        (site / 'home.ylang').write_text('@page 0.1\n  title Distributed YNS demo\n= Distributed YNS demo\n', encoding='utf-8')
        (site / 'default.ysl').write_text('', encoding='utf-8')

        def server(name, peer=None):
            log = (root / (name + '.log')).open('w+', encoding='utf-8')
            logs.append(log)
            args = [str(node), 'serve-dir', '127.0.0.1:0', str(site), '--identity', str(root / (name + '-identity')),
                    '--tls', '--dht', '--dht-local', '--dht-dir', str(root / (name + '-dht')),
                    '--yns-dir', str(root / (name + '-records'))]
            if peer:
                args.extend(['--yns-peer', peer])
            process = subprocess.Popen(args, env=profile(name), stdout=log, stderr=log)
            processes.append(process)
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                log.flush()
                text = Path(log.name).read_text(encoding='utf-8')
                match = re.search(r'Serving site .* via YNP over TLS 1.3 on (127\.0\.0\.1:\d+) as (yn1-[a-z2-7]+)', text)
                if match:
                    return process, match[2], match[1]
                if process.poll() is not None:
                    raise RuntimeError('mirror process failed: ' + text)
                time.sleep(0.05)
            raise RuntimeError('mirror startup timed out')

        try:
            namespace = browser_run('publisher', 'yns', 'namespace-create', root / 'offline-owner')[0].splitlines()[0]
            owner_id = namespace.removesuffix('.yns')
            mirror, mirror_id, mirror_endpoint = server('mirror')
            seed, destination, seed_endpoint = server('seed', f'{mirror_id}@{mirror_endpoint}')
            assert owner_id not in (mirror_id, destination)
            name = f'blog.{namespace}'
            now = int(time.time())
            record = browser_run('publisher', 'yns', 'record-sign', root / 'offline-owner', namespace,
                                 'blog', 1, now, now + 3600, 'map', destination)[0]
            record_file = root / 'publisher-record.json'
            record_file.write_text(record, encoding='utf-8')
            # Neither server receives a path to or loads the owner key directory.
            shutil.move(root / 'offline-owner', root / 'owner-offline-after-signing')
            browser_run('publisher', 'yns', 'publish', name, f'{mirror_id}@{mirror_endpoint}', record_file)
            # Receiving profile receives only a seed identity/endpoint, no JSON records.
            browser_run('fresh-browser', 'yns', 'providers-set', f'{destination}@{seed_endpoint}')
            state_path = root / 'fresh-browser' / 'yns' / 'yns.json'
            assert not json.loads(state_path.read_text(encoding='utf-8'))['records']
            resolved = browser_run('fresh-browser', 'yns', 'resolve', name)[0]
            assert resolved == destination
            # The existing DHT independently locates the final authenticated page node.
            page = browser_run('fresh-browser', '--bootstrap', f'{destination}@{seed_endpoint}',
                               '--terminal', f'yami://{name}/home')[0]
            assert 'Distributed YNS demo' in page
            # An independent process copies the signed chain, with no owner key.
            browser_run('republisher', 'yns', 'mirror', name, f'{mirror_id}@{mirror_endpoint}', f'{destination}@{seed_endpoint}')
            mirror.terminate()
            mirror.wait(timeout=10)
            browser_run('second-fresh-browser', 'yns', 'providers-set', f'{destination}@{seed_endpoint}')
            assert browser_run('second-fresh-browser', 'yns', 'resolve', name)[0] == destination
            browser_run('fresh-browser', 'yns', 'alias-set', 'demo', destination)
            assert browser_run('fresh-browser', 'yns', 'resolve', '@demo')[0] == destination
            seed.terminate()
            seed.wait(timeout=10)
            assert browser_run('fresh-browser', 'yns', 'resolve', name)[0] == destination
            assert browser_run('fresh-browser', 'yns', 'resolve', '@demo')[0] == destination
            assert browser_run('no-bootstrap', 'yns', 'resolve', name, succeeds=False)[1].find('No YNS providers configured') >= 0
            return {'schema': 'yaminet-yns-network-demo-1', 'result': 'PASS',
                    'scope': 'isolated IPv4 loopback, independent native processes',
                    'real_internet_validation': 'NOT VERIFIED', 'owner_separate_from_servers': True,
                    'publisher_to_mirror': 'PASS', 'fresh_browser_via_referral': 'PASS',
                    'dht_and_destination_pinned_page': 'PASS', 'independent_mirror_copy': 'PASS',
                    'offline_validated_cache': 'PASS', 'aliases': 'PASS',
                    'without_bootstrap': 'remote naming fails explicitly', 'manual_receiver_imports': 0}
        finally:
            for process in processes:
                if process.poll() is None:
                    process.kill()
                process.wait(timeout=10)
            for log in logs:
                log.close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    suffix = '.exe' if os.name == 'nt' else ''
    parser.add_argument('--node', type=Path, default=ROOT / 'target/debug' / ('yami-node' + suffix))
    parser.add_argument('--browser', type=Path, default=ROOT / 'target/debug' / ('yami-browser' + suffix))
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()
    report = run_demo(args.node.resolve(), args.browser.resolve())
    text = json.dumps(report, indent=2) + '\n'
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(text, encoding='utf-8')
    print(text, end='')
