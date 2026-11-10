#!/usr/bin/env python3
"""One disposable TLS network: live PoS lifecycle plus existing encrypted wallets."""
import argparse,json,os,pathlib,subprocess
root=pathlib.Path(__file__).resolve().parents[1]
p=argparse.ArgumentParser(description=__doc__);p.add_argument('--out',type=pathlib.Path,required=True);p.add_argument('--port-offset',type=int,default=2000);a=p.parse_args()
def run(args,env=None):
 value=subprocess.run(list(map(str,args)),env=env,capture_output=True,text=True)
 if value.returncode:raise RuntimeError(value.stderr)
 return json.loads(value.stdout)
provision=run([root/'scripts/yamichain-rpc-config','--out',a.out,'--port-offset',a.port_offset])
config=pathlib.Path(provision['config']);directory=config.parent
env=dict(os.environ,YAMI_DEVNET_CONFIG=str(config),YAMI_DEVNET_DIR=provision['data'],YAMI_POS_SMOKE_WALLET_DIR=str(directory/'pos-wallets'))
pos=run([root/'scripts/yamichain-pos-smoke'],env)
wallets=run(['python3',root/'tools/yamichain-rpc-smoke.py',config,directory/'encrypted-wallets'],env)
print(json.dumps(dict(software_only=True,production_activation=False,transport='authenticated TLS RPC; configured bootstrap P2P peers',gateway=provision['gateway'],pos=pos,encrypted_payments=wallets),indent=2))
