#!/usr/bin/env python3
"""Concurrent large-state cold recovery with independent anchors and import crashes."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
IMAGE = 'python:3.13-slim-trixie@sha256:ae14d4ce8771606477a81446e7ef429edd6762305fc50c84507c1041fa8285d1'
WORKER = '''import hashlib,json,os,resource,subprocess,sys,time
from pathlib import Path
binary='/validator';anchor=sys.argv[1];height,block,root=anchor.split(':');directory=Path('/results')
def command(kind,i,extra=()):return [binary,kind,'--dir',str(directory/('reader-'+str(i))),*extra]
def run(kind,i,extra=(),code=0):
 r=subprocess.run(command(kind,i,extra),capture_output=True,text=True,timeout=120)
 assert r.returncode==code,(kind,r.returncode,r.stderr)
 return json.loads(r.stdout.splitlines()[-1]) if code==0 else None
for i in range(4):
 extra=['--snapshot','/snapshot','--anchor',anchor]
 if i==0:run('import-checkpoint',i,extra+['--storage-crash','base'],86)
 state=run('import-checkpoint',i,extra)
 assert state['height']==int(height) and state['block']==block and state['state_root']==root
before={str(p.relative_to(directory)):hashlib.sha256(p.read_bytes()).hexdigest() for p in directory.rglob('*') if p.is_file() and p.name!='store.lock'}
rounds=[];peak_rss=0;started=time.monotonic()
for cycle in range(3):
 at=time.monotonic();children=[subprocess.Popen(command('inspect',i),stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True) for i in range(4)]
 while any(p.poll() is None for p in children):
  rss=0
  for p in children:
   try:rss+=int(Path('/proc/'+str(p.pid)+'/statm').read_text().split()[1])*os.sysconf('SC_PAGE_SIZE')
   except (FileNotFoundError,ProcessLookupError):pass
  peak_rss=max(peak_rss,rss);time.sleep(.02)
 states=[]
 for p in children:
  out,err=p.communicate(timeout=10);assert p.returncode==0,err;state=json.loads(out.splitlines()[-1]);assert state['height']==int(height) and state['block']==block and state['state_root']==root;states.append(state)
 rounds.append(dict(seconds=time.monotonic()-at,states=states))
after={str(p.relative_to(directory)):hashlib.sha256(p.read_bytes()).hexdigest() for p in directory.rglob('*') if p.is_file() and p.name!='store.lock'}
assert before==after
bad=bytearray(Path('/snapshot').read_bytes());bad[-1]^=1;(directory/'hostile.bin').write_bytes(bad)
r=subprocess.run(command('import-checkpoint','hostile',['--snapshot',str(directory/'hostile.bin'),'--anchor',anchor]),capture_output=True,text=True,timeout=120)
assert r.returncode!=0 and not (directory/'reader-hostile'/'compact-base.bin').exists()
u=resource.getrusage(resource.RUSAGE_CHILDREN)
print(json.dumps(dict(passed=True,anchor=anchor,rounds=rounds,crash_import_resumed=True,committed_files_unchanged=True,hostile_snapshot_refused=True,cpu_user_seconds=u.ru_utime,cpu_system_seconds=u.ru_stime,sampled_combined_peak_rss_bytes=peak_rss,container_memory_peak_bytes=int(Path('/sys/fs/cgroup/memory.peak').read_text()),concurrent_recovery_seconds=time.monotonic()-started)))
'''
def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,required=True);p.add_argument('--snapshot',type=Path,required=True);p.add_argument('--anchor',required=True);p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();root=a.output.resolve();root.mkdir(parents=True,exist_ok=False);binary=root/'validator';shutil.copy2(a.binary,binary)
    env=os.environ.copy()
    for k in ['DOCKER_HOST','DOCKER_CONTEXT','DOCKER_TLS','DOCKER_TLS_VERIFY','DOCKER_CERT_PATH']:env.pop(k,None)
    r=subprocess.run(['docker','--host=unix:///var/run/docker.sock','run','--rm','--network','none','--cpus','2','--memory','2g','--memory-swap','2g','--pids-limit','128','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--user',f'{os.getuid()}:{os.getgid()}','--mount',f'type=bind,src={binary},dst=/validator,readonly','--mount',f'type=bind,src={a.snapshot.resolve()},dst=/snapshot,readonly','--mount',f'type=bind,src={root},dst=/results',IMAGE,'python','-c',WORKER,a.anchor],env=env,capture_output=True,text=True,timeout=900)
    report=dict(passed=False,validator_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),snapshot_sha256=hashlib.sha256(a.snapshot.read_bytes()).hexdigest(),cpus=2,memory_bytes=2147483648)
    if r.returncode:report['error']=r.stderr
    else:report.update(json.loads(r.stdout))
    (root/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    if not report['passed']:raise RuntimeError(report)
    print(json.dumps(report))
if __name__=='__main__':main()
