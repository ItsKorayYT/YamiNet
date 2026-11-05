#!/usr/bin/env python3
"""Reproducible PUBLIC-METADATA attacks, not a proof/payment simulator.
Cryptographic state/proof tests live separately in tests/cross_cohort.rs.
Python PRNG gives uniform test samples; it is not protocol randomness.
"""
import json, random, math, time
from itertools import combinations
from fractions import Fraction
from pathlib import Path
N, WIDTH, Q = 500_000, 64, 4
GROUPS = math.ceil(N/WIDTH)
rng = random.Random(18)
def choose(s, t):
    real={s//WIDTH,t//WIDTH}
    out=set(real)
    while len(out)<Q: out.add(rng.randrange(GROUPS))
    return out
out={"population":N,"cohort_width":WIDTH,"selected_cohorts":Q,
     "claim":"candidate rejected; metadata attacks only"}
for profile in ("repeated_pair", "same_sender_rotating_recipient", "same_recipient_rotating_sender"):
    intersection=set(range(GROUPS));trace=[]
    for epoch in range(1,9):
        s=17 if profile!="same_recipient_rotating_sender" else epoch*WIDTH+17
        t=300_017 if profile!="same_sender_rotating_recipient" else (100+epoch)*WIDTH+17
        intersection &= choose(s,t)
        trace.append(len(intersection))
    assert trace[-1]==(2 if profile=="repeated_pair" else 1)
    out[profile]={"epochs":8,"surviving_cohorts":trace,"linkage_assumption":"adversary is given same-pair/same-sender/same-recipient linkage, or chooses that challenge history"}
# Chosen-history game: equal public parameters, balance feasibility and amount.
# World 0 uses (17, 81), world 1 uses (145, 209), four distinct cohorts.
trials=20_000;wins=0
for _ in range(trials):
    bit=rng.randrange(2)
    support=choose(17,81) if bit==0 else choose(145,209)
    guess=0 if 0 in support else 1
    wins+=guess==bit
expected=1-(Q-2)/(GROUPS-2)/2
assert wins/trials>0.99
out['selective_censorship'] = {
    'policy': 'malicious proposer includes iff cohort 0 appears in public support',
    'world0_inclusion_probability': 1.0,
    'world1_inclusion_probability': (Q-2)/(GROUPS-2),
    'status': 'analytic consequence of the same measured support classifier; no consensus liveness simulation'
}
out["chosen_pair_game"]={"trials":trials,"wins":wins,"success_rate":wins/trials,"uniform_cover_expected_success":expected,"advantage_over_half":wins/trials-0.5,"uses_ciphertext_or_proof_break":False}
# Same attack still applies with ideal, unknown-in-advance partition randomness.
# Materialize a public random permutation each epoch only for this stress test.
intersection=set(range(N));trace=[];start=time.perf_counter()
for epoch in range(4):
    perm=list(range(N));rng.shuffle(perm)
    a=perm.index(17)//WIDTH;b=perm.index(300_017)//WIDTH
    groups={a,b}
    while len(groups)<Q:groups.add(rng.randrange(GROUPS))
    support={v for g in groups for v in perm[g*WIDTH:min((g+1)*WIDTH,N)]}
    intersection &= support;trace.append(len(intersection))
assert intersection=={17,300_017}
out["ideal_public_partition_refresh"]={"surviving_accounts":trace,"materialized_permutations":4,"elapsed_s":time.perf_counter()-start,"same_pair_linkage_assumed":True,"purpose":"unpredictability alone does not repair public support"}
# Freeze cover by reusing entropy: removes per-epoch intersection shrinkage,
# but leaves only those four public groups and fails the global challenge game.
frozen=choose(17,300_017)
assert len(set.intersection(*(frozen.copy() for _ in range(8))))==Q
out["reused_seed_control"]={"surviving_cohorts":Q,"global_anonymity_restored":False}
# Registration is permissionless: possession does not bound corruption fraction.
# Account owners who know x_i decrypt C_i - x_i D and remove their zero updates.
out["colluding_accounts"]=[{"support_accounts":256,"controlled_accounts":c,"remaining_candidates":256-c} for c in (0,128,254)]
# A client controlling its randomness can grind a cover-group placement target.
attempts=0
while True:
    attempts+=1
    support=choose(17,300_017)
    if 42 in support:break
assert attempts<1_000_000
out["sender_seed_grinding"]={"attempts_to_force_specific_cover_cohort":attempts,"target_cohort":42,"selection_fairness_proved_by_base_proof":False}
# Analytic, not measured cryptography. k is maximum publicly written slots.
out["support_bound"]=[{"N":N,"support_limit":k,"uniform_pair_game_success":1-(k-2)/(N-2)/2,"interpretation":"account-uniform cover comparator, not the grouped candidate"} for k in (16,64,128,256)]
# Exact small-universe check of the perfect-indistinguishability support lemma.
# A common transcript-support distribution must lie in the intersection of
# support families valid for EVERY candidate pair. Only the dense support does.
exact=[]
for size in range(2,7):
    universe=set(range(6))
    common=set(combinations(range(6),size))
    for a,b in combinations(range(6),2):
        common &= {support for support in combinations(range(6),size) if {a,b}<=set(support)}
    assert bool(common)==(size==6)
    exact.append(dict(N=6,support_size=size,common_valid_supports=len(common)))
# Sender 0 vs sender 2 with a common recipient 1, uniform 4-of-8 support.
def exact_supports(sender):
    return [{sender,1,*cover} for cover in combinations([i for i in range(8) if i not in (sender,1)],2)]
a,b=exact_supports(0),exact_supports(2)
prob0=Fraction(sum(2 in v for v in a),len(a));prob1=Fraction(sum(2 in v for v in b),len(b))
success=(1-prob0+prob1)/2
assert success==Fraction(5,6)
out['exact_support_checks']={'families':exact,'uniform_4_of_8_game_success':str(success)}
Path('attacks.json').write_text(json.dumps(out,indent=2)+'\n')
print(json.dumps(out))
