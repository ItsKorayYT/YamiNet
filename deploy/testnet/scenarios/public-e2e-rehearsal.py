#!/usr/bin/env python3
"""Exercise the existing testnet kit on loopback. No deployment or firewall changes."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[3]


def rehearse(binary_dir):
    suffix = '.exe' if os.name == 'nt' else ''
    binaries = {name: binary_dir / (name + suffix) for name in ('yami-testnet', 'yami-node', 'yami-relay', 'yami-browser')}
    checks = []
    children = []
    logs = []

    def result(name, status='PASS', **evidence):
        checks.append({'name': name, 'status': status, **evidence})

    with tempfile.TemporaryDirectory(prefix='Yami public e2e rehearsal ') as directory:
        base = Path(directory)
        profile = base / 'profile'
        env = os.environ.copy()
        env['YAMINET_BROWSER_DHT_NETWORK'] = 'local'
        env['YAMINET_BROWSER_DATA_DIR'] = str(base / 'browser-library')
        env['YAMINET_PEER_DIR'] = str(base / 'publisher-state')
        env['YAMINET_BROWSER_RELAYS'] = '0'

        def pace_trial(args):
            # Every simulated machine shares one source IP. Keep independent
            # scenarios out of the existing one-second TLS/relay admission
            # windows, especially on fast Windows runners. Do not relax limits
            # or retry a negative assertion until an unrelated failure passes.
            if '--network-trial' in args:
                time.sleep(1.1)

        def run(binary, *args, ok=True, timeout=45):
            pace_trial(args)
            completed = subprocess.run([str(binaries[binary]), *map(str, args)], env=env,
                                       capture_output=True, text=True, encoding='utf-8',
                                       errors='replace', timeout=timeout)
            if (completed.returncode == 0) != ok:
                raise RuntimeError(f'{binary} {args[0]}: unexpected exit {completed.returncode}: {completed.stderr}')
            return completed

        def start(binary, config, label):
            log = (base / (label + '.log')).open('w', encoding='utf-8')
            logs.append(log)
            process = subprocess.Popen([str(binaries[binary]), '--config', str(config)],
                                       env=env, stdout=log, stderr=log)
            children.append(process)
            return process

        def browser_config(path, state, bootstrap):
            path.write_text(f'yaminet-config 1\nrole browser\nnetwork loopback\nstate {state}\nbootstrap {bootstrap}\nrelay-fallback yes\n', encoding='utf-8')
            return path

        def step_status(report, name):
            return next((s['status'] for s in report['steps'] if s['name'] == name), None)

        try:
            # Reserve distinct local ports during offline profile generation.
            reservations = [socket.socket() for _ in range(3)]
            for reservation in reservations:
                reservation.bind(('127.0.0.1', 0))
            ports = [reservation.getsockname()[1] for reservation in reservations]
            try:
                run('yami-testnet', 'init', '--profile', profile, '--relay', '--site', '--client', '--yns',
                    '--binary-dir', binary_dir, '--bootstrap-port', ports[0], '--relay-port', ports[1], '--site-port', ports[2])
            finally:
                for reservation in reservations:
                    reservation.close()
            public = dict(re.findall(r"^([A-Z_]+)='([^']*)'$", (profile / 'addresses.env').read_text(encoding='utf-8'), re.M))
            bootstrap_pin = f"{public['BOOTSTRAP_ID']}@{public['BOOTSTRAP_ENDPOINT']}"
            provider_pin = public['YNS_PROVIDER_PIN']
            host_id = public['HOST_ID']
            url = public['HOST_URL']
            client = profile / 'client/client.conf'
            host = profile / 'host/host.conf'
            site = profile / 'host/site'
            config = (profile / 'bootstrap/bootstrap.conf').read_text(encoding='utf-8')
            assert 'yns yes' in config
            assert 'listen ' not in host.read_text(encoding='utf-8')
            assert 'advertise ' not in host.read_text(encoding='utf-8')
            result('offline-profile-and-topology', listener='loopback bootstrap/provider and relay; host has no listener')
            # Stage only a reviewed local Wasm example; players receive it in memory.
            games = site / 'games'
            games.mkdir()
            wasm = (ROOT / 'games/breakout/breakout.wasm').read_bytes()
            local = json.loads((ROOT / 'games/breakout/game.yami-game.json').read_text(encoding='utf-8'))
            manifest = {k: local[k] for k in ('version', 'abi', 'title', 'description')}
            manifest['module'] = {'key': 'prism', 'bytes': len(wasm), 'sha256': hashlib.sha256(wasm).hexdigest()}
            (games / 'prism.json').write_text(json.dumps(manifest), encoding='utf-8')
            module = games / 'prism.wasm'
            module.write_bytes(wasm)
            homepage = site / 'home.ylang'
            homepage.write_text(homepage.read_text(encoding='utf-8') + '\n> game:prism | Play Prism Breakout\n', encoding='utf-8')
            run('yami-node', 'site', 'check', site)
            bootstrap = start('yami-node', profile / 'bootstrap/bootstrap.conf', 'bootstrap')
            relay = start('yami-relay', profile / 'relay/relay.conf', 'relay')
            start('yami-node', host, 'outbound-host')
            preflight_args = ['preflight', '--config', client, '--url', url, '--asset', 'cover', '--route', 'relay', '--timeout', '5', '--json']
            ready = None
            deadline = time.monotonic() + 35
            while time.monotonic() < deadline:
                assert all(process.poll() is None for process in children)
                attempt = subprocess.run([str(binaries['yami-testnet']), *map(str, preflight_args)], env=env,
                                         capture_output=True, text=True, timeout=12)
                if attempt.returncode == 0:
                    ready = json.loads(attempt.stdout)
                    break
                time.sleep(0.2)
            if ready is None:
                raise RuntimeError('bootstrap/relay/outbound host did not become ready')
            result('dht-pinned-relay-page-and-png', observed=ready['result'])
            direct = json.loads(run('yami-testnet', 'preflight', '--config', client, '--url', url,
                                    '--route', 'direct', '--timeout', '5', '--json', ok=False).stdout)
            assert any(check['id'] == 'route-selection' and check['status'] == 'FAIL' for check in direct['checks'])
            result('outbound-host-direct-route-refused', expected='FAIL at route-selection', observed=direct['result'])

            owner = base / 'offline-owner'
            namespace = run('yami-browser', 'yns', 'namespace-create', owner).stdout.splitlines()[0]
            name = 'trial.' + namespace

            def publish(path, sequence, expires):
                now = int(time.time())
                record = run('yami-browser', 'yns', 'record-sign', owner, namespace, path, sequence, now, expires, 'map', host_id).stdout
                file = base / ('signed-' + path + '.json')
                file.write_text(record, encoding='utf-8')
                run('yami-browser', 'yns', 'publish', path + '.' + namespace, provider_pin, file)

            publish('trial', 1, int(time.time()) + 3600)
            run('yami-browser', '--config', client, 'yns', 'providers-set', provider_pin)
            cached = json.loads((profile / 'client/state/yns/yns.json').read_text(encoding='utf-8'))
            assert not cached['records']
            trial_args = ['--network-trial', url, '--config', client, '--asset', 'cover', '--name', name, '--navigate', 'about', '--game', 'prism']
            report = json.loads(run('yami-browser', *trial_args).stdout)
            for check in ('yns-resolution', 'signed-publication-resolution', 'document-and-png', 'page-navigation', 'hosted-game'):
                assert step_status(report, check) == 'PASS', (check, report)
                result(check, evidence='native browser trial; game execution explicitly requested')
            assert report['gui_rendering'] == 'NOT RUN'

            # A TLS pin mismatch must reach the identity check, not just fail parsing.
            wrong = run('yami-browser', '--probe', f"ynps://{host_id}@{public['BOOTSTRAP_ENDPOINT']}/home", ok=False)
            assert 'not the trusted node' in wrong.stdout and 'Nothing was requested' in wrong.stdout and host_id in wrong.stdout and public['BOOTSTRAP_ID'] in wrong.stdout, wrong.stdout
            result('wrong-destination-identity', expected='TLS identity mismatch before document retrieval')
            wrong_config = browser_config(base / 'wrong-provider.conf', base / 'wrong-provider', bootstrap_pin)
            run('yami-browser', '--config', wrong_config, 'yns', 'providers-set', f"{host_id}@{public['BOOTSTRAP_ENDPOINT']}")
            run('yami-browser', '--config', wrong_config, 'yns', 'resolve', name, ok=False)
            assert not json.loads((base / 'wrong-provider/yns/yns.json').read_text(encoding='utf-8'))['records']
            result('wrong-provider-identity', observed='no record accepted into fresh cache')
            wrong_bootstrap = browser_config(base / 'wrong-bootstrap.conf', base / 'wrong-bootstrap', f"{host_id}@{public['BOOTSTRAP_ENDPOINT']}")
            denied = json.loads(run('yami-browser', '--network-trial', url, '--config', wrong_bootstrap, '--asset', 'cover', ok=False).stdout)
            assert step_status(denied, 'bootstrap-pin') == 'FAIL', denied
            result('wrong-bootstrap-identity', expected='FAIL at bootstrap-pin')
            invalid_link = json.loads(run('yami-browser', '--network-trial', url, '--config', client, '--asset', 'cover', '--navigate', 'not-linked', ok=False).stdout)
            assert step_status(invalid_link, 'page-navigation') == 'FAIL', invalid_link
            result('unadvertised-navigation-refused')
            module.write_bytes(wasm[:-1] + bytes([wasm[-1] ^ 1]))
            try:
                bad_game = json.loads(run('yami-browser', *trial_args, ok=False).stdout)
                assert step_status(bad_game, 'hosted-game') == 'FAIL', bad_game
            finally:
                module.write_bytes(wasm)
            result('hosted-module-integrity-refusal', expected='FAIL before game execution')

            expiry = int(time.time()) + 20
            publish('short', 1, expiry)
            short = 'short.' + namespace
            assert run('yami-browser', '--config', client, 'yns', 'resolve', short).stdout.strip() == host_id
            time.sleep(max(0, expiry - time.time() + 1))
            expired = json.loads(run('yami-browser', '--network-trial', url, '--config', client, '--asset', 'cover', '--name', short, ok=False).stdout)
            assert step_status(expired, 'yns-resolution') == 'FAIL', expired
            publish('short', 2, int(time.time()) + 3600)
            assert run('yami-browser', '--config', client, 'yns', 'refresh', short).stdout.strip() == host_id
            result('expired-yns-record-and-higher-sequence-recovery', expected='expired cached record refused, signed sequence 2 accepted')

            relay.terminate()
            relay.wait(timeout=10)
            outage = json.loads(run('yami-browser', '--network-trial', url, '--config', client, '--asset', 'cover', ok=False).stdout)
            assert any(s['name'] in ('signed-publication-resolution', 'document-and-png') and s['status'] == 'FAIL' for s in outage['steps']), outage
            result('relay-outage', expected='authenticated delivery fails without plaintext fallback')
            start('yami-relay', profile / 'relay/relay.conf', 'relay-restarted')
            recovered = None
            deadline = time.monotonic() + 35
            while time.monotonic() < deadline:
                pace_trial(trial_args)
                attempt = subprocess.run([str(binaries['yami-browser']), *map(str, trial_args)], env=env,
                                         capture_output=True, text=True, timeout=25)
                if attempt.returncode == 0:
                    recovered = json.loads(attempt.stdout)
                    break
                time.sleep(0.2)
            assert recovered is not None, 'relay/host did not reconnect with existing identities and history'
            assert step_status(recovered, 'hosted-game') == 'PASS'
            result('relay-recovery-including-hosted-game')
            bootstrap.terminate()
            bootstrap.wait(timeout=10)
            # Cached YNS cannot make the fresh DHT-availability check pass.
            unavailable = json.loads(run('yami-browser', '--network-trial', url, '--config', client, '--asset', 'cover', ok=False).stdout)
            assert step_status(unavailable, 'bootstrap-pin') == 'FAIL', unavailable
            result('unavailable-bootstrap', expected='FAIL despite previously cached records')
            for check in ('real-vps-deployment', 'home-router-outbound-path', 'independent-internet-client', 'public-firewall-state', 'native-gui-click-and-keyboard'):
                result(check, 'NOT RUN', reason='local rehearsal does not establish physical topology or operator authorization')
        except Exception as error:
            result('rehearsal', 'FAIL', reason=f'{type(error).__name__}: {error}')
        finally:
            for process in children:
                if process.poll() is None:
                    process.kill()
                process.wait(timeout=10)
            for log in logs:
                log.close()
    return {'schema': 'yaminet-public-e2e-rehearsal-1', 'result': 'FAIL' if any(c['status'] == 'FAIL' for c in checks) else 'PASS',
            'scope': 'independent processes on isolated IPv4 loopback; simulation only',
            'public_internet_result': 'NOT RUN', 'physical_topology': 'NOT VERIFIED',
            'infrastructure_changes': 'NONE', 'checks': checks}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary-dir', type=Path, default=ROOT / 'target/debug')
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()
    report = rehearse(args.binary_dir.resolve())
    text = json.dumps(report, indent=2) + '\n'
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(text, encoding='utf-8')
    print(text, end='')
    raise SystemExit(1 if report['result'] == 'FAIL' else 0)
