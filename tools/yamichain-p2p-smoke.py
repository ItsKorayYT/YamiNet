#!/usr/bin/env python3
"""Disposable native PoS: one ingress, signed discovery, unconfigured joining process."""
import argparse
import json
import os
import pathlib
import signal
import socket
import subprocess
import sys
import time
ROOT=pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'tools'))
from yamichain_rpc import Transport

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--out',type=pathlib.Path,required=True)
parser.add_argument('--port-offset',type=int,default=5000)
a=parser.parse_args()
out=a.out.resolve();out.parent.mkdir(parents=True,exist_ok=True)
base=json.loads((ROOT/'docs/devnet/pos-network.json').read_text());base['p2p_discovery']=True
source=out.parent/(out.name+'-base.json')
fd=os.open(source,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
with os.fdopen(fd,'w') as stream:json.dump(base,stream)

def run(args,env=None,timeout=300):
    result=subprocess.run(list(map(str,args)),env=env,text=True,capture_output=True,timeout=timeout)
    if result.returncode:raise RuntimeError(result.stderr)
    return result.stdout.strip()

provision=json.loads(run([ROOT/'scripts/yamichain-rpc-config','--base',source,'--out',out,'--port-offset',a.port_offset]))
config=pathlib.Path(provision['config']);data=pathlib.Path(provision['data'])
profile=json.loads(config.read_text())
env=dict(os.environ,YAMI_DEVNET_CONFIG=str(config),YAMI_DEVNET_DIR=str(data))
rpc=Transport(profile['rpc'],{'gateway':[profile['gateway']]})

def cli(*args):return run([ROOT/'scripts/yamichain-pos-devnet',*args],env)
def get(path=''):return rpc.request(profile['gateway']+path,timeout=20)
def until(predicate,seconds=90):
    deadline=time.monotonic()+seconds
    while time.monotonic()<deadline:
        if joining is not None and joining.poll() is not None:raise RuntimeError('joining validator exited; inspect joining-node.log')
        try:
            value=predicate()
            if value:return value
        except (FileNotFoundError,ConnectionError):pass
        time.sleep(.2)
    raise RuntimeError('P2P demonstration deadline; inspect retained process logs')

def records(directory,height):return (directory/f'native-finalized-{height}.bin').exists()
def hint_count(directory):
    raw=(directory/'network-hints.bin').read_bytes()
    assert raw[:8]==b'YCLIVE01'
    assert len(raw)==9+raw[8]*238
    return raw[8]

joining=None;log=None;joining_args=None;joining_env=None
try:
    cli('start')
    assert get()['height']==0
    alice=out/'operator.wallet';bob=out/'recipient.wallet';key=out/'joining.validator'
    owner=cli('wallet',alice);recipient=cli('wallet',bob)
    cli('faucet',alice,1000)
    until(lambda:all(hint_count(data/f'validator-{i}')>=3 for i in range(4)))
    # This identity and endpoint are absent from ALL configured validator slots.
    consensus=cli('validator-key',key);identity=json.loads(key.read_text())
    node_key=out/'joining-key.bin';fd=os.open(node_key,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
    with os.fdopen(fd,'wb') as stream:stream.write(bytes(identity['consensus_secret']+identity['transport_secret']))
    with socket.socket() as tcp, socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as udp:
        tcp.bind(('127.0.0.1',0));port=tcp.getsockname()[1];udp.bind(('127.0.0.1',port))
    endpoint=f'127.0.0.1:{port}'
    assert endpoint not in profile['validator_peers']
    directory=out/'joining-node'
    public_policy=out/'joining-policy.json';public_policy.write_bytes((data/'policy.json').read_bytes())
    joining_env=dict(os.environ,YAMI_NATIVE_POLICY=str(public_policy),YAMI_NATIVE_KEYS=str(node_key),
                     YAMI_NATIVE_DISCOVERY='1',YAMI_NATIVE_DISCOVERY_LISTEN=endpoint,
                     YAMI_NATIVE_DISCOVERY_SEEDS=profile['validator_peers'][0])
    for variable in ('YAMI_NATIVE_VALIDATOR_ENDPOINTS','YAMI_DEVNET_CONFIG','YAMI_DEVNET_DIR','YAMI_WALLET_PASSWORD_FILE'):
        joining_env.pop(variable,None)
    joining_args=[str(ROOT/'target/release/yami-validator'),'run','--dir',str(directory),'--index','6',
                  '--height',str(2**64-1),'--timeout','3600']
    log=open(out/'joining-node.log','a')
    joining=subprocess.Popen(joining_args,env=joining_env,stdout=log,stderr=log)
    initial=get()['height'];until(lambda:records(directory,initial))
    assert consensus not in get('/committee')['members']
    until(lambda:hint_count(directory)>=4)
    until(lambda:bytes.fromhex(consensus) in (data/'validator-0/network-hints.bin').read_bytes())
    cli('register',alice,key);cli('stake',alice,consensus,200)
    joined=None;signed=None
    for _ in range(48):
        committee=get('/committee')
        result=json.loads(cli('faucet',bob,1))
        if consensus in committee['members']:joined=joined or result['height']
        if consensus in get('/history/'+str(result['height']))['finality']['signers']:
            signed=result['height'];break
    assert joined and signed,'unconfigured peer must be selected and sign native finality'
    # Invalid unauthenticated pages consume only bounded receive/verification work.
    with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as attack:
        target=profile['validator_peers'][0].rsplit(':',1)
        for _ in range(100):attack.sendto(b'YCLIVE01'+bytes([255])+b'forged'*32,(target[0],int(target[1])))
    result=json.loads(cli('faucet',bob,1));until(lambda:records(directory,result['height']))
    joining.send_signal(signal.SIGINT);joining.wait(timeout=15);joining=None
    for _ in range(3):result=json.loads(cli('faucet',bob,1))
    joining=subprocess.Popen(joining_args,env=joining_env,stdout=log,stderr=log)
    until(lambda:records(directory,result['height']))
    counts=[hint_count(data/f'validator-{i}') for i in range(4)]+[hint_count(directory)]
    for node in [data/f'validator-{i}' for i in range(4)]+[directory]:
        hints=(node/'network-hints.bin').read_bytes()
        assert bytes.fromhex(owner) not in hints and bytes.fromhex(recipient) not in hints
    for receipt in data.glob('receipt-*.json'):
        raw=json.dumps(json.loads(receipt.read_text())['certificate'])
        assert all(word not in raw for word in ('sender','recipient','amount','balance','portfolio'))
    print(json.dumps(dict(software_only=True,production_activation=False,independent_validator_processes=5,
                         bootstrap_seeds_per_joiner=1,joining_endpoint_not_in_inventory=True,joining_uses_only_public_policy=True,
                         synchronization_before_registration=True,discovered_peers=counts,
                         one_gateway_ingress=True,selected_height=joined,signed_height=signed,
                         restart_catchup=True,invalid_discovery_burst_survived=True,
                         owner_and_recipient_absent_from_discovery=True,private_fields_absent_from_relayed_certificates=True,final_height=result['height']),indent=2))
finally:
    if joining is not None and joining.poll() is None:
        joining.send_signal(signal.SIGINT)
        try:joining.wait(timeout=15)
        except subprocess.TimeoutExpired:joining.kill();joining.wait()
    if log is not None:log.close()
    try:cli('stop')
    except Exception:pass
