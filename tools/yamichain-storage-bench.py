#!/usr/bin/env python3
"""Measure real-signature archive/pruned/compact recovery in bounded containers."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

IMAGE = 'python:3.13-slim-trixie@sha256:ae14d4ce8771606477a81446e7ef429edd6762305fc50c84507c1041fa8285d1'


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--process-lab-output', type=Path)
    args = p.parse_args()
    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    env = os.environ.copy()
    for key in ['DOCKER_HOST', 'DOCKER_CONTEXT', 'DOCKER_TLS', 'DOCKER_TLS_VERIFY', 'DOCKER_CERT_PATH']:
        env.pop(key, None)
    cli = ['docker', '--host=unix:///var/run/docker.sock']
    repo = Path(__file__).resolve().parent.parent
    report = dict(binary_sha256=hashlib.sha256((repo/"target/release/examples/storage-bench").read_bytes()).hexdigest(), passed=False, cpus=2, memory_bytes=2147483648, target_ssd_bytes=20000000000, store_refusal_bytes=18000000000, cases=[])
    common = ['run', '--rm', '--network', 'none', '--cpus', '2', '--memory', '2g', '--memory-swap', '2g',
              '--pids-limit', '128', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges', '--read-only',
              '--user', f'{os.getuid()}:{os.getgid()}']
    measure = """import json,resource,subprocess,sys
r=subprocess.run(['/bench','/results/chain-'+sys.argv[1],sys.argv[1],'8'],capture_output=True,text=True,timeout=600)
if r.returncode: print(r.stderr,file=sys.stderr);sys.exit(r.returncode)
d=json.loads(r.stdout);u=resource.getrusage(resource.RUSAGE_CHILDREN)
d.update(cpu_user_seconds=u.ru_utime,cpu_system_seconds=u.ru_stime,peak_rss_kib=u.ru_maxrss)
print(json.dumps(d))
"""
    try:
        for height in [64, 256, 1024]:
            command = cli+common+['--mount', f'type=bind,src={repo/"target/release/examples/storage-bench"},dst=/bench,readonly',
                                 '--mount', f'type=bind,src={root},dst=/results', IMAGE, 'python', '-c', measure, str(height)]
            result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=650)
            if result.returncode:
                raise RuntimeError(result.stderr)
            case = json.loads(result.stdout)
            report['cases'].append(case)
            print(json.dumps(case), flush=True)
        if args.process_lab_output:
            source = args.process_lab_output.resolve()
            disk_full = """import hashlib,json,os,shutil,subprocess
from pathlib import Path
shutil.copytree('/src','/store',dirs_exist_ok=True)
def hashes():return {str(p.relative_to('/store')):hashlib.sha256(p.read_bytes()).hexdigest() for p in Path('/store').rglob('*') if p.is_file() and p.name!='store.lock'}
before=hashes();full=False
with open('/store/filler','wb',buffering=0) as f:
 try:
  for _ in range(256):f.write(b'0'*65536)
 except OSError as e:full=e.errno==28
r=subprocess.run(['/validator','prune','--dir','/store','--isolated-network','/config','--index','0'],capture_output=True,text=True,timeout=60)
Path('/store/filler').unlink()
after=hashes()
print(json.dumps(dict(enospc_observed=full,prune_exit=r.returncode,committed_files_unchanged=before==after,error=r.stderr.strip(),passed=full and r.returncode!=0 and before==after)))
"""
            command = cli+common+['--tmpfs', f'/store:size=8m,uid={os.getuid()},gid={os.getgid()}',
                                 '--mount', f'type=bind,src={source/"store-0"},dst=/src,readonly',
                                 '--mount', f'type=bind,src={source/"config-0"},dst=/config,readonly',
                                 '--mount', f'type=bind,src={repo/"target/release/yami-validator"},dst=/validator,readonly',
                                 IMAGE, 'python', '-c', disk_full]
            result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=90, check=True)
            report['disk_exhaustion'] = json.loads(result.stdout)
            if not report['disk_exhaustion']['passed']:
                raise RuntimeError('disk exhaustion damaged committed state')
        report['passed'] = True
    finally:
        (root/'report.json').write_text(json.dumps(report, indent=2)+'\n')


if __name__ == '__main__':
    main()
