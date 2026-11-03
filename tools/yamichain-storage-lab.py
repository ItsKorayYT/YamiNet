#!/usr/bin/env python3
"""Private, resource-limited compact bootstrap, pruning and crash-recovery lab."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

IMAGE = 'python:3.13-slim-trixie@sha256:ae14d4ce8771606477a81446e7ef429edd6762305fc50c84507c1041fa8285d1'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--binary', type=Path, default=Path('target/release/yami-validator'))
    args = parser.parse_args()
    root = args.output.resolve()
    root.mkdir(parents=True, mode=0o700, exist_ok=False)
    binary = args.binary.resolve()
    env = os.environ.copy()
    for key in ['DOCKER_HOST', 'DOCKER_CONTEXT', 'DOCKER_TLS', 'DOCKER_TLS_VERIFY', 'DOCKER_CERT_PATH']:
        env.pop(key, None)
    cli = ['docker', '--host=unix:///var/run/docker.sock']
    tag = f'yami-storage-{os.getpid()}'
    names = [f'{tag}-{i}' for i in range(4)]
    report = dict(passed=False, binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), samples=[], events=[])
    started = time.monotonic()

    def docker(*cmd, check=True):
        return subprocess.run(cli+list(cmd), env=env, capture_output=True, text=True, check=check, timeout=40).stdout.strip()

    def local(command, i=0, store=None, extra=(), expected=0):
        result = subprocess.run([str(binary), command, '--dir', str(store or root/f'store-{i}'),
                                 '--isolated-network', str(root/f'config-{i}'), '--index', str(i), *map(str, extra)], capture_output=True, text=True, timeout=90)
        if result.returncode != expected:
            raise RuntimeError(f'{command}: {result.returncode}: {result.stderr}')
        return json.loads(result.stdout.splitlines()[-1]) if expected == 0 else None

    def height(i):
        records = list((root/f'store-{i}').glob('finalized-*.bin'))
        return max([int(f.stem.split('-')[1]) for f in records], default=0)

    def launch(i, target, linger):
        docker('run', '-d', '--name', names[i], '--network', tag, '--ip', f'10.83.0.{i+2}',
               '--cpus', '2', '--memory', '2g', '--memory-swap', '2g', '--pids-limit', '256',
               '--ulimit', 'nofile=512:512', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges',
               '--read-only', '--user', f'{os.getuid()}:{os.getgid()}',
               '--mount', f'type=bind,src={binary},dst=/validator,readonly',
               '--mount', f'type=bind,src={root/f"config-{i}"},dst=/config,readonly',
               '--mount', f'type=bind,src={root/f"store-{i}"},dst=/store', IMAGE,
               '/validator', 'run', '--dir', '/store', '--index', str(i), '--isolated-network', '/config',
               '--height', str(target), '--timeout', '240', '--linger', str(linger), '--test-transactions', '4',
               '--storage-mode', 'archive' if i == 0 else 'pruned', '--checkpoint-interval', '32')

    def sample():
        raw = docker('stats', '--no-stream', '--format', '{{json .}}', *names, check=False)
        report['samples'].append(dict(at=time.monotonic()-started, nodes=[json.loads(line) for line in raw.splitlines() if line.startswith('{')]))

    def wait_all(label):
        deadline = time.monotonic()+260
        while time.monotonic() < deadline:
            sample()
            states = [json.loads(docker('inspect', '--format', '{{json .State}}', name)) for name in names]
            if all(not state['Running'] for state in states):
                if any(state['ExitCode'] != 0 for state in states):
                    raise RuntimeError(f'{label}: process failed: {states}')
                result = [local('inspect', i) for i in range(4)]
                if len({(s['height'], s['block'], s['state_root']) for s in result}) != 1:
                    raise RuntimeError(f'{label}: certified state disagreement')
                report[label] = result
                return
            time.sleep(.5)
        raise RuntimeError(f'{label}: deadline exceeded')

    def preserve_logs(phase):
        for i, node in enumerate(names):
            result = subprocess.run(cli+['logs', node], env=env, capture_output=True, text=True)
            (root/f'{phase}-node-{i}.log').write_text(result.stdout+result.stderr)

    def safety(path, include_wal=True):
        return {str(p.relative_to(path)): hashlib.sha256(p.read_bytes()).hexdigest()
                for p in path.rglob('*') if p.is_file() and (p.name.startswith(('signed-', 'equivocation-')) or (include_wal and p.parent.name == 'wal'))}

    provision = root/'provision'
    subprocess.run([str(binary), 'init-isolated', '--dir', str(provision), '--endpoints', ','.join(f'10.83.0.{i+2}:28100' for i in range(4))], check=True)
    for i in range(4):
        config = root/f'config-{i}'
        config.mkdir(mode=0o700)
        for file in ['roster.bin', 'bootstrap-0.bin', 'bootstrap-1.bin', f'identity-{i}.bin']:
            shutil.copy2(provision/file, config/file)
        (root/f'store-{i}').mkdir()
    network_created = False
    try:
        docker('network', 'create', '--internal', '--subnet', '10.83.0.0/24', tag)
        network_created = True
        for i in range(3):
            launch(i, 128, 30)
        deadline = time.monotonic()+150
        while height(0) < 64:
            if time.monotonic() > deadline:
                raise RuntimeError('initial history generation stalled')
            time.sleep(.3)
        docker('kill', '--signal', 'INT', names[0])
        docker('wait', names[0])
        export = local('export-checkpoint', extra=['--out', root/'bootstrap.bin'])
        report['bootstrap'] = export
        report['bootstrap_trust'] = 'anchor obtained from controlled authenticated archive independently of transferred snapshot payload'
        fresh = local('import-checkpoint', 3, extra=['--snapshot', root/'bootstrap.bin', '--anchor', export['anchor']])
        if fresh['height'] != export['height'] or list((root/'store-3').glob('finalized-*.bin')):
            raise RuntimeError('fresh bootstrap downloaded history or changed state')
        report['events'].append(dict(at=time.monotonic()-started, event='fresh_identity_bootstrapped_without_prefix_history', height=fresh['height']))
        docker('start', names[0])
        launch(3, 128, 30)
        wait_all('first_convergence')
        preserve_logs('phase1')
        report['phase1_storage'] = [local('storage-info', i) for i in range(4)]
        report['safety_before_restart'] = [safety(root/f'store-{i}', False) for i in range(4)]
        for node in names:
            docker('rm', node)
        for i in range(4):
            launch(i, 160, 5)
        wait_all('restart_convergence')
        preserve_logs('phase2')
        for i, before in enumerate(report['safety_before_restart']):
            after = safety(root/f'store-{i}', False)
            if any(after.get(path) != digest for path, digest in before.items()):
                raise RuntimeError('restart deleted or changed original signing safety data')
        report['final_storage'] = [local('storage-info', i) for i in range(4)]
        # Real offline process interruption at precise storage transaction boundaries.
        report['storage_crashes'] = []
        for stage in ['base', 'unlink', 'export-temp']:
            store = root/f'crash-{stage}'
            shutil.copytree(root/'store-0', store)
            before = safety(store)
            command = 'export-checkpoint' if stage == 'export-temp' else 'prune'
            extra = ['--storage-crash', stage]
            if command == 'export-checkpoint':
                extra += ['--out', store/'test-export.bin']
            local(command, store=store, extra=extra, expected=86)
            restored = local('inspect', store=store)
            if restored != report['restart_convergence'][0] or safety(store) != before:
                raise RuntimeError(f'{stage}: interrupted operation damaged state/safety data')
            if command == 'export-checkpoint':
                if (store/'test-export.bin').exists():
                    raise RuntimeError('partial export published')
                local(command, store=store, extra=['--out', store/'test-export.bin'])
            else:
                local('prune', store=store)
            report['storage_crashes'].append(dict(stage=stage, exit=86, authenticated_recovery=True, safety_preserved=True))
        final_export = local('export-checkpoint', extra=['--out', root/'final-bootstrap.bin'])
        destination = root/'crash-import'
        local('import-checkpoint', 3, store=destination, extra=['--snapshot', root/'final-bootstrap.bin', '--anchor', final_export['anchor'], '--storage-crash', 'base'], expected=86)
        imported = local('import-checkpoint', 3, store=destination, extra=['--snapshot', root/'final-bootstrap.bin', '--anchor', final_export['anchor']])
        if imported != report['restart_convergence'][0]:
            raise RuntimeError('interrupted import failed authenticated resume')
        report['storage_crashes'].append(dict(stage='import-base', exit=86, authenticated_recovery=True))
        report['passed'] = True
    finally:
        report['elapsed_seconds'] = time.monotonic()-started
        (root/'report.json').write_text(json.dumps(report, indent=2)+'\n')
        preserve_logs('last')
        for node in names:
            docker('rm', '-f', node, check=False)
        if network_created:
            docker('network', 'rm', tag, check=False)
    print(json.dumps(dict(passed=True, seconds=report['elapsed_seconds'], report=str(root/'report.json'))))


if __name__ == '__main__':
    main()
