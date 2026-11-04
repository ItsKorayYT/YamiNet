#!/usr/bin/env python3
"""Restart the ORIGINAL large-state validators with their WAL/signing/archive minima."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import runpy
import shutil
import subprocess
import time
from yamichain_safety import capture_safety, preserved, observed_anchor
LAB=runpy.run_path(str(Path(__file__).with_name('yamichain-large-live.py')))
IMAGE=LAB['IMAGE'];WRAPPER=LAB['WRAPPER']
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',type=Path,required=True);p.add_argument('--source',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--anchor',required=True);p.add_argument('--step',type=int,default=32);p.add_argument('--cpuset',default=None);a=p.parse_args()
 height,block,state=a.anchor.split(':');height=int(height);assert 1<=a.step<=128
 source=a.source.resolve();root=a.output.resolve();root.mkdir(parents=True,exist_ok=False);binary=root/'validator';shutil.copy2(a.binary,binary);target=height+a.step
 env=os.environ.copy()
 for k in ['DOCKER_HOST','DOCKER_CONTEXT','DOCKER_TLS','DOCKER_TLS_VERIFY','DOCKER_CERT_PATH']:env.pop(k,None)
 cli=['docker','--host=unix:///var/run/docker.sock'];tag='yami-large-followup-'+str(os.getpid());names=[tag+'-'+str(i) for i in range(4)]
 def docker(*args,check=True):
  r=subprocess.run(cli+list(args),env=env,capture_output=True,text=True,timeout=40)
  if check and r.returncode:raise RuntimeError(r.stderr)
  return r.stdout.strip()
 report=dict(passed=False,binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),minimum_finalized_anchor=a.anchor,target_height=target,cpus_per_node=2,memory_bytes_per_node=2147483648,cpuset=a.cpuset,original_identity_and_safety_volumes=True,minima=[],finality_observations=[]);network=False;started=time.monotonic()
 before=[capture_safety(source/f'store-{i}',live=True) for i in range(4)]
 try:
  # Independent input was recorded by the preceding authenticated full campaign.
  original=json.loads((source/'report.json').read_text());assert original['passed']
  for s in original['verified_states']:assert (s['height'],s['block'],s['state_root'])==(height,block,state)
  docker('network','create','--internal','--subnet','10.84.0.0/24',tag);network=True
  for i in range(4):
   store=source/f'store-{i}';cfg=source/f'config-{i}';measure=root/f'measure-{i}';measure.mkdir()
   assert (store/'wal'/'consensus.wal').exists()
   # Query the authenticated existing archive through its API, not private offsets.
   stats=subprocess.run([str(binary),'migrate-archive','--dir',str(store),'--isolated-network',str(cfg),'--index',str(i)],capture_output=True,text=True,timeout=90)
   assert stats.returncode==0,stats.stderr;archive=json.loads(stats.stdout);assert archive['height']==height
   archive_anchor=str(height)+':'+archive['root'];signing_anchor=observed_anchor(store);assert signing_anchor
   report['minima'].append(dict(node=i,archive=archive_anchor,signing=signing_anchor))
   docker('run',*(['--cpuset-cpus',a.cpuset] if a.cpuset else []),'-d','--name',names[i],'--network',tag,'--ip',f'10.84.0.{i+2}','--cpus','2','--memory','2g','--memory-swap','2g','--pids-limit','256','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--user',f'{os.getuid()}:{os.getgid()}','--mount',f'type=bind,src={binary},dst=/validator,readonly','--mount',f'type=bind,src={cfg},dst=/config,readonly','--mount',f'type=bind,src={store},dst=/store','--mount',f'type=bind,src={measure},dst=/measure',IMAGE,'python','-u','-c',WRAPPER,'followup','run','--dir','/store','--index',str(i),'--isolated-network','/config','--height',str(target),'--timeout','300','--linger','5','--test-transactions','64','--test-distinct-accounts','true','--signing-anchor',signing_anchor,'--archive-anchor',archive_anchor)
  while time.monotonic()-started<360:
   states=[json.loads(s) for s in docker('inspect','--format','{{json .State}}',*names).splitlines()]
   if all(not s['Running'] for s in states):assert all(s['ExitCode']==0 for s in states),states;break
   if any(not s['Running'] and s['ExitCode']!=0 for s in states):raise RuntimeError(states)
   time.sleep(.5)
  else:raise TimeoutError('followup deadline')
  report['final_states']=[];report['usage']=[]
  for i in range(4):
   assert preserved(source/f'store-{i}',before[i])
   measure=root/f'measure-{i}';rows=[json.loads(s) for s in (measure/'finalized.jsonl').read_text().splitlines()];report['final_states'].append(rows[-1]['state']);report['usage'].append(json.loads((measure/'usage-followup.json').read_text()))
   for line in (measure/'stderr-followup').read_text().splitlines():
    try:d=json.loads(line)
    except ValueError:continue
    if d.get('lab_finality'):report['finality_observations'].append(dict(node=i,**d))
  assert len({(s['height'],s['block'],s['state_root']) for s in report['final_states']})==1 and report['final_states'][0]['height']==target
  for origin in ['local_builder_start','authenticated_admission']:
   samples=sorted(o['proposal_to_finalized_seconds'] for o in report['finality_observations'] if o['origin']==origin);assert len(samples)>=a.step//2 and min(samples)>=0
   report[origin]=dict(count=len(samples),p50_seconds=samples[len(samples)//2],p95_seconds=samples[int(len(samples)*.95)],max_seconds=max(samples))
  report['signing_prefixes_preserved']=True;report['passed']=True
 finally:
  report['elapsed_seconds']=time.monotonic()-started;(root/'report.json').write_text(json.dumps(report,indent=2)+'\n')
  for n in names:docker('rm','-f',n,check=False)
  if network:docker('network','rm',tag,check=False)
 print(json.dumps(report))
if __name__=='__main__':main()
