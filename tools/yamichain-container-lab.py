#!/usr/bin/env python3
"""Run owned, network-isolated YamiChain process campaigns with explicit limits."""
import argparse
import json
import os
from pathlib import Path
import subprocess

CASES = {
    'smoke4': dict(nodes=4, height=16, tx=8, timeout=90, memory='2g'),
    'sustain4': dict(nodes=4, height=256, tx=16, timeout=600, memory='2g'),
    'fault4': dict(nodes=4, height=48, tx=8, timeout=240, memory='2g', faults='partition,crash,socket-fuzz', delay=25),
    'stress25': dict(nodes=25, height=24, tx=4, timeout=300, memory='2g', faults='partition,crash', delay=25),
    'stress100': dict(nodes=100, height=8, tx=2, timeout=300, memory='6g'),
    'transitions': dict(nodes=25, height=64, tx=4, timeout=600, memory='2g', plan=','.join(f'{h}:{25 if n % 2 == 0 else 19}' for n, h in enumerate(range(1, 65, 4)))),
    'disk-full': dict(nodes=4, height=48, tx=8, timeout=240, memory='2g', faults='disk-full'),
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--case', choices=CASES, required=True)
    parser.add_argument('--output', type=Path, default=Path('work/chain-lab'))
    parser.add_argument('--binary', type=Path, default=Path('target/release/yami-validator'))
    parser.add_argument('--image', default='python:3.13-slim-trixie@sha256:ae14d4ce8771606477a81446e7ef429edd6762305fc50c84507c1041fa8285d1')
    args = parser.parse_args()
    repo = Path(__file__).resolve().parent.parent
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    case = CASES[args.case]
    directory = output / args.case
    report = output / f'{args.case}.json'
    if report.exists() or (directory.exists() and any(directory.iterdir())):
        parser.error('case/report already exists; choose a fresh output directory')
    name = f'yamichain-{args.case}-{os.getpid()}'
    cli = ['docker', '--host=unix:///var/run/docker.sock']
    environment = os.environ.copy()
    for key in ['DOCKER_HOST', 'DOCKER_CONTEXT', 'DOCKER_TLS', 'DOCKER_TLS_VERIFY', 'DOCKER_CERT_PATH']:
        environment.pop(key, None)
    # One writable owned bind for stores/evidence. Repository and binary are read-only.
    command = cli + ['run', '--name', name, '--rm', '--network', 'none', '--cpus', '2',
                     '--memory', case['memory'], '--memory-swap', case['memory'], '--pids-limit', '4096',
                     '--ulimit', 'nofile=65536:65536', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges',
                     '--user', f'{os.getuid()}:{os.getgid()}',
                     '--mount', f'type=bind,src={repo},dst=/repo,readonly',
                     '--mount', f'type=bind,src={output},dst=/results']
    if case.get('faults') == 'disk-full':
        directory.mkdir(parents=True, exist_ok=True)
        # Fill only this explicitly bounded tmpfs; never the host filesystem.
        command += ['--tmpfs', f'/results/{args.case}/node-0:size=2m,uid={os.getuid()},gid={os.getgid()}']
    binary = args.binary.resolve()
    if not binary.is_relative_to(repo):
        parser.error('binary must be inside this repository')
    command += [args.image, 'python', '/repo/tools/yamichain-stress.py',
                '--binary', '/repo/' + str(binary.relative_to(repo)),
                '--directory', f'/results/{args.case}', '--report', f'/results/{args.case}.json',
                '--validators', str(case['nodes']), '--height', str(case['height']),
                '--transactions', str(case['tx']), '--timeout', str(case['timeout'])]
    if case.get('faults'):
        command += ['--faults', case['faults']]
    if case.get('delay'):
        command += ['--delay-ms', str(case['delay'])]
    if case.get('plan'):
        command += ['--committee-plan', case['plan']]
    try:
        process = subprocess.run(command, env=environment, timeout=case['timeout'] + 120)
        if report.exists():
            evidence = json.loads(report.read_text())
            evidence['container_image'] = args.image
            evidence['container_limits_requested'] = dict(cpus=2, memory=case['memory'], swap=False, network='none')
            evidence['container_exit_code'] = process.returncode
            report.write_text(json.dumps(evidence, indent=2) + '\n')
        else:
            report.write_text(json.dumps({'result': 'FAIL', 'case': args.case,
                'container_exit_code': process.returncode,
                'failure': 'container exited before producing measurements; no resource claim',
                'container_limits_requested': {'cpus': 2, 'memory': case['memory'], 'swap': False}}, indent=2) + '\n')
        raise SystemExit(process.returncode)
    except subprocess.TimeoutExpired:
        report.write_text(json.dumps({'result': 'FAIL', 'case': args.case,
            'failure': 'container campaign deadline; no successful measurement claimed'}, indent=2) + '\n')
        raise SystemExit(1)
    finally:
        subprocess.run(cli + ['rm', '--force', name], env=environment, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)


if __name__ == '__main__':
    main()
