#!/usr/bin/env python3
"""Signed/quorum-certified history profiles; never label fixture certificates live finality."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
IMAGE = 'python:3.13-slim-trixie@sha256:ae14d4ce8771606477a81446e7ef429edd6762305fc50c84507c1041fa8285d1'
BUILD = '''import json,resource,subprocess,sys,time
from pathlib import Path
at=time.monotonic();r=subprocess.run(['/bench','/results/state-'+sys.argv[1],sys.argv[1],'distinct'],capture_output=True,text=True,timeout=3600)
assert r.returncode==0,r.stderr
d=json.loads(r.stdout);u=resource.getrusage(resource.RUSAGE_CHILDREN);d.update(cpu_user_seconds=u.ru_utime,cpu_system_seconds=u.ru_stime,peak_rss_kib=u.ru_maxrss,wall_seconds=time.monotonic()-at,cgroup_cpu_max=Path('/sys/fs/cgroup/cpu.max').read_text().strip(),cgroup_memory_max=Path('/sys/fs/cgroup/memory.max').read_text().strip(),memory_peak_bytes=int(Path('/sys/fs/cgroup/memory.peak').read_text()));print(json.dumps(d))
'''
READERS = '''import json,os,resource,shutil,subprocess,sys,time
from pathlib import Path
c=json.loads(sys.argv[1]);v2=sys.argv[2]=='v2';args=[str(c['entries']),'distinct',str(c['height']),c['tip'],c['root']];commands=[]
for i in range(4):
 if v2:commands.append(['/bench','v2-recover','/source',*args,c['v2_manifest']])
 else:
  dest='/results/replay-'+str(c['entries'])+'-'+str(i);shutil.copytree('/source',dest);commands.append(['/bench','replay',dest,*args])
at=time.monotonic();ps=[subprocess.Popen(cmd,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True) for cmd in commands];peak=0
while any(p.poll() is None for p in ps):
 rss=0
 for p in ps:
  try:rss+=int(Path('/proc/'+str(p.pid)+'/statm').read_text().split()[1])*os.sysconf('SC_PAGE_SIZE')
  except (FileNotFoundError,ProcessLookupError):pass
 peak=max(peak,rss);time.sleep(.02)
for p in ps:
 out,err=p.communicate();assert p.returncode==0,err;d=json.loads(out);assert d['root']==c['root'] and d['names']==c['entries'] and d['height']==c['height']
u=resource.getrusage(resource.RUSAGE_CHILDREN);print(json.dumps(dict(passed=True,kind='v2' if v2 else 'full_archive',entries=c['entries'],processes=4,seconds=time.monotonic()-at,cpu_user_seconds=u.ru_utime,cpu_system_seconds=u.ru_stime,sampled_combined_peak_rss_bytes=peak,memory_peak_bytes=int(Path('/sys/fs/cgroup/memory.peak').read_text()))))
'''
PRESSURE = '''import errno,hashlib,json,os,shutil,subprocess,sys,time
from pathlib import Path
c=json.loads(sys.argv[1]);kind=sys.argv[2];dest=Path('/pressure/archive');shutil.copytree('/source',dest)
def hashes():return {p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in dest.iterdir() if p.is_file()}
before=hashes();fillers=[]
if kind=='blocks':
 p=Path('/pressure/filler');fillers.append(p)
 with p.open('wb',buffering=0) as f:
  try:
   while True:f.write(bytes(65536))
  except OSError as e:assert e.errno==errno.ENOSPC
else:
 for n in range(100000):
  p=Path('/pressure/fill-'+str(n))
  try:p.touch();fillers.append(p)
  except OSError as e:assert e.errno==errno.ENOSPC;break
 else:raise AssertionError('inode pressure not reached')
cmd=['/validator','export-checkpoint','--dir',str(dest),'--committee-plan','1:4,'+str(c['future_epoch'])+':5','--out','/pressure/failed.bin']
r=subprocess.run(cmd,capture_output=True,text=True,timeout=300)
assert r.returncode!=0 and ('os error 28' in r.stderr or 'No space left' in r.stderr),r.stderr
failed_error=r.stderr.strip()
assert not Path('/pressure/failed.bin').exists() and hashes()==before
for p in fillers:p.unlink()
cmd[-1]='/pressure/complete.bin';r=subprocess.run(cmd,capture_output=True,text=True,timeout=300);assert r.returncode==0,r.stderr;d=json.loads(r.stdout);assert d['height']==c['height'] and d['state_root']==c['root'] and hashes()==before
print(json.dumps(dict(passed=True,entries=c['entries'],kind=kind,errno=28,source_bytes_unchanged=True,retry_verified=True,error=failed_error)))
'''
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',type=Path,required=True);p.add_argument('--validator',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--sizes',type=int,nargs='+',default=[100000,250000,500000]);p.add_argument('--cpuset',default=None);a=p.parse_args()
 root=a.output.resolve();root.mkdir(parents=True,exist_ok=False);binary=root/'bench';shutil.copy2(a.binary,binary);validator=root/'validator';shutil.copy2(a.validator,validator)
 env=os.environ.copy()
 for k in ['DOCKER_HOST','DOCKER_CONTEXT','DOCKER_TLS','DOCKER_TLS_VERIFY','DOCKER_CERT_PATH']:env.pop(k,None)
 cli=['docker','--host=unix:///var/run/docker.sock'];common=['run',*(['--cpuset-cpus',a.cpuset] if a.cpuset else []),'--rm','--network','none','--cpus','2','--memory','2g','--memory-swap','2g','--pids-limit','128','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--user',f'{os.getuid()}:{os.getgid()}','--mount',f'type=bind,src={binary},dst=/bench,readonly','--mount',f'type=bind,src={validator},dst=/validator,readonly','--mount',f'type=bind,src={root},dst=/results']
 report=dict(passed=False,cpus=2,memory_bytes=2147483648,synthetic_snapshots=False,live_engine_generated=False,binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),validator_sha256=hashlib.sha256(validator.read_bytes()).hexdigest(),cases=[],concurrent=[],attacks=[],enospc=[])
 def docker(extra,program,*args):
  r=subprocess.run(cli+common+extra+[IMAGE,'python','-c',program,*args],env=env,capture_output=True,text=True,timeout=3700)
  if r.returncode:raise RuntimeError(r.stderr+r.stdout)
  return json.loads(r.stdout)
 try:
  for n in a.sizes:
   d=docker([],BUILD,str(n));report['cases'].append(d);print(json.dumps(d),flush=True)
  for c in [report['cases'][0],report['cases'][-1]]:
   source=root/f"state-{c['entries']}"/'v2';d=docker(['--mount',f'type=bind,src={source},dst=/source,readonly'],READERS,json.dumps(c),'v2');report['concurrent'].append(d);print(json.dumps(d),flush=True)
  c=report['cases'][0];source=root/f"state-{c['entries']}"/'archive';mount=['--mount',f'type=bind,src={source},dst=/source,readonly']
  d=docker(mount,READERS,json.dumps(c),'archive');report['concurrent'].append(d);print(json.dumps(d),flush=True)
  for kind in ['blocks','inodes']:
   d=docker(mount+['--tmpfs',f'/pressure:size=128m,nr_inodes=2048,uid={os.getuid()},gid={os.getgid()}'],PRESSURE,json.dumps(c),kind);report['enospc'].append(d);print(json.dumps(d),flush=True)
  assert (source/'validator-index.bin').read_bytes()==bytes([99]),'fixture observer fence required'
  for index in [0,4,99]:
   r=subprocess.run([str(validator),'run','--dir',str(source),'--index',str(index),'--committee-plan',f"1:4,{c['future_epoch']}:5",'--height',str(c['height']+1),'--timeout','1'],capture_output=True,text=True,timeout=15)
   assert r.returncode!=0 and ('binding conflict' in r.stderr or 'invalid local options' in r.stderr) and not (source/'wal').exists(),r.stderr
  report['fixture_cannot_be_live_identity']=True
  for kind in ['corrupt','missing','cross-context']:
   dest=root/('hostile-'+kind);shutil.copytree(source,dest)
   segments=sorted(dest.glob('archive-segment-*.bin'));assert segments
   if kind=='corrupt':
    b=bytearray(segments[-1].read_bytes());b[-1]^=1;segments[-1].write_bytes(b)
   if kind=='missing':segments[-1].unlink()
   pin=report['cases'][-1] if kind=='cross-context' and len(report['cases'])>1 else c
   r=subprocess.run([str(binary),'replay',str(dest),str(pin['entries']),'distinct',str(pin['height']+(1 if kind=='cross-context' and len(report['cases'])==1 else 0)),pin['tip'],pin['root']],capture_output=True,text=True,timeout=300)
   assert r.returncode!=0 and '"root"' not in r.stdout,(kind,r.stdout);report['attacks'].append(dict(kind=kind,rejected=True,error=r.stderr.strip()))
  prefix=root/'historical-prefix'
  r=subprocess.run([str(binary),'prefix',str(source),str(prefix),str(c['entries']),'distinct',str(c['height']-1)],capture_output=True,text=True,timeout=300)
  assert r.returncode==0,r.stderr;older=json.loads(r.stdout)
  r=subprocess.run([str(binary),'replay',str(prefix),str(c['entries']),'distinct',str(c['height']),c['tip'],c['root']],capture_output=True,text=True,timeout=300)
  assert r.returncode!=0 and 'independent archive anchor mismatch' in r.stderr and not r.stdout
  report['attacks'].append(dict(kind='same-chain-rollback',rejected=True,valid_certified_prefix=older,error=r.stderr.strip()))
  report['passed']=True
 finally:(root/'report.json').write_text(json.dumps(report,indent=2)+'\n')
 print(json.dumps(dict(passed=True,report=str(root/'report.json'))))
if __name__=='__main__':main()
