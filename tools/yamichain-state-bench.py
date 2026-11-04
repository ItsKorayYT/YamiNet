#!/usr/bin/env python3
"""Profile deterministic application state in sequential 2-CPU / 2-GiB containers."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
IMAGE = 'python:3.13-slim-trixie@sha256:ae14d4ce8771606477a81446e7ef429edd6762305fc50c84507c1041fa8285d1'
MEASURE = '''import json,resource,subprocess,sys,time
from pathlib import Path
started=time.monotonic()
r=subprocess.run(['/bench','/results/state-'+sys.argv[1]+'-'+sys.argv[2],sys.argv[1],sys.argv[2]],capture_output=True,text=True,timeout=7200)
if r.returncode:print(r.stderr,file=sys.stderr);sys.exit(r.returncode)
d=json.loads(r.stdout);u=resource.getrusage(resource.RUSAGE_CHILDREN)
d.update(cpu_user_seconds=u.ru_utime,cpu_system_seconds=u.ru_stime,peak_rss_kib=u.ru_maxrss,wall_seconds=time.monotonic()-started,cgroup_cpu_max=Path('/sys/fs/cgroup/cpu.max').read_text().strip(),cgroup_memory_max=Path('/sys/fs/cgroup/memory.max').read_text().strip())
print(json.dumps(d))
'''
def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--sizes',type=int,nargs='+',default=[1024,10000,50000,100000])
    p.add_argument('--patterns',choices=['shared','distinct'],nargs='+',default=['shared','distinct'])
    a=p.parse_args();root=a.output.resolve();root.mkdir(parents=True,exist_ok=False)
    binary=root/'bench';shutil.copy2(a.binary,binary)
    env=os.environ.copy()
    for key in ['DOCKER_HOST','DOCKER_CONTEXT','DOCKER_TLS','DOCKER_TLS_VERIFY','DOCKER_CERT_PATH']:env.pop(key,None)
    report=dict(binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),cpus=2,memory_bytes=2147483648,passed=False,cases=[])
    try:
        for pattern in a.patterns:
            for size in a.sizes:
                r=subprocess.run(['docker','--host=unix:///var/run/docker.sock','run','--rm','--network','none','--cpus','2','--memory','2g','--memory-swap','2g','--pids-limit','128','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--user',f'{os.getuid()}:{os.getgid()}','--mount',f'type=bind,src={binary},dst=/bench,readonly','--mount',f'type=bind,src={root},dst=/results',IMAGE,'python','-c',MEASURE,str(size),pattern],env=env,capture_output=True,text=True,timeout=7300)
                if r.returncode:raise RuntimeError(r.stderr)
                d=json.loads(r.stdout);report['cases'].append(d);print(json.dumps(d),flush=True)
        report['passed']=True
    finally:(root/'report.json').write_text(json.dumps(report,indent=2)+'\n')
if __name__=='__main__':main()
