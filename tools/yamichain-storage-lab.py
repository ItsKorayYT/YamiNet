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
from yamichain_safety import capture_safety, preserved, observed_anchor

IMAGE = 'python:3.13-slim-trixie@sha256:ae14d4ce8771606477a81446e7ef429edd6762305fc50c84507c1041fa8285d1'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--binary', type=Path, default=Path('target/release/yami-validator'))
    parser.add_argument('--legacy-binary', type=Path)
    parser.add_argument('--segmented-archive', action='store_true')
    parser.add_argument('--height', type=int, default=128)
    parser.add_argument('--restart-count', type=int, default=1)
    parser.add_argument('--restart-step', type=int, default=32)
    args = parser.parse_args()
    if not 128 <= args.height <= 2048 or not 1 <= args.restart_count <= 5 or not 32 <= args.restart_step <= 128:
        parser.error('height 128..2048, restart-count 1..5, restart-step 32..128 required')
    root = args.output.resolve()
    root.mkdir(parents=True, mode=0o700, exist_ok=False)
    # Keep one immutable executable for the entire campaign even if another
    # terminal rebuilds the source target while containers are running.
    binary = root/'validator'
    shutil.copy2(args.binary.resolve(), binary)
    legacy_binary = None
    if args.legacy_binary:
        legacy_binary = root/'legacy-validator'
        shutil.copy2(args.legacy_binary.resolve(), legacy_binary)
    env = os.environ.copy()
    for key in ['DOCKER_HOST', 'DOCKER_CONTEXT', 'DOCKER_TLS', 'DOCKER_TLS_VERIFY', 'DOCKER_CERT_PATH']:
        env.pop(key, None)
    cli = ['docker', '--host=unix:///var/run/docker.sock']
    tag = f'yami-storage-{os.getpid()}'
    names = [f'{tag}-{i}' for i in range(4)]
    report = dict(passed=False, binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), samples=[], events=[])
    report['legacy_binary_sha256'] = hashlib.sha256(legacy_binary.read_bytes()).hexdigest() if legacy_binary else None
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
        tip = root/f'store-{i}'/'archive-tip.bin'
        if tip.exists():
            data = tip.read_bytes()
            if len(data) >= 24:
                return int.from_bytes(data[16:24], 'big')
        records = list((root/f'store-{i}').glob('finalized-*.bin'))
        return max([int(f.stem.split('-')[1]) for f in records], default=0)

    def launch(i, target, linger, legacy=False):
        executable = legacy_binary if legacy else binary
        anchor = None if legacy else observed_anchor(root/f'store-{i}')
        minimum = ['--signing-anchor', anchor] if anchor else []
        if args.segmented_archive and i == 0 and 'archive_migration' in report and not legacy:
            a = report['archive_migration']
            minimum += ['--archive-anchor', f"{a['height']}:{a['root']}"]
        docker('run', '-d', '--name', names[i], '--network', tag, '--ip', f'10.83.0.{i+2}',
               '--cpus', '2', '--memory', '2g', '--memory-swap', '2g', '--pids-limit', '256',
               '--ulimit', 'nofile=512:512', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges',
               '--read-only', '--user', f'{os.getuid()}:{os.getgid()}',
               '--mount', f'type=bind,src={executable},dst=/validator,readonly',
               '--mount', f'type=bind,src={root/f"config-{i}"},dst=/config,readonly',
               '--mount', f'type=bind,src={root/f"store-{i}"},dst=/store', IMAGE,
               '/validator', 'run', '--dir', '/store', '--index', str(i), '--isolated-network', '/config',
               '--height', str(target), '--timeout', str(min(3600, max(240, args.height*3))), '--linger', str(linger), '--test-transactions', '4',
               '--storage-mode', 'archive' if i == 0 else 'pruned', '--checkpoint-interval', '32', *minimum)

    def sample():
        raw = docker('stats', '--no-stream', '--format', '{{json .}}', *names, check=False)
        report['samples'].append(dict(at=time.monotonic()-started, nodes=[json.loads(line) for line in raw.splitlines() if line.startswith('{')]))

    def wait_all(label, target):
        deadline = time.monotonic()+min(3600, max(240, args.height*3))+60
        while time.monotonic() < deadline:
            sample()
            states = [json.loads(docker('inspect', '--format', '{{json .State}}', name)) for name in names]
            if all(not state['Running'] for state in states):
                if any(state['ExitCode'] != 0 for state in states):
                    raise RuntimeError(f'{label}: process failed: {states}')
                result = [local('inspect', i) for i in range(4)]
                if len({(s['height'], s['block'], s['state_root']) for s in result}) != 1:
                    raise RuntimeError(f'{label}: certified state disagreement')
                if any(s['height'] != target for s in result):
                    raise RuntimeError(f'{label}: did not reach requested height {target}')
                report[label] = result
                return
            time.sleep(.5)
        raise RuntimeError(f'{label}: deadline exceeded')

    def preserve_logs(phase):
        for i, node in enumerate(names):
            result = subprocess.run(cli+['logs', node], env=env, capture_output=True, text=True)
            (root/f'{phase}-node-{i}.log').write_text(result.stdout+result.stderr)

    def safety(path, include_wal=True):
        return capture_safety(path, live=not include_wal)

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
            launch(i, args.height, 30, legacy=legacy_binary is not None)
        deadline = time.monotonic()+150
        # Exercise an imported base beyond the 32-height pruning boundary.
        while height(0) < 65:
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
        if args.segmented_archive:
            shutil.copytree(root/'store-0', root/'original-archive')
            report['initial_archive_state'] = local('inspect')
            report['archive_migration'] = local('migrate-archive')
            report['initial_archive_migration'] = report['archive_migration']
            docker('rm', names[0])
            launch(0, args.height, 30)
            # The fresh identity has never signed. Delay its first launch until
            # the segmented source is ahead of its independently anchored base.
            deadline = time.monotonic()+min(3600, max(240, args.height*3))
            while height(0) < args.height:
                if time.monotonic() > deadline:
                    raise RuntimeError('segmented archive did not advance')
                time.sleep(.3)
            report['events'].append(dict(event='fresh_pruned_peer_requests_segmented_history', source_height=height(0), imported_height=fresh['height']))
        else:
            docker('start', names[0])
        launch(3, args.height, 30)
        wait_all('first_convergence', args.height)
        preserve_logs('phase1')
        report['phase1_storage'] = [local('storage-info', i) for i in range(4)]
        report['safety_before_restart'] = [safety(root/f'store-{i}', False) for i in range(4)]
        report['migrations'] = []
        if legacy_binary and not args.segmented_archive:
            for i in range(4):
                store = root/f'store-{i}'
                before = {str(p.relative_to(store)):hashlib.sha256(p.read_bytes()).hexdigest()
                          for p in store.rglob('*') if p.is_file() and p.name != 'validator-index.bin'}
                info = local('migrate-signing', i)
                if any(not (store/path).is_file() or hashlib.sha256((store/path).read_bytes()).hexdigest()!=value for path,value in before.items()):
                    raise RuntimeError('migration changed historical storage bytes')
                refused = root/f'downgrade-{i}'
                shutil.copytree(store, refused)
                result = subprocess.run([str(legacy_binary), 'run', '--dir', str(refused), '--isolated-network', str(root/f'config-{i}'), '--index', str(i), '--height', str(args.height+1), '--timeout', '1'],capture_output=True,text=True,timeout=30)
                if result.returncode == 0 or 'conflicting durable record' not in result.stderr:
                    raise RuntimeError(f'old validator did not respect format fence: {result.stderr}')
                report['migrations'].append(dict(index=i, signing=info, historical_files_preserved=len(before), downgrade_refused=True))
        if args.segmented_archive:
            before = {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                      for p in (root/'store-0').iterdir() if p.is_file() and p.name != 'genesis.bin'}
            info = local('migrate-archive')
            if any(not (root/'store-0'/name).is_file() or hashlib.sha256((root/'store-0'/name).read_bytes()).hexdigest() != digest for name, digest in before.items()):
                raise RuntimeError('archive migration changed historical or safety bytes')
            report['archive_migration'] = info
            if legacy_binary:
                refused = root/'archive-downgrade'
                shutil.copytree(root/'store-0', refused)
                result = subprocess.run([str(legacy_binary), 'inspect', '--dir', str(refused), '--isolated-network', str(root/'config-0'), '--index', '0'], capture_output=True, text=True, timeout=30)
                if result.returncode == 0 or 'conflicting durable record' not in result.stderr:
                    raise RuntimeError(f'archive downgrade did not refuse: {result.stderr}')
                report['archive_downgrade_refused'] = True
            report['archive_migration_crashes'] = []
            for stage in ['manifest', 'segment', 'tip', 'active', 'fence']:
                store = root/f'archive-crash-{stage}'
                shutil.copytree(root/'original-archive', store)
                local('migrate-archive', store=store, extra=['--storage-crash', f'archive-{stage}'], expected=86)
                local('migrate-archive', store=store)
                if local('inspect', store=store) != report['initial_archive_state']:
                    raise RuntimeError('interrupted archive migration changed certified state')
                report['archive_migration_crashes'].append(dict(stage=stage, resumed=True))
        report['post_migration_storage'] = [local('storage-info', i) for i in range(4)]
        report['post_migration_signing'] = [local('signing-info', i) for i in range(4)]

        report['restarts'] = []
        for restart in range(args.restart_count):
            before_restart = [safety(root/f'store-{i}', False) for i in range(4)]
            for node in names:
                docker('rm', node)
            for i in range(4):
                launch(i, args.height + (restart+1)*args.restart_step, 5)
            wait_all('restart_convergence', args.height + (restart+1)*args.restart_step)
            preserve_logs(f'restart-{restart}')
            for i, before in enumerate(before_restart):
                if not preserved(root/f'store-{i}', before):
                    raise RuntimeError('restart changed original safety records')
            report['restarts'].append(report['restart_convergence'])
        for i, before in enumerate(report['safety_before_restart']):
            if not preserved(root/f'store-{i}', before):
                raise RuntimeError('restart deleted or changed original signing safety data')
        report['final_storage'] = [local('storage-info', i) for i in range(4)]
        report['final_signing'] = [local('signing-info', i) for i in range(4)]
        # Real offline process interruption at precise storage transaction boundaries.
        report['storage_crashes'] = []
        for stage in ['base', 'unlink', 'export-temp']:
            store = root/f'crash-{stage}'
            crash_index = 1 if args.segmented_archive else 0
            shutil.copytree(root/f'store-{crash_index}', store)
            before = safety(store)
            command = 'export-checkpoint' if stage == 'export-temp' else 'prune'
            extra = ['--storage-crash', stage]
            if command == 'export-checkpoint':
                extra += ['--out', store/'test-export.bin']
            local(command, i=crash_index, store=store, extra=extra, expected=86)
            restored = local('inspect', i=crash_index, store=store)
            if restored != report['restart_convergence'][0] or safety(store) != before:
                raise RuntimeError(f'{stage}: interrupted operation damaged state/safety data')
            if command == 'export-checkpoint':
                if (store/'test-export.bin').exists():
                    raise RuntimeError('partial export published')
                local(command, i=crash_index, store=store, extra=['--out', store/'test-export.bin'])
            else:
                local('prune', i=crash_index, store=store)
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
