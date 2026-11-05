#!/usr/bin/env python3
"""Actual inactive-reader ENOSPC tests; never point this at a safety store."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
from importlib.util import spec_from_file_location, module_from_spec


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--test-binary', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--cpuset', default='0,1')
    a = p.parse_args()
    spec = spec_from_file_location('integration', Path(__file__).with_name('yamichain-v2-integration.py'))
    integration = module_from_spec(spec)
    spec.loader.exec_module(integration)
    binary = a.test_binary.resolve()
    report = dict(passed=False, binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), cases=[])
    env = os.environ.copy()
    for key in ('DOCKER_HOST', 'DOCKER_CONTEXT', 'DOCKER_TLS', 'DOCKER_TLS_VERIFY', 'DOCKER_CERT_PATH'):
        env.pop(key, None)
    try:
        for kind in ('blocks', 'inodes'):
            command = ['docker', '--host=unix:///var/run/docker.sock', 'run', '--rm',
                       '--network', 'none', '--cpuset-cpus', a.cpuset, '--cpus', '2', '--memory', '2g',
                       '--memory-swap', '2g', '--read-only', '--pids-limit', '128', '--cap-drop', 'ALL',
                       '--security-opt', 'no-new-privileges', '--user', f'{os.getuid()}:{os.getgid()}',
                       '--tmpfs', f'/store:size=128m,nr_inodes=2048,uid={os.getuid()},gid={os.getgid()}',
                       '--mount', f'type=bind,src={binary},dst=/tests,readonly',
                       '-e', 'YAMI_V2_ENOSPC=/store', '-e', f'YAMI_V2_PRESSURE={kind}',
                       integration.IMAGE, '/tests', '--exact', 'checkpoint_v2::tests::actual_enospc_child',
                       '--test-threads=1', '--nocapture']
            result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=120)
            assert result.returncode == 0 and '1 passed; 0 failed' in result.stdout, result.stderr + result.stdout
            report['cases'].append(dict(kind=kind, passed=True, stdout=result.stdout))
        report['passed'] = True
    finally:
        a.output.parent.mkdir(parents=True, exist_ok=True)
        a.output.write_text(json.dumps(report, indent=2) + '\n')

if __name__ == '__main__':
    main()
