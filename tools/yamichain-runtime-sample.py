#!/usr/bin/env python3
"""Sample an existing supervisor tree; no workload or low-resource claims implied."""
import argparse,json,os,pathlib,time
p=argparse.ArgumentParser();p.add_argument('--pid',type=int,required=True);p.add_argument('--seconds',type=int,default=10);a=p.parse_args()
if not 1<=a.seconds<=60:raise SystemExit('sample duration must be 1..60 seconds')
clock=os.sysconf('SC_CLK_TCK');page=os.sysconf('SC_PAGE_SIZE');cpu={};peak_rss=peak_pss=peak_count=0;complete=True
start=time.monotonic()
while time.monotonic()-start<a.seconds:
 procs={}
 for path in pathlib.Path('/proc').iterdir():
  if path.name.isdigit():
   try:
    fields=(path/'stat').read_text().split(') ',1)[1].split();procs[int(path.name)]=(int(fields[1]),fields,path)
   except (OSError,ValueError):pass
 tree={a.pid};changed=True
 while changed:
  changed=False
  for pid,(parent,fields,path) in procs.items():
   if parent in tree and pid not in tree:tree.add(pid);changed=True
 rss=pss=0
 for pid in tree:
  if pid not in procs:complete=False;continue
  parent,f,path=procs[pid];ticks=(int(f[11])+int(f[12]))/clock
  # Process start time prevents PID reuse from turning into negative accounting.
  identity=(pid,f[19]);first,last=cpu.get(identity,(ticks,ticks));cpu[identity]=(first,ticks)
  rss+=int(f[21])*page
  try:
   line=next(v for v in (path/'smaps_rollup').read_text().splitlines() if v.startswith('Pss:'));pss+=int(line.split()[1])*1024
  except (OSError,StopIteration):complete=False
 peak_rss=max(peak_rss,rss);peak_pss=max(peak_pss,pss);peak_count=max(peak_count,len(tree));time.sleep(.5)
elapsed=time.monotonic()-start
print(json.dumps(dict(duration_seconds=elapsed,cpu_seconds_observed=sum(last-first for first,last in cpu.values()),peak_sum_rss_bytes=peak_rss,peak_sum_pss_bytes=peak_pss,peak_process_count=peak_count,observations_complete=complete,workload='idle supervised TLS devnet; no throughput benchmark',production_readiness_claim=False),indent=2))
