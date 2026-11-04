#!/usr/bin/env python3
"""Isolated snapshot-v2 capacity, adversarial transfer and cold recovery laboratory."""
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
started=time.monotonic();r=subprocess.run(['/bench','build','/results/state-'+sys.argv[1],sys.argv[1]],capture_output=True,text=True,timeout=7200)
if r.returncode:print(r.stderr,file=sys.stderr);sys.exit(r.returncode)
d=json.loads(r.stdout);u=resource.getrusage(resource.RUSAGE_CHILDREN);d.update(cpu_user_seconds=u.ru_utime,cpu_system_seconds=u.ru_stime,peak_rss_kib=u.ru_maxrss,wall_seconds=time.monotonic()-started,container_memory_peak_bytes=int(Path('/sys/fs/cgroup/memory.peak').read_text()),cgroup_cpu_max=Path('/sys/fs/cgroup/cpu.max').read_text().strip(),cgroup_memory_max=Path('/sys/fs/cgroup/memory.max').read_text().strip());print(json.dumps(d))
'''
READERS = '''import json,os,resource,subprocess,sys,time
from pathlib import Path
anchor=json.loads(sys.argv[1]);args=[str(anchor['height']),anchor['tip'],anchor['root'],anchor['manifest']];peak=0;rounds=[]
for _ in range(3):
 started=time.monotonic();children=[subprocess.Popen(['/bench','recover','/source',*args],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True) for i in range(4)]
 while any(p.poll() is None for p in children):
  rss=0
  for p in children:
   try:rss+=int(Path('/proc/'+str(p.pid)+'/statm').read_text().split()[1])*os.sysconf('SC_PAGE_SIZE')
   except (FileNotFoundError,ProcessLookupError):pass
  peak=max(peak,rss);time.sleep(.02)
 for p in children:
  out,err=p.communicate(timeout=10);assert p.returncode==0,err;d=json.loads(out)
  assert all(d[k]==anchor[k] for k in ['height','tip','root','manifest','accounts']) and d['names']==anchor['entries'] and d['experimental_only']
 rounds.append(time.monotonic()-started)
