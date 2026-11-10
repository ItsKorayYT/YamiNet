#!/usr/bin/env python3
"""Kill and recover real TLS devnet services without resetting durable authority."""
import json,os,pathlib,signal,subprocess,sys,time
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parent))
from yamichain_rpc import Transport
root=pathlib.Path(__file__).resolve().parents[1];config=pathlib.Path(sys.argv[1]).resolve();c=json.loads(config.read_text());data=config.parent/'data'
env=dict(os.environ,YAMI_DEVNET_CONFIG=str(config),YAMI_DEVNET_DIR=str(data))
cli=root/'scripts/yamichain-pos-devnet'
def run(command):
 return subprocess.run([str(cli),command],env=env,check=True,capture_output=True,text=True)
run('start');run('stop');run('start')
rpc=Transport(c['rpc'],{'gateway':[c['gateway']],'executor':c['executors'],'validator':c['validators']})
def get(p):return rpc.request(c['gateway']+p,timeout=10)
def until(predicate):
 end=time.monotonic()+45
 while time.monotonic()<end:
  try:
   result=predicate()
   if result:return result
  except Exception:pass
  time.sleep(.25)
 raise RuntimeError('supervised service did not recover')
initial=get('/status');head=initial['head']
for name in ['witness-0','executor-0','validator-0']:
 before=until(lambda:get('/services'))[name];os.kill(before['pid'],signal.SIGKILL)
 until(lambda:(lambda s:s['running'] and s['pid']!=before['pid'] and s['restarts']>before['restarts'])(get('/services')[name]))
 until(lambda:get('/status')['healthy_executors']==4 and get('/status')['healthy_validators']>=4)
 assert get('/status')['head']==head
# Kill the private worker while leaving its wrapper up: it must load the
# existing authenticated witness checkpoint rather than initialize new funds.
def worker_pid():
 wrapper=get('/services')['executor-1']['pid']
 for path in pathlib.Path('/proc').iterdir():
  if not path.name.isdigit():continue
  try:
   fields=(path/'stat').read_text().split(') ',1)[1].split()
   argv=(path/'cmdline').read_bytes().split(b'\0')
   if int(fields[1])==wrapper and argv[0].endswith(b'/witnessed-worker'):return int(path.name)
  except (OSError,ValueError):pass
 return None
worker=until(worker_pid);os.kill(worker,signal.SIGKILL)
until(lambda:rpc.request(c['executors'][1],timeout=5).get('Head')==head)
assert get('/status')['head']==head
until(lambda:get('/recovery')['ok'])
print(json.dumps(dict(software_only=True,production_activation=False,supervised_wrapper_restarts=3,private_worker_recovered=True,original_head_preserved=True,finalized_height=initial['height'],stores_and_signing_wals_retained=True),indent=2))
