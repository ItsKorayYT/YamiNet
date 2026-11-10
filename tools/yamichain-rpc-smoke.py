#!/usr/bin/env python3
"""Exercise existing encrypted wallets and native PoS finality over authenticated TLS."""
import json,os,pathlib,subprocess,sys,urllib.request,urllib.error,ssl,socket
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
# A wallet gets only public policy/CA and its gateway role credential.
# Hold a bound, non-listening socket so the offline seed cannot be reused.
offline=socket.socket();offline.bind(('127.0.0.1',0))
trusted=work/'trusted-policy.json'
trusted.write_text((config.parent/'data/policy.json').read_text())
seed_config=dict(c)
seed_config['wallet_gateway_seeds']=['https://127.0.0.1:'+str(offline.getsockname()[1]),c['gateway']]
seed_path=work/'trusted-seeds.json';seed_path.write_text(json.dumps(seed_config))
exported=json.loads(run('scripts/yamichain-wallet-config','--network-config',seed_path,'--policy',trusted,'--out',work/'wallet-profile'))
wallet_path=pathlib.Path(exported['config'])
wallet_config=json.loads(wallet_path.read_text())
assert set(wallet_config['rpc']['tokens'])=={'gateway_file'}
assert 'key_file' not in wallet_config['rpc'] and 'witness_tls' not in wallet_config
wallet_env=dict(env,YAMI_DEVNET_CONFIG=str(wallet_path),YAMI_DEVNET_DIR=str(work/'wallet-data'))
def wallet(*args):
 p=subprocess.run([str(root/'scripts/yami-wallet'),'--network','pos',*map(str,args)],env=wallet_env,text=True,capture_output=True)
 if p.returncode:raise RuntimeError(p.stderr)
 return p.stdout.strip()
a=work/'alice.secure';b=work/'bob.secure'
wallet('create',a);bob=wallet('create',b)
run('scripts/yamichain-pos-devnet','faucet',a,1000)
r=json.loads(wallet('send',a,bob,100))
assert json.loads(wallet('balance',a))['balance']==898
assert json.loads(wallet('balance',b))['balance']==100
verified=json.loads(wallet('verify',r['height']));assert verified['verified']
history=json.loads(wallet('transactions',a))['transactions']
assert len(history)==1 and history[0]['status']=='finalized'
assert history[0]['block']==verified['block'] and history[0]['amount']==100
assert json.loads(wallet('retry',a,r['transaction']))['block']==verified['block']
assert json.loads(wallet('balance',b))['balance']==100
portfolio=json.loads(wallet('balance',a))['portfolio']
assert portfolio['rewards']['direct_delegator_rewards'] is False
ctx=ssl.create_default_context(cafile=c['rpc']['ca_file'])
try:urllib.request.urlopen(c['gateway']+'/status',context=ctx)
except urllib.error.HTTPError as e:assert e.code==401
else:raise AssertionError('unauthenticated RPC accepted')
run('scripts/yamichain-pos-devnet','stop')
assert json.loads(wallet('transactions',a))['transactions'][0]['status']=='unknown'
run('scripts/yamichain-pos-devnet','start')
assert json.loads(wallet('balance',b))['balance']==100
assert json.loads(wallet('verify',r['height']))['block']==verified['block']
assert json.loads(wallet('transactions',a))['transactions'][0]['status']=='finalized'
print(json.dumps(dict(software_only=True,production_activation=False,tls_native_finality=True,unauthenticated_rpc_rejected=True,alice_after_payment=898,bob_received_offline=100,restart_preserved=True,encrypted_history_survived=True,pinned_seed_discovery=True,wallet_has_no_backend_credentials=True,idempotent_retry=True,offline_history_available=True,verified_height=r['height'],block=verified['block']),indent=2))
