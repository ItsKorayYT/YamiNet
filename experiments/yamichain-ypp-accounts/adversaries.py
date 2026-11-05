#!/usr/bin/env python3
"""Public-set leakage experiment; does not simulate cryptographic verification."""
import json, random
from pathlib import Path
rng = random.Random(293)
results=[]
for n in (16,64,128):
    population=list(range(4096))
    # Repeat the same sender/recipient, independently sampling everyone else.
    intersection=set(population)
    trace=[]
    for _ in range(8):
        selected={0,1,*rng.sample(population[2:],n-2)}
        intersection &= selected
        trace.append(len(intersection))
    fixed=set(range(n))
    assert len(set.intersection(*(fixed.copy() for _ in range(8))))==n
    # Every observer-controlled account can decrypt its zero change and exclude itself.
    for controlled in (0,n//2,n-2):
        honest=fixed-set(range(2,2+controlled))
        assert {0,1} <= honest
        results.append(dict(accounts=n,controlled=controlled,effective_candidates=len(honest),
            intersection_independent_sets=trace,fixed_cohort_intersection=n,
            sybil_registrations_with_valid_possession="accepted; possession does not imply independence"))
assert all(r['intersection_independent_sets'][-1]==2 for r in results)
Path('adversaries.json').write_text(json.dumps(results,indent=2)+'\n')
print(json.dumps(results))
