#!/usr/bin/env python3
"""Actual ephemeral 2048-bit Paillier arithmetic, NOT a transfer proof."""
import json, math, secrets, time, statistics, resource
from pathlib import Path
from cryptography.hazmat.primitives.asymmetric import rsa
start=time.perf_counter()
k=rsa.generate_private_key(public_exponent=65537,key_size=2048).private_numbers()
p,q=k.p,k.q
n=p*q; nn=n*n; lam=math.lcm(p-1,q-1); mu=pow(lam,-1,n)
def enc(m):
    while True:
        r=secrets.randbelow(n)
        if r and math.gcd(r,n)==1: break
    return ((1+n*m)*pow(r,n,nn))%nn
def dec(c): return (((pow(c,lam,nn)-1)//n)*mu)%n
setup_ms=(time.perf_counter()-start)*1000
et=[]; dt=[]
for _ in range(5):
    start=time.perf_counter(); c=enc(2**64-1);et.append((time.perf_counter()-start)*1000)
    start=time.perf_counter(); assert dec(c)==2**64-1;dt.append((time.perf_counter()-start)*1000)
assert dec(enc(123)*enc(456)%nn)==579
out=dict(modulus_bits=n.bit_length(),ciphertext_bytes=(nn.bit_length()+7)//8,
    rsa_prime_setup_ms=setup_ms,encrypt_ms_median=statistics.median(et),decrypt_ms_median=statistics.median(dt),
    samples=5,full_u64_roundtrip=True,homomorphic_credit=True,proof_implemented=False,
    proof_bytes=None,proof_generation_ms=None,verifier_ms=None,
    reason='Distinct per-account moduli cannot share the ElGamal common-randomness statement. Need proofs of well-formed keys, plaintext knowledge, cross-modulus equality/conservation, hidden selectors, ranges and authorization. No justified implementation of that composed proof here.',
    peak_rss_kib=resource.getrusage(resource.RUSAGE_SELF).ru_maxrss)
Path('paillier.json').write_text(json.dumps(out,indent=2)+'\n');print(json.dumps(out))
