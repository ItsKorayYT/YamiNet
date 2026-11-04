#!/usr/bin/env python3
"""Real Malachite histories with distinct signed naming accounts; private Docker only."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
from yamichain_safety import capture_safety, preserved, observed_anchor
IMAGE = 'python:3.13-slim-trixie@sha256:ae14d4ce8771606477a81446e7ef429edd6762305fc50c84507c1041fa8285d1'
WRAPPER = '''import json,resource,subprocess,sys,time
from pathlib import Path
p=Path('/measure');attempt=sys.argv[1];start=time.time()
with (p/('stderr-'+attempt)).open('w') as err,(p/'finalized.jsonl').open('a',buffering=1) as out:
 child=subprocess.Popen(['/validator',*sys.argv[2:]],stdout=subprocess.PIPE,stderr=err,text=True)
 for line in child.stdout:
  try:d=json.loads(line)
  except ValueError:continue
  if 'height' in d:out.write(json.dumps(dict(at=time.time(),state=d))+'\\n')
 code=child.wait()
u=resource.getrusage(resource.RUSAGE_CHILDREN);c=Path('/sys/fs/cgroup')
(p/('usage-'+attempt+'.json')).write_text(json.dumps(dict(code=code,wall_seconds=time.time()-start,cpu_user=u.ru_utime,cpu_system=u.ru_stime,peak_rss_kib=u.ru_maxrss,memory_peak=int((c/'memory.peak').read_text()),cpu_max=(c/'cpu.max').read_text().strip(),memory_max=(c/'memory.max').read_text().strip())))
sys.exit(code)
'''
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--height',type=int,default=1640);p.add_argument('--fault-height',type=int,default=1563);p.add_argument('--timeout',type=int,default=3300);p.add_argument('--cpuset',default=None);a=p.parse_args()
 if not 1<=a.fault_height<a.height<=100000 or not 1<=a.timeout<=3600:p.error('invalid height/deadline')
 root=a.output.resolve();root.mkdir(parents=True,exist_ok=False);binary=root/'validator';shutil.copy2(a.binary,binary)
 env=os.environ.copy()
 for k in ['DOCKER_HOST','DOCKER_CONTEXT','DOCKER_TLS','DOCKER_TLS_VERIFY','DOCKER_CERT_PATH']:env.pop(k,None)
 cli=['docker','--host=unix:///var/run/docker.sock'];tag='yami-large-'+str(os.getpid());names=[tag+'-'+str(i) for i in range(4)];report=dict(passed=False,live_engine_generated=True,distinct_accounts=True,cpus_per_node=2,memory_bytes_per_node=2147483648,binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),events=[],samples=[]);started=time.monotonic();attempt=[0]*4
 def docker(*args,check=True):
  r=subprocess.run(cli+list(args),env=env,capture_output=True,text=True,timeout=40)
  if check and r.returncode:raise RuntimeError(r.stderr)
  return r.stdout.strip()
 def local(command,i=0,extra=(),expected=0):
  at=time.monotonic();r=subprocess.run([str(binary),command,'--dir',str(root/f'store-{i}'),'--isolated-network',str(root/f'config-{i}'),'--index',str(i),*extra],capture_output=True,text=True,timeout=600)
  assert r.returncode==expected,(command,r.returncode,r.stderr)
  return (json.loads(r.stdout.splitlines()[-1]) if expected==0 else None,time.monotonic()-at)
 offsets=[0]*4;history=[[] for _ in range(4)]
 def records(i):
  p=root/f'measure-{i}'/'finalized.jsonl'
  if p.exists():
   with p.open() as f:
    f.seek(offsets[i])
    while True:
     start=f.tell();line=f.readline()
     if not line or not line.endswith('\n'):f.seek(start);break
     history[i].append(json.loads(line))
    offsets[i]=f.tell()
  return history[i]
 def heights():return [r[-1]['state']['height'] if (r:=records(i)) else 0 for i in range(4)]
 def event(kind,**kw):report['events'].append(dict(at=time.monotonic()-started,kind=kind,heights=heights(),**kw));(root/'progress.json').write_text(json.dumps(report['events'],indent=2)+'\n')
 def launch(i):
  attempt[i]+=1;anchor=observed_anchor(root/f'store-{i}');extra=['--signing-anchor',anchor] if anchor else []
  docker('run',*(['--cpuset-cpus',a.cpuset] if a.cpuset else []),'-d','--name',names[i],'--network',tag,'--ip',f'10.84.0.{i+2}','--cpus','2','--memory','2g','--memory-swap','2g','--pids-limit','256','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--user',f'{os.getuid()}:{os.getgid()}','--mount',f'type=bind,src={binary},dst=/validator,readonly','--mount',f'type=bind,src={root/f"config-{i}"},dst=/config,readonly','--mount',f'type=bind,src={root/f"store-{i}"},dst=/store','--mount',f'type=bind,src={root/f"measure-{i}"},dst=/measure',IMAGE,'python','-u','-c',WRAPPER,str(attempt[i]),'run','--dir','/store','--index',str(i),'--isolated-network','/config','--height',str(a.height),'--timeout',str(a.timeout),'--linger','30','--test-transactions','64','--test-distinct-accounts','true',*extra)
 network=False
 try:
  subprocess.run([str(binary),'init-isolated','--dir',str(root/'provision'),'--endpoints',','.join(f'10.84.0.{i+2}:28100' for i in range(4))],check=True)
  for i in range(4):
   cfg=root/f'config-{i}';cfg.mkdir(mode=0o700)
   for f in ['roster.bin','bootstrap-0.bin','bootstrap-1.bin',f'identity-{i}.bin']:shutil.copy2(root/'provision'/f,cfg/f)
   (root/f'store-{i}').mkdir();(root/f'measure-{i}').mkdir();local('migrate-archive',i)
  docker('network','create','--internal','--subnet','10.84.0.0/24',tag);network=True
  for i in range(4):launch(i)
  partitioned=False;crashed=False;healed=False;crash_before=None;last_sample=0
  while time.monotonic()-started<a.timeout+180:
   h=heights()
   if min(h)>=a.fault_height and not partitioned:
    for i in [0,1]:docker('network','disconnect',tag,names[i])
    event('quorum_loss_two_disconnected');partitioned=True;partition_at=time.monotonic();partition_h=h
   if partitioned and not healed and time.monotonic()-partition_at>=12:
    now=heights();assert max(now)<=max(partition_h)+2,(partition_h,now)
    for i in [0,1]:docker('network','connect','--ip',f'10.84.0.{i+2}',tag,names[i])
    event('partition_healed',maximum_advance=max(now)-max(partition_h));healed=True
   if healed and not crashed and min(h)>=a.fault_height+4:
    crash_before=capture_safety(root/'store-0',live=True);anchor=observed_anchor(root/'store-0');docker('kill','--signal','KILL',names[0]);docker('rm',names[0]);event('actual_sigkill',independent_signing_anchor=anchor);crash_at=time.monotonic();crashed=True
   if crashed and attempt[0]==1 and (min(h[1:])>=a.fault_height+20 or time.monotonic()-crash_at>30):
    launch(0);event('restart_original_identity_with_original_wal_and_signing_logs')
   if time.monotonic()-last_sample>15:
    raw=docker('stats','--no-stream','--format','{{json .}}',*names,check=False);report['samples'].append(dict(at=time.monotonic()-started,heights=h,nodes=[json.loads(s) for s in raw.splitlines() if s.startswith('{')]));last_sample=time.monotonic();print(json.dumps(dict(elapsed=last_sample-started,heights=h)),flush=True)
   active=[n for n in names if not (n==names[0] and crashed and attempt[0]==1)];states=[json.loads(s) for s in docker('inspect','--format','{{json .State}}',*active).splitlines()]
   if states and all(not s['Running'] for s in states) and attempt[0]>1:
    assert all(s['ExitCode']==0 for s in states),states
    break
   if any(not s['Running'] and s['ExitCode']!=0 for s in states):raise RuntimeError(states)
   time.sleep(.4)
  else:raise TimeoutError('campaign deadline')
  assert crashed and healed and preserved(root/'store-0',crash_before)
  report['verified_states']=[];report['archive_replay_seconds']=[]
  for i in range(4):
   state,seconds=local('inspect',i);report['verified_states'].append(state);report['archive_replay_seconds'].append(seconds)
  states=report['verified_states'];assert len({(s['height'],s['block'],s['state_root'],s['test_tlds']) for s in states})==1 and states[0]['height']==a.height and states[0]['test_tlds']==64*a.height
  report['signing_prefix_preserved']=True
  report['usage']=[[json.loads(p.read_text()) for p in (root/f'measure-{i}').glob('usage-*.json')] for i in range(4)]
  report['finalized_intervals']=[]
  for i in range(4):
   seen={}
   for r in records(i):seen.setdefault(r['state']['height'],r['at'])
   pairs=sorted(seen.items());d=[b[1]-a[1] for a,b in zip(pairs,pairs[1:]) if b[0]==a[0]+1];terminal=sum(d[-100:])/min(100,len(d));d.sort();report['finalized_intervals'].append(dict(node=i,count=len(d),p50_seconds=d[len(d)//2],p95_seconds=d[int(len(d)*.95)],last_100_mean_seconds=terminal))
  report['passed']=True
 finally:
  report['elapsed_seconds']=time.monotonic()-started;(root/'report.json').write_text(json.dumps(report,indent=2)+'\n')
  for n in names:docker('rm','-f',n,check=False)
  if network:docker('network','rm',tag,check=False)
 print(json.dumps(dict(passed=True,report=str(root/'report.json'))))
if __name__=='__main__':main()
