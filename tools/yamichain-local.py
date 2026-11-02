#!/usr/bin/env python3
"""Isolated real-process Malachite rehearsal; no public networking or funds."""
import argparse
import hashlib
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time


def free_ports(count=4):
    for _ in range(100):
        sockets = []
        try:
            first = socket.socket()
            first.bind(('127.0.0.1', 0))
            base = first.getsockname()[1]
            sockets.append(first)
            if base > 65436:
                continue
            for port in range(base + 1, base + count):
                sock = socket.socket()
                sockets.append(sock)
                sock.bind(('127.0.0.1', port))
            return base
        except OSError:
            pass
        finally:
            for sock in sockets:
                sock.close()
    raise RuntimeError('cannot find four loopback ports')


def scenario(binary, root, name, active=4, crash=None, timeout=45, target=6, plan="1:4"):
    directory = root / name
    directory.mkdir()
    base = free_ports(max(active,4))
    processes = {}
    logs = []
    started = time.monotonic()

    def launch(index, fault=None):
        log = open(directory / f'node-{index}-{len(logs)}.log', 'w', encoding='utf-8')
        logs.append(log)
        cmd = [str(binary), 'run', '--dir', str(directory / f'node-{index}'),
               '--index', str(index), '--base-port', str(base), '--height', str(target),
               '--timeout', str(timeout), '--committee-plan',plan, '--linger','8']
        if fault:
            cmd += ['--crash-after', fault]
        processes[index] = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT)

    try:
        late_index = 3 if name == 'catchup' else 6 if name == 'rotating-catchup' else None
        for index in range(late_index if late_index is not None else active):
            launch(index, crash if index == 0 else None)
        injected = False
        late_started = late_index is None
        deadline = time.monotonic() + timeout + 15
        while time.monotonic() < deadline:
            if not late_started:
                lines = (directory / 'node-0-0.log').read_text(encoding='utf-8').splitlines()
                heights = [json.loads(line)['height'] for line in lines if line.startswith('{') and line.endswith('}')]
                if heights and max(heights) >= (25 if name == 'catchup' else 9):
                    launch(late_index)
                    late_started = True
            if crash and not injected and processes[0].poll() is not None:
                if processes[0].returncode != 86:
                    raise RuntimeError(f'{name}: injection failed (exit {processes[0].returncode})')
                injected = True
                launch(0, crash)  # The durable marker prevents repeated injection.
            if late_started and all(p.poll() is not None for p in processes.values()):
                break
            time.sleep(0.05)
        else:
            raise RuntimeError(f'{name}: process deadline exceeded')
        codes = {i: p.returncode for i, p in processes.items()}
        if active >= 3 and any(c != 0 for c in codes.values()):
            raise RuntimeError(f'{name}: nonzero exits {codes}')
        if active < 3 and any(c != 1 for c in codes.values()):
            raise RuntimeError(f'{name}: expected quorum deadline exits {codes}')
        if crash and not injected:
            raise RuntimeError(f'{name}: crash point never reached')
        states = []
        for index in range(active):
            text = subprocess.check_output([str(binary), 'inspect', '--dir',
                                           str(directory / f'node-{index}'), '--committee-plan',plan], text=True)
            states.append(json.loads(text))
        synced_payloads = sum(p.read_text(encoding='utf-8').count('validated synced payload') for p in directory.glob('*.log'))
        served_history = sum(p.read_text(encoding='utf-8').count('served certified history') for p in directory.glob('*.log'))
        if name in ('catchup', 'rotating-catchup'):
            assert synced_payloads > 0 and served_history > 0, 'late validator did not exercise certified value sync'
        expected_height = target if active >= 3 else 0
        assert all(s['height'] == expected_height for s in states), states
        assert len({(s['block'], s['state_root']) for s in states}) == 1, states
        assert all(not s['payments_enabled'] for s in states), states
        # A subsequent independent process must replay exactly the same history.
        again = json.loads(subprocess.check_output([str(binary), 'status', '--dir',
                                                   str(directory / 'node-0'), '--committee-plan',plan], text=True))
        assert again == states[0]
        return {'result': 'PASS', 'processes': active, 'committee_plan': plan,
                'crash': crash, 'injected_exit_86': injected, 'exit_codes': codes,
                'validated_synced_payloads': synced_payloads, 'certified_history_responses': served_history,
                'height': expected_height, 'state_root': states[0]['state_root'],
                'block': states[0]['block'], 'elapsed_seconds': round(time.monotonic()-started, 3)}
    finally:
        for proc in processes.values():
            if proc.poll() is None:
                proc.kill()
            proc.wait()
        for log in logs:
            log.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/debug/yami-validator.exe' if os.name == 'nt' else 'target/debug/yami-validator'))
    parser.add_argument('--report', type=Path, default=Path('work/yamichain-local.json'))
    parser.add_argument('--scenario', choices=['all', 'normal', 'offline', 'halt', 'catchup', 'proposal', 'vote', 'before-commit', 'seven', 'rotation', 'epoch', 'rotating-catchup'], default='all')
    args = parser.parse_args()
    binary = args.binary.resolve()
    args.report.parent.mkdir(parents=True, exist_ok=True)
    report = {'scope': 'loopback only; public dev keys; no real money',
              'generated_at': datetime.now(timezone.utc).isoformat(),
              'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'driver_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              'scenarios': {}}
    cases = [('normal', 4, None, 60, 6, '1:4'), ('offline', 3, None, 60, 6, '1:4'),
             ('halt', 2, None, 8, 1, '1:4'), ('catchup', 4, None, 180, 40, '1:4'),
             ('proposal', 4, 'proposal', 60, 8, '1:4'), ('vote', 4, 'vote', 60, 8, '1:4'),
             ('before-commit', 4, 'before-commit', 60, 8, '1:4'),
             ('seven', 7, None, 90, 5, '1:7'), ('rotation', 7, None, 120, 10, '1:4,4:7,7:5'),
             ('epoch', 7, 'epoch', 120, 10, '1:4,4:7,7:5'),
             ('rotating-catchup', 7, None, 180, 12, '1:4,4:7,8:5')]
    with tempfile.TemporaryDirectory(prefix='yamichain-', dir=args.report.parent) as temp:
        for name, active, crash, timeout, target, plan in cases:
            if args.scenario not in ('all', name):
                continue
            try:
                result = scenario(binary, Path(temp), name, active, crash, timeout, target, plan)
                report['scenarios'][name] = result
                print(f'{name}: PASS at height {result["height"]}', flush=True)
            except Exception as error:
                report['scenarios'][name] = {'result': 'FAIL', 'reason': str(error)}
                # Preserve bounded diagnostic logs in the report, including failures.
                report['logs'] = {str(p.relative_to(temp)): p.read_text(encoding='utf-8')[-24000:]
                                  for p in Path(temp).rglob('*.log')}
                args.report.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
                raise
    args.report.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
