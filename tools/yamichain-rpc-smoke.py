#!/usr/bin/env python3
"""Exercise existing encrypted wallets and native PoS finality over authenticated TLS."""
import json,os,pathlib,subprocess,sys,urllib.request,urllib.error,ssl
root=pathlib.Path(__file__).resolve().parents[1]
config=pathlib.Path(sys.argv[1]).resolve();work=pathlib.Path(sys.argv[2]).resolve();work.mkdir(mode=0o700,parents=True,exist_ok=False)
c=json.loads(config.read_text());env=dict(os.environ,YAMI_DEVNET_CONFIG=str(config),YAMI_DEVNET_DIR=str(config.parent/'data'))
pw=work/'password';fd=os.open(pw,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
with os.fdopen(fd,'w') as f:f.write(os.urandom(32).hex())
env['YAMI_WALLET_PASSWORD_FILE']=str(pw)
def run(path,*args):
 p=subprocess.run([str(root/path),*map(str,args)],env=env,text=True,capture_output=True)
 if p.returncode:raise RuntimeError(p.stderr)
 return p.stdout.strip()
def wallet(*args):return run('scripts/yami-wallet','--network','pos',*args)
a=work/'alice.secure';b=work/'bob.secure'
wallet('create',a);bob=wallet('create',b)
run('scripts/yamichain-pos-devnet','faucet',a,1000)
r=json.loads(wallet('send',a,bob,100))
assert json.loads(wallet('balance',a))['balance']==898
assert json.loads(wallet('balance',b))['balance']==100
verified=json.loads(wallet('verify',r['height']));assert verified['verified']
ctx=ssl.create_default_context(cafile=c['rpc']['ca_file'])
try:urllib.request.urlopen(c['gateway']+'/status',context=ctx)
except urllib.error.HTTPError as e:assert e.code==401
else:raise AssertionError('unauthenticated RPC accepted')
run('scripts/yamichain-pos-devnet','stop');run('scripts/yamichain-pos-devnet','start')
assert json.loads(wallet('balance',b))['balance']==100
assert json.loads(wallet('verify',r['height']))['block']==verified['block']
print(json.dumps(dict(software_only=True,production_activation=False,tls_native_finality=True,unauthenticated_rpc_rejected=True,alice_after_payment=898,bob_received_offline=100,restart_preserved=True,verified_height=r['height'],block=verified['block']),indent=2))
