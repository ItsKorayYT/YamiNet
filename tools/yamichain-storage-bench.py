#!/usr/bin/env python3
"""Measure real-signature archive/pruned/compact recovery in bounded containers."""
import argparse
import hashlib
import json
import os
import shutil
from pathlib import Path
import subprocess

IMAGE = 'python:3.13-slim-trixie@sha256:ae14d4ce8771606477a81446e7ef429edd6762305fc50c84507c1041fa8285d1'


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--process-lab-output', type=Path)
    p.add_argument('--heights', type=int, nargs='+', default=[64, 256, 1024])
    args = p.parse_args()
    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    env = os.environ.copy()
    for key in ['DOCKER_HOST', 'DOCKER_CONTEXT', 'DOCKER_TLS', 'DOCKER_TLS_VERIFY', 'DOCKER_CERT_PATH']:
        env.pop(key, None)
    cli = ['docker', '--host=unix:///var/run/docker.sock']
    repo = Path(__file__).resolve().parent.parent
    binary = root/'bench'
    shutil.copy2(repo/'target/release/examples/storage-bench', binary)
    validator = root/'validator'
    if args.process_lab_output:
        shutil.copy2(repo/'target/release/yami-validator', validator)
    report = dict(binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), passed=False, cpus=2, memory_bytes=2147483648, target_ssd_bytes=20000000000, store_refusal_bytes=18000000000, target_ssd_enforced=False, cases=[])
    common = ['run', '--rm', '--network', 'none', '--cpus', '2', '--memory', '2g', '--memory-swap', '2g',
              '--pids-limit', '128', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges', '--read-only',
              '--user', f'{os.getuid()}:{os.getgid()}']
    measure = """import json,os,resource,subprocess,sys,time
from pathlib import Path
started=time.monotonic()
r=subprocess.run(['/bench','/results/chain-'+sys.argv[1],sys.argv[1],'8'],capture_output=True,text=True,timeout=7200)
if r.returncode: print(r.stderr,file=sys.stderr);sys.exit(r.returncode)
d=json.loads(r.stdout);u=resource.getrusage(resource.RUSAGE_CHILDREN)
d.update(cgroup_cpu_max=Path('/sys/fs/cgroup/cpu.max').read_text().strip(),cgroup_memory_max=Path('/sys/fs/cgroup/memory.max').read_text().strip(),filesystem_fragment_bytes=os.statvfs('/results').f_frsize,wall_seconds=time.monotonic()-started,cpu_user_seconds=u.ru_utime,cpu_system_seconds=u.ru_stime,peak_rss_kib=u.ru_maxrss)
print(json.dumps(d))
"""
    try:
        for height in args.heights:
            command = cli+common+['--mount', f'type=bind,src={binary},dst=/bench,readonly',
                                 '--mount', f'type=bind,src={root},dst=/results', IMAGE, 'python', '-c', measure, str(height)]
            result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=7300)
            if result.returncode:
                raise RuntimeError(result.stderr)
            case = json.loads(result.stdout)
            report['cases'].append(case)
            print(json.dumps(case), flush=True)
        if args.process_lab_output:
            source = args.process_lab_output.resolve()
            report['validator_sha256'] = hashlib.sha256(validator.read_bytes()).hexdigest()
            disk_full = """import hashlib,json,os,shutil,subprocess
from pathlib import Path
shutil.copytree('/src','/store',dirs_exist_ok=True)
def hashes():return {str(p.relative_to('/store')):hashlib.sha256(p.read_bytes()).hexdigest() for p in Path('/store').rglob('*') if p.is_file() and p.name!='store.lock'}
before=hashes();full=False
with open('/store/filler','wb',buffering=0) as f:
 try:
  for _ in range(2048):f.write(b'0'*65536)
 except OSError as e:full=e.errno==28
r=subprocess.run(['/validator','prune','--dir','/store','--isolated-network','/config','--index','0'],capture_output=True,text=True,timeout=60)
Path('/store/filler').unlink()
after=hashes()
print(json.dumps(dict(enospc_observed=full,prune_exit=r.returncode,committed_files_unchanged=before==after,error=r.stderr.strip(),passed=full and r.returncode!=0 and before==after)))
"""
            command = cli+common+['--tmpfs', f'/store:size=64m,uid={os.getuid()},gid={os.getgid()}',
                                 '--mount', f'type=bind,src={source/"store-0"},dst=/src,readonly',
                                 '--mount', f'type=bind,src={source/"config-0"},dst=/config,readonly',
                                 '--mount', f'type=bind,src={validator},dst=/validator,readonly',
                                 IMAGE, 'python', '-c', disk_full]
            result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=90, check=True)
            report['disk_exhaustion'] = json.loads(result.stdout)
            if not report['disk_exhaustion']['passed']:
                raise RuntimeError('disk exhaustion damaged committed state')
            inode_full = """import hashlib,json,os,shutil,subprocess
from pathlib import Path
shutil.copytree('/src','/store',dirs_exist_ok=True)
def hashes():return {str(p.relative_to('/store')):hashlib.sha256(p.read_bytes()).hexdigest() for p in Path('/store').rglob('*') if p.is_file() and p.name!='store.lock'}
before=hashes();fillers=[]
while os.statvfs('/store').f_favail >= 128:
 p=Path('/store')/('inode-fill-'+str(len(fillers)));p.touch();fillers.append(p)
free=os.statvfs('/store').f_favail
command=['/validator','prune','--dir','/store','--isolated-network','/config','--index','0']
r=subprocess.run(command,capture_output=True,text=True,timeout=60)
exhausted=False
try:
 while True:
  p=Path('/store')/('inode-fill-'+str(len(fillers)));p.touch();fillers.append(p)
except OSError as e:exhausted=e.errno==28
full=subprocess.run(command,capture_output=True,text=True,timeout=60)
for p in fillers:p.unlink()
after=hashes()
print(json.dumps(dict(free_inodes_at_reserve_refusal=free,prune_exit=r.returncode,inode_enospc_observed=exhausted,exhausted_prune_exit=full.returncode,committed_files_unchanged=before==after,error=r.stderr.strip(),exhausted_error=full.stderr.strip(),passed=free<128 and r.returncode!=0 and full.returncode!=0 and exhausted and before==after and 'inode reserve' in r.stderr)))
"""
            command = cli+common+['--tmpfs', f'/store:size=256m,nr_inodes=16384,uid={os.getuid()},gid={os.getgid()}',
                                 '--mount', f'type=bind,src={source/"store-0"},dst=/src,readonly',
                                 '--mount', f'type=bind,src={source/"config-0"},dst=/config,readonly',
                                 '--mount', f'type=bind,src={validator},dst=/validator,readonly',
                                 IMAGE, 'python', '-c', inode_full]
            result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=90, check=True)
            report['inode_exhaustion'] = json.loads(result.stdout)
            if not report['inode_exhaustion']['passed']:
                raise RuntimeError('inode exhaustion did not refuse safely')
        report['passed'] = True
    finally:
        (root/'report.json').write_text(json.dumps(report, indent=2)+'\n')


if __name__ == '__main__':
    main()
