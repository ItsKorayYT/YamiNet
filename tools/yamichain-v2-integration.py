#!/usr/bin/env python3
"""Inactive V2 signed-history evidence under enforced private container budgets."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

IMAGE = 'python:3.13-slim-trixie@sha256:ae14d4ce8771606477a81446e7ef429edd6762305fc50c84507c1041fa8285d1'
PROFILE = '''import json,resource,subprocess,sys,time
from pathlib import Path
at=time.monotonic()
r=subprocess.run(['/bench','/results/state-'+sys.argv[1],sys.argv[1],'distinct'],capture_output=True,text=True,timeout=7200)
assert r.returncode==0,r.stderr
result=json.loads(r.stdout);u=resource.getrusage(resource.RUSAGE_CHILDREN);c=Path('/sys/fs/cgroup')
assert result['passed'] and result['inactive_v2']['production_active'] is False
assert result['inactive_v2']['entries']==int(sys.argv[1])
result.update(cpu_user_seconds=u.ru_utime,cpu_system_seconds=u.ru_stime,peak_rss_kib=u.ru_maxrss,wall_seconds=time.monotonic()-at,cgroup_cpu_max=(c/'cpu.max').read_text().strip(),cgroup_memory_max=(c/'memory.max').read_text().strip(),memory_peak_bytes=int((c/'memory.peak').read_text()))
print(json.dumps(result))
'''

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--sizes', type=int, nargs='+', default=[1024, 250000, 500000])
    p.add_argument('--cpuset', default='0,1')
    a = p.parse_args()
    if any(n not in (1024, 100000, 250000, 500000) for n in a.sizes):
        p.error('unsupported isolated fixture size')
    root = a.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    binary = root / 'bench'
    shutil.copy2(a.binary, binary)
    env = os.environ.copy()
    for key in ('DOCKER_HOST', 'DOCKER_CONTEXT', 'DOCKER_TLS', 'DOCKER_TLS_VERIFY', 'DOCKER_CERT_PATH'):
        env.pop(key, None)
    report = dict(passed=False, live_engine_generated=False, cpus=2, memory_bytes=2147483648,
                  binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), cases=[])
    try:
        for size in a.sizes:
            command = ['docker', '--host=unix:///var/run/docker.sock', 'run', '--rm',
                       '--network', 'none', '--cpuset-cpus', a.cpuset, '--cpus', '2',
                       '--memory', '2g', '--memory-swap', '2g', '--pids-limit', '128',
                       '--read-only', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges',
                       '--user', f'{os.getuid()}:{os.getgid()}',
                       '--mount', f'type=bind,src={binary},dst=/bench,readonly',
                       '--mount', f'type=bind,src={root},dst=/results',
                       IMAGE, 'python', '-c', PROFILE, str(size)]
            result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=7250)
            if result.returncode:
                raise RuntimeError(result.stderr + result.stdout)
            case = json.loads(result.stdout)
            report['cases'].append(case)
            print(json.dumps(case), flush=True)
        report['passed'] = True
    finally:
        (root / 'report.json').write_text(json.dumps(report, indent=2) + '\n')

if __name__ == '__main__':
    main()
