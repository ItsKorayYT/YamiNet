#!/usr/bin/env python3
"""Independent public genesis/state vectors (requires Python cryptography)."""
from hashlib import sha256
from struct import pack
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

def h(d,b):
    return sha256(pack('>Q',len(d))+d+pack('>Q',len(b))+b).digest()
def key(role,i):
    return Ed25519PrivateKey.from_private_bytes(h(b'yamichain-public-development-key-v1',role.encode()+bytes([i]))).public_key().public_bytes(Encoding.Raw,PublicFormat.Raw)
def field(b): return pack('>I',len(b))+b
members=sorted((key('consensus',i),key('transport',i),1) for i in range(4))
genesis=(b'YC-GENESIS-2'+field(b'yamichain-local-0.2')+pack('>HQIQI',2,0,1,1,4)+b''.join(c+t+pack('>Q',p) for c,t,p in members))
gid=h(b'yamichain-genesis-v2',genesis)
def root(height):
    suffix=pack('>II',0,0) if height==0 else pack('>I',1)+key('registry',0)+pack('>QI',1,1)+field(b'test-local')+key('registry',0)
    return h(b'yamichain-state-v1',gid+pack('>Q',height)+suffix).hex()
print('genesis',gid.hex())
print('state-0',root(0))
print('state-20',root(20))
