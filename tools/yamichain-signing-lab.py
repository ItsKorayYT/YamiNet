#!/usr/bin/env python3
"""Bounded, private signing-storage measurements and actual filesystem faults."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

IMAGE = 'python:3.13-slim-trixie@sha256:ae14d4ce8771606477a81446e7ef429edd6762305fc50c84507c1041fa8285d1'


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--baseline-validator', type=Path)
    args = p.parse_args()
    repo = Path(__file__).resolve().parent.parent
    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    binary = root/'signing-bench'
    shutil.copy2(repo/'target/release/examples/signing-bench', binary)
    env = os.environ.copy()
    for key in ['DOCKER_HOST', 'DOCKER_CONTEXT', 'DOCKER_TLS', 'DOCKER_TLS_VERIFY', 'DOCKER_CERT_PATH']:
        env.pop(key, None)
    cli = ['docker', '--host=unix:///var/run/docker.sock']
    common = ['run', '--rm', '--network', 'none', '--cpus', '2', '--memory', '2g', '--memory-swap', '2g',
              '--pids-limit', '128', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges', '--read-only',
              '--user', f'{os.getuid()}:{os.getgid()}']
    mounts = ['--mount', f'type=bind,src={binary},dst=/bench,readonly',
              '--mount', f'type=bind,src={root},dst=/results']
    report = dict(passed=False, binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                  cpus=2, memory_bytes=2147483648, hard_20GB_volume_enforced=False, cases=[])

    def run(command, timeout=1800):
        r = subprocess.run(command, env=env, capture_output=True, text=True, timeout=timeout)
        if r.returncode:
            raise RuntimeError(r.stderr)
        return json.loads(r.stdout)

    measure = """import json,resource,subprocess,sys,time
from pathlib import Path
start=time.monotonic()
r=subprocess.run(['/bench',sys.argv[1],'/results/'+sys.argv[2],sys.argv[3]],capture_output=True,text=True,timeout=1700)
if r.returncode:print(r.stderr,file=sys.stderr);sys.exit(r.returncode)
d=json.loads(r.stdout);u=resource.getrusage(resource.RUSAGE_CHILDREN)
d.update(cpu_user_seconds=u.ru_utime,cpu_system_seconds=u.ru_stime,peak_rss_kib=u.ru_maxrss,wall_seconds=time.monotonic()-start,cgroup_cpu_max=Path('/sys/fs/cgroup/cpu.max').read_text().strip(),cgroup_memory_max=Path('/sys/fs/cgroup/memory.max').read_text().strip())
print(json.dumps(d))
"""
    try:
        for mode, name, count in [('legacy','legacy-1024',1024),('fresh','fresh-1024',1024),
                                  ('legacy','legacy-8192',8192),('fresh','fresh-8192',8192),
                                  ('fresh','fresh-65536',65536),('migrate','legacy-8192',65536)]:
            if mode == 'migrate':
                shutil.copytree(root/name, root/'legacy-pressure-source')
            data = run(cli+common+mounts+[IMAGE,'python','-c',measure,mode,name,str(count)])
            if not data['passed']:
                raise RuntimeError('signing case failed')
            report['cases'].append(data)
            print(json.dumps(data), flush=True)
        run(cli+common+mounts+[IMAGE,'python','-c',measure,'fresh','pressure-seed','1'])
        if args.baseline_validator:
            copy = root/'old-validator-attempt'
            shutil.copytree(root/'pressure-seed', copy)
            before = {str(p.relative_to(copy)):hashlib.sha256(p.read_bytes()).hexdigest() for p in copy.rglob('*') if p.is_file()}
            r = subprocess.run([str(args.baseline_validator.resolve()),'run','--dir',str(copy),'--index','0','--height','1','--timeout','1'],capture_output=True,text=True,timeout=30)
            after = {str(p.relative_to(copy)):hashlib.sha256(p.read_bytes()).hexdigest() for p in copy.rglob('*') if p.is_file()}
            report['old_validator_fence'] = dict(passed=r.returncode!=0 and 'conflicting durable record' in r.stderr and before==after,
                exit=r.returncode, error=r.stderr.strip(),binary_sha256=hashlib.sha256(args.baseline_validator.read_bytes()).hexdigest())
            if not report['old_validator_fence']['passed']:
                raise RuntimeError('old binary bypassed format fence')
        faults = """import errno,hashlib,json,os,shutil,subprocess,sys
