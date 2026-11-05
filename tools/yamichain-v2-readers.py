#!/usr/bin/env python3
"""Fresh-process inactive recovery from independently retained local fixture evidence."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import runpy
import shutil
import subprocess

IMAGE = runpy.run_path(str(Path(__file__).with_name('yamichain-v2-integration.py')))['IMAGE']
READERS = '''import json,os,resource,subprocess,sys,time
from pathlib import Path
case=json.loads(sys.argv[1]);count=int(sys.argv[2]);at=time.monotonic()
args=['/reader','/source',str(case['entries']),str(case['height']),case['tip'],case['root']]
ps=[subprocess.Popen(args,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True) for _ in range(count)]
peak=0
while any(p.poll() is None for p in ps):
 rss=0
 for p in ps:
  try:rss+=int(Path('/proc/'+str(p.pid)+'/statm').read_text().split()[1])*os.sysconf('SC_PAGE_SIZE')
  except (FileNotFoundError,ProcessLookupError):pass
 peak=max(peak,rss);time.sleep(.01)
outputs=[]
for p in ps:
 out,err=p.communicate();assert p.returncode==0,err
 result=json.loads(out);assert result['root']==case['root'] and result['entries']==case['entries'] and result['height']==case['height'] and result['production_active'] is False
 outputs.append(result)
u=resource.getrusage(resource.RUSAGE_CHILDREN);c=Path('/sys/fs/cgroup')
print(json.dumps(dict(passed=True,entries=case['entries'],processes=count,seconds=time.monotonic()-at,cpu_user_seconds=u.ru_utime,cpu_system_seconds=u.ru_stime,individual_peak_rss_kib=u.ru_maxrss,sampled_combined_peak_rss_bytes=peak,memory_peak_bytes=int((c/'memory.peak').read_text()),cpu_max=(c/'cpu.max').read_text().strip(),memory_max=(c/'memory.max').read_text().strip(),results=outputs)))
'''

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--history', type=Path, required=True, help='completed LOCAL signed-history campaign, not peer evidence')
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--cpuset', default='0,1')
    a = p.parse_args()
    source = a.history.resolve()
    history = json.loads((source / 'report.json').read_text())
    assert history['passed'] and not history['live_engine_generated']
    root = a.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    binary = root / 'reader'
    shutil.copy2(a.binary, binary)
    env = os.environ.copy()
    for key in ('DOCKER_HOST', 'DOCKER_CONTEXT', 'DOCKER_TLS', 'DOCKER_TLS_VERIFY', 'DOCKER_CERT_PATH'):
        env.pop(key, None)
    report = dict(passed=False, independent_exact_target=True, cold_device_cache=False,
                  history_binary_sha256=history['binary_sha256'], binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), cases=[])
    try:
        for case in history['cases']:
            for count in (1, 4):
                command = ['docker', '--host=unix:///var/run/docker.sock', 'run', '--rm',
                           '--network', 'none', '--cpuset-cpus', a.cpuset, '--cpus', '2',
                           '--memory', '2g', '--memory-swap', '2g', '--pids-limit', '128',
                           '--read-only', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges',
                           '--user', f'{os.getuid()}:{os.getgid()}',
                           '--mount', f'type=bind,src={binary},dst=/reader,readonly',
                           '--mount', f'type=bind,src={source / ("state-" + str(case["entries"]))},dst=/source,readonly',
                           IMAGE, 'python', '-c', READERS, json.dumps(case), str(count)]
                result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=300)
                if result.returncode:
                    raise RuntimeError(result.stderr + result.stdout)
                row = json.loads(result.stdout)
                report['cases'].append(row)
                print(json.dumps(row), flush=True)
        report['passed'] = True
    finally:
        (root / 'report.json').write_text(json.dumps(report, indent=2) + '\n')

if __name__ == '__main__':
    main()
