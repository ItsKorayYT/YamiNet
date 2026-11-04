#!/usr/bin/env python3
"""Run real block/inode ENOSPC archive tests on private, bounded tmpfs mounts."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

IMAGE = 'python:3.13-slim-trixie@sha256:ae14d4ce8771606477a81446e7ef429edd6762305fc50c84507c1041fa8285d1'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--test-binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--cpuset', help='optional CPU affinity for isolated lab scheduling')
    args = parser.parse_args()
    binary = args.test_binary.resolve()
    if args.output.exists():
        parser.error('output must be new')
    env = os.environ.copy()
    for key in ['DOCKER_HOST', 'DOCKER_CONTEXT', 'DOCKER_TLS', 'DOCKER_TLS_VERIFY', 'DOCKER_CERT_PATH']:
        env.pop(key, None)
    report = dict(passed=False, test_binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                  cpus=2, memory_bytes=2147483648, tmpfs_bytes=134217728, cases=[])
    try:
        for kind in ['blocks', 'inodes']:
            for mode in ['append', 'migration', 'recovery']:
                started = time.monotonic()
                result = subprocess.run(['docker', '--host=unix:///var/run/docker.sock', 'run',
                    *(['--cpuset-cpus', args.cpuset] if args.cpuset else []), '--rm',
                    '--network', 'none', '--cpus', '2', '--memory', '2g', '--memory-swap', '2g',
                    '--read-only', '--pids-limit', '64', '--cap-drop', 'ALL',
                    '--security-opt', 'no-new-privileges', '--user', f'{os.getuid()}:{os.getgid()}',
                    '--tmpfs', f'/store:size=128m,nr_inodes=2048,uid={os.getuid()},gid={os.getgid()}',
                    '--mount', f'type=bind,src={binary},dst=/tests,readonly',
                    '-e', 'YAMI_ARCHIVE_ENOSPC_PATH=/store', '-e', f'YAMI_ARCHIVE_ENOSPC_KIND={kind}',
                    '-e', f'YAMI_ARCHIVE_ENOSPC_MODE={mode}', IMAGE, '/tests', '--exact',
                    'archive::tests::actual_enospc_child', '--nocapture', '--test-threads=1'],
                    env=env, capture_output=True, text=True, timeout=120)
                passed = result.returncode == 0 and '1 passed; 0 failed' in result.stdout
                case = dict(kind=kind, mode=mode, passed=passed,
                            seconds=time.monotonic()-started, stdout=result.stdout, stderr=result.stderr)
                report['cases'].append(case)
                print(json.dumps(case), flush=True)
                if not passed:
                    raise RuntimeError(f'{kind}/{mode} failed')
        report['passed'] = True
    finally:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2)+'\n')


if __name__ == '__main__':
    main()