u=resource.getrusage(resource.RUSAGE_CHILDREN);print(json.dumps(dict(passed=True,entries=anchor['entries'],processes=4,round_seconds=rounds,sampled_combined_peak_rss_bytes=peak,container_memory_peak_bytes=int(Path('/sys/fs/cgroup/memory.peak').read_text()),cpu_user_seconds=u.ru_utime,cpu_system_seconds=u.ru_stime)))
'''
def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,required=True);p.add_argument('--test-binary',type=Path)
    p.add_argument('--production-validator',type=Path);p.add_argument('--output',type=Path,required=True)
    p.add_argument('--sizes',type=int,nargs='+',default=[100000,250000,500000]);a=p.parse_args()
    root=a.output.resolve();root.mkdir(parents=True,exist_ok=False);binary=root/'bench';shutil.copy2(a.binary,binary)
    env=os.environ.copy()
    for k in ['DOCKER_HOST','DOCKER_CONTEXT','DOCKER_TLS','DOCKER_TLS_VERIFY','DOCKER_CERT_PATH']:env.pop(k,None)
    cli=['docker','--host=unix:///var/run/docker.sock']
    common=['run','--rm','--network','none','--cpus','2','--memory','2g','--memory-swap','2g','--pids-limit','128','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--user',f'{os.getuid()}:{os.getgid()}']
    mounts=['--mount',f'type=bind,src={binary},dst=/bench,readonly','--mount',f'type=bind,src={root},dst=/results']
    report=dict(passed=False,binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),cpus=2,memory_bytes=2147483648,experimental_only=True,bft_finality_claimed=False,cases=[],concurrent=[],attacks=[],crashes=[],enospc=[])
    def docker(extra,program,*args,timeout=7300):
        r=subprocess.run(cli+common+mounts+extra+[IMAGE,'python','-c',program,*args],env=env,capture_output=True,text=True,timeout=timeout)
        if r.returncode:raise RuntimeError(r.stderr or r.stdout)
        return json.loads(r.stdout)
    def call(command,expected=0):
        r=subprocess.run([str(binary),*map(str,command)],capture_output=True,text=True,timeout=180)
        if r.returncode!=expected:raise RuntimeError((command,r.returncode,r.stderr))
        return json.loads(r.stdout) if expected==0 else None
    def pins(c):return [str(c['height']),c['tip'],c['root'],c['manifest']]
    try:
        for n in a.sizes:
            d=docker([],BUILD,str(n));report['cases'].append(d);print(json.dumps(d),flush=True)
            source=root/f'state-{n}'/'received'
            r=docker(['--mount',f'type=bind,src={source},dst=/source,readonly'],READERS,json.dumps(d));report['concurrent'].append(r);print(json.dumps(r),flush=True)
        first=report['cases'][0];source=root/f"state-{first['entries']}"/'source'
        for name,stage in [('state.bin','Synced'),('anchor.bin','Renamed'),('chunk-000000.bin','Synced'),('chunk-000001.bin','Renamed'),('manifest.bin','DirectorySynced'),('complete.bin','Renamed')]:
            dest=root/f'crash-{name}-{stage}'
            call(['transfer',source,dest,*pins(first),name,stage],86)
            result=call(['transfer',source,dest,*pins(first)]);assert result['root']==first['root'];report['crashes'].append(dict(file=name,stage=stage,resumed=True))
        for mode in ['corrupt','missing','duplicate','tail','manifest','pending-conflict','rollback']:
            dest=root/f'attack-{mode}';shutil.copytree(source,dest)
            chunk=dest/'chunk-000001.bin'
            if mode=='corrupt':
                b=bytearray(chunk.read_bytes());b[20]^=1;chunk.write_bytes(b)
            elif mode=='missing':chunk.unlink()
            elif mode=='duplicate':shutil.copyfile(dest/'chunk-000000.bin',chunk)
            elif mode=='tail':
                with chunk.open('ab') as f:f.write(b'unknown')
            elif mode=='manifest':
                b=bytearray((dest/'manifest.bin').read_bytes());b[-1]^=1;(dest/'manifest.bin').write_bytes(b)
            elif mode=='pending-conflict':(dest/'.pending-chunk-000001.bin').write_bytes(b'unknown')
            args=pins(first)
            if mode=='rollback':args=pins(report['cases'][-1]) if len(report['cases'])>1 else [str(first['height']+1),*args[1:]]
            r=subprocess.run([str(binary),'recover',str(dest),*args],capture_output=True,text=True,timeout=180)
            assert r.returncode!=0,(mode,r.stdout);report['attacks'].append(dict(mode=mode,rejected=True));shutil.rmtree(dest)
        if a.production_validator:
            before={f.name:hashlib.sha256(f.read_bytes()).hexdigest() for f in source.iterdir()}
            r=subprocess.run([str(a.production_validator.resolve()),'run','--dir',str(source),'--index','0','--height','1','--timeout','1'],capture_output=True,text=True,timeout=30)
            after={f.name:hashlib.sha256(f.read_bytes()).hexdigest() for f in source.iterdir()}
            assert r.returncode!=0 and before==after and 'experimental' in r.stderr
            report['production_journal_refused_without_writes']=True
        if a.test_binary:
            test=root/'unit-tests';shutil.copy2(a.test_binary,test);report['test_binary_sha256']=hashlib.sha256(test.read_bytes()).hexdigest()
            for kind in ['blocks','inodes']:
                r=subprocess.run(cli+common+['--tmpfs',f'/store:size=128m,nr_inodes=2048,uid={os.getuid()},gid={os.getgid()}','--mount',f'type=bind,src={test},dst=/tests,readonly','-e','YAMI_V2_ENOSPC=/store','-e',f'YAMI_V2_PRESSURE={kind}',IMAGE,'/tests','--exact','snapshot_v2::tests::actual_enospc_child','--test-threads=1','--nocapture'],env=env,capture_output=True,text=True,timeout=120)
                assert r.returncode==0 and '1 passed; 0 failed' in r.stdout,r.stderr+r.stdout
                report['enospc'].append(dict(kind=kind,passed=True,stdout=r.stdout))
        report['passed']=True
    finally:(root/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(dict(passed=True,report=str(root/'report.json'))))
if __name__=='__main__':main()