from pathlib import Path
migration=sys.argv[1]=='migration';inode=sys.argv[2]=='inode'
shutil.copytree('/source','/store',dirs_exist_ok=True)
def hashes():return {str(p.relative_to('/store')):hashlib.sha256(p.read_bytes()).hexdigest() for p in Path('/store').rglob('*') if p.is_file() and p.name not in ('store.lock','validator-index.bin') and not p.name.startswith(('signing-','fill-'))}
def all_hashes():return {str(p.relative_to('/store')):hashlib.sha256(p.read_bytes()).hexdigest() for p in Path('/store').rglob('*') if p.is_file() and not p.name.startswith('fill-')}
before=hashes();all_before=all_hashes();fillers=[]
if inode:
 while os.statvfs('/store').f_favail > (129 if migration else 127):
  p=Path('/store')/('fill-'+str(len(fillers)));p.touch();fillers.append(p)
else:
 p=Path('/store/fill-blocks');fillers.append(p)
 with p.open('wb',buffering=0) as f:
  if migration:
   while os.statvfs('/store').f_bavail*os.statvfs('/store').f_frsize > 64*1024*1024+512*1024:f.write(bytes(65536))
  else:
   try:
    while True:f.write(bytes(65536))
   except OSError as e:
    if e.errno!=errno.ENOSPC:raise
command=['/bench','migrate' if migration else 'probe','/store','8192' if migration else '1']
first=subprocess.run(command,capture_output=True,text=True,timeout=180)
if migration:
 first_ok=first.returncode!=0 and ('reserve exhausted' in first.stderr) and not Path('/store/signing-active.bin').exists() and before==hashes()
else:first_ok=first.returncode==0 and json.loads(first.stdout)['passed']
full=False
if inode:
 try:
  while True:
   p=Path('/store')/('fill-'+str(len(fillers)));p.touch();fillers.append(p)
 except OSError as e:full=e.errno==errno.ENOSPC
else:
 with Path('/store/fill-blocks').open('ab',buffering=0) as f:
  try:
   while True:f.write(bytes(65536))
  except OSError as e:full=e.errno==errno.ENOSPC
second=subprocess.run(command,capture_output=True,text=True,timeout=180)
second_ok=(second.returncode!=0 and not Path('/store/signing-active.bin').exists()) if migration else (second.returncode==0 and json.loads(second.stdout)['passed'])
for p in fillers:p.unlink()
retained=before==hashes()
if migration:
 resumed=subprocess.run(command,capture_output=True,text=True,timeout=180)
 resume_ok=resumed.returncode==0 and json.loads(resumed.stdout)['passed'] and json.loads(resumed.stdout)['legacy_files_retained']==8192
else:resume_ok=all_before==all_hashes()
print(json.dumps(dict(passed=first_ok and second_ok and full and retained and resume_ok,kind=sys.argv[1]+'-'+sys.argv[2],enospc_observed=full,first_refused_safely=first_ok,second_refused_safely=second_ok,legacy_and_nonsigning_bytes_preserved=retained,resume_or_unchanged=resume_ok,first_error=first.stderr.strip(),second_error=second.stderr.strip())))
"""
        report['faults'] = []
        for operation in ['append','migration']:
            for resource in ['block','inode']:
                source = root/('legacy-pressure-source' if operation == 'migration' else 'pressure-seed')
                options = f'/store:size=256m,uid={os.getuid()},gid={os.getgid()}'
                if resource == 'inode':
                    options += ',nr_inodes=16384' if operation == 'migration' else ',nr_inodes=1024'
                data = run(cli+common+['--tmpfs',options,'--mount',f'type=bind,src={source},dst=/source,readonly',
                    '--mount',f'type=bind,src={binary},dst=/bench,readonly',IMAGE,'python','-c',faults,operation,resource],timeout=600)
                report['faults'].append(data)
                if not data['passed']:
                    raise RuntimeError(f'filesystem fault failed: {data}')
        report['passed'] = True
    finally:
        (root/'report.json').write_text(json.dumps(report,indent=2)+'\n')


if __name__ == '__main__':
    main()
