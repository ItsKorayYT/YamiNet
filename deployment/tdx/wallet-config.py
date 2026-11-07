#!/usr/bin/env python3
"""Assemble local trusted policy from independently approved software/boot policies.
Never derive approved measurements from an unverified quote.
"""
import argparse,hashlib,json,os,pathlib
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--qvl-library',required=True,type=pathlib.Path)
p.add_argument('--qvl-sha256',required=True)
p.add_argument('--witness-policy',required=True,type=pathlib.Path)
p.add_argument('--software-policy',required=True,type=pathlib.Path,nargs=4)
p.add_argument('--out',required=True,type=pathlib.Path)
a=p.parse_args()
if hashlib.sha256(a.qvl_library.read_bytes()).hexdigest()!=a.qvl_sha256:raise SystemExit('trusted QVL digest mismatch')
policy=json.loads(a.witness_policy.read_text());software=[json.loads(f.read_text()) for f in a.software_policy]
for s in software:
    if len(s['identity'])!=32 or len(s['mrtd'])!=48 or len(s['rtmrs'])!=4 or any(len(r)!=48 for r in s['rtmrs']):
        raise SystemExit('independently approved MRTD/RTMR policy required; file hashes are not TDX measurements')
    if any(len(s[f])!=48 for f in ('config_id','owner','owner_config')) or s['td_attributes']&1:
        raise SystemExit('complete non-debug software policy required')
v=dict(qvl_library=str(a.qvl_library.resolve()),qvl_sha256=list(bytes.fromhex(a.qvl_sha256)),committee=policy['committee'],genesis=policy['genesis'],policies=software,witness_policy=policy)
fd=os.open(a.out,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
with os.fdopen(fd,'w') as f:json.dump(v,f,indent=2);f.write('\n');f.flush();os.fsync(f.fileno())
print(a.out)
