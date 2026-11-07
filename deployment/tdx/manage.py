#!/usr/bin/env python3
"""TDX disposable-network provisioning. No deployment or verification success is simulated."""
import argparse,importlib.util,json,os,pathlib,runpy,time,urllib.error
ROOT=pathlib.Path(__file__).resolve().parents[2]
d=runpy.run_path(str(ROOT/'scripts/yamichain-devnet'))
read,save,http,bridge=d['read'],d['save'],d['http'],d['bridge']
spec=importlib.util.spec_from_file_location('collateral',ROOT/'deployment/tdx/collateral.py')
col=importlib.util.module_from_spec(spec);spec.loader.exec_module(col)
admin=col.native
p=argparse.ArgumentParser(description=__doc__)
s=p.add_subparsers(dest='command',required=True)
k=s.add_parser('custodian-key');k.add_argument('path',type=pathlib.Path)
v=s.add_parser('guest-config');v.add_argument('--index',required=True,type=int);v.add_argument('--key',required=True,type=pathlib.Path);v.add_argument('--qgs-port',required=True,type=int);v.add_argument('--out',required=True,type=pathlib.Path)
v=s.add_parser('plan');v.add_argument('--network',required=True,type=pathlib.Path);v.add_argument('--authorities',required=True,type=pathlib.Path);v.add_argument('--data',required=True,type=pathlib.Path)
v=s.add_parser('provision');v.add_argument('--index',type=int,required=True);v.add_argument('--network',type=pathlib.Path,required=True);v.add_argument('--data',type=pathlib.Path,required=True);v.add_argument('--key',type=pathlib.Path,required=True);v.add_argument('--trusted-config',type=pathlib.Path,required=True);v.add_argument('--backup',type=pathlib.Path,required=True,help='new encrypted backup destination, or existing backup for restore')
v=s.add_parser('witness-config');v.add_argument('--index',type=int,required=True);v.add_argument('--key',type=pathlib.Path,required=True);v.add_argument('--policy',type=pathlib.Path,required=True);v.add_argument('--listen',required=True);v.add_argument('--database',required=True);v.add_argument('--out',type=pathlib.Path,required=True)
a=p.parse_args()
print('DISPOSABLE TDX DEVNET ONLY; PRODUCTION ACTIVATION CLOSED')
if a.command=='custodian-key':
    if a.path.exists():raise SystemExit('refusing to overwrite custodian key')
    save(a.path,admin({'op':'key'}));print('Stored private custodian key locally; do not copy into image')
elif a.command=='guest-config':
    key=read(a.key)
    save(a.out,dict(index=a.index,operator=key['operator'],recovery=key['recovery'],listen='0.0.0.0:28790',qgs_port=a.qgs_port))
elif a.command=='plan':
    a.data.mkdir(parents=True,exist_ok=True);os.chmod(a.data,0o700)
    if (a.data/'policy.json').exists():raise SystemExit('policy already exists; use provision/restore without resetting genesis')
    network=read(a.network);identities=[http(url+'/identity') for url in network['executors']]
    if any(id['index']!=i for i,id in enumerate(identities)):raise SystemExit('executor index mismatch')
    if len({tuple(id['operator']) for id in identities})!=4 or len({tuple(id['recovery']) for id in identities})!=4:raise SystemExit('distinct per-executor custodians required')
    authorities=read(a.authorities)
    if len(authorities)!=4 or len({tuple(k) for k in authorities})!=4:raise SystemExit('four distinct independent witness keys required')
    treasury=bridge('key');root=list(os.urandom(32));allocations=[(list(bytes.fromhex(treasury['address'])),1_000_000_000_000)]
    genesis=bridge('genesis',allocations=allocations,root=root)
    policy=dict(committee=dict(chain=24,members=[id['member'] for id in identities]),genesis=genesis,authorities=authorities,observer=list(bytes.fromhex(bridge('key')['address'])))
    save(a.data/'policy.json',policy);save(a.data/'state.json',dict(head=genesis,height=0));save(a.data/'treasury.wallet',treasury)
    save(a.data/'provision.json',dict(root=root,allocations=allocations,restored=None))
    save(a.data/'identities.json',identities)
    print('Provisional public committee pinned. Each custodian MUST verify enrollment before provisioning. No attestation has been accepted yet.')
elif a.command=='provision':
    i=a.index;network=read(a.network);trusted=read(a.trusted_config);key=read(a.key);policy=read(a.data/'policy.json')
    if trusted['witness_policy']!=policy:raise SystemExit('trusted wallet policy mismatch')
    identity=http(network['executors'][i]+'/identity')
    if identity['index']!=i or identity['operator']!=key['operator'] or identity['recovery']!=key['recovery']:raise SystemExit('custodian/guest identity mismatch')
    started=time.monotonic()
    challenge=admin({'op':'challenge'})
    quote=http(network['executors'][i]+'/attest',dict(challenge=challenge,enrollment=True),timeout=20)['quote']
    source=network.get('collateral',['intel-pcs']*4)[i]
    collateral=col.retrieve(quote,trusted) if source=='intel-pcs' else http(source)
    if time.monotonic()-started>50:raise SystemExit('enrollment challenge expired; no provisioning')
    request=admin(dict(op='configure',qvl_library=trusted['qvl_library'],qvl_sha256=trusted['qvl_sha256'],software_policy=trusted['policies'][i],identity=identity,challenge=challenge,evidence=dict(quote=quote,collateral=collateral),operator_secret=key['signing'],recovery_secret=key['recovery_secret'],backup=read(a.backup) if a.backup.exists() else None,policy=policy,authority=network['witnesses'][i],provision=read(a.data/'provision.json')))
    if time.monotonic()-started>60:raise SystemExit('enrollment challenge expired; no provisioning')
    try:
        response=http(network['executors'][i]+'/configure',request)
    except urllib.error.HTTPError as error:
        if error.code!=503:raise
        # Recover a lost configure response. Native QVL above still verifies this boot.
        current=http(network['executors'][i]+'/identity')
        if current['member']!=policy['committee']['members'][i]:raise
        response={'backup':http(network['executors'][i]+'/backup')}
    save(a.backup,response['backup'])
    ack=admin(dict(op='activate',identity=http(network['executors'][i]+'/identity'),backup=response['backup'],operator_secret=key['signing']))
    http(network['executors'][i]+'/activate',ack)
    print('Native QVL verified enrollment; recovery capsule fsynced before activation. Guest uses external witness.')
elif a.command=='witness-config':
    key=read(a.key);policy=read(a.policy)
    if list(bytes.fromhex(key['address']))!=policy['authorities'][a.index]:raise SystemExit('wrong witness key')
    save(a.out,dict(listen=a.listen,database=a.database,policy=policy,index=a.index,signing_secret=key['secret']))
