#!/usr/bin/env python3
"""Ensure ordinary YamiNet applications have no dependency on the optional chain."""
import json
import subprocess

metadata = json.loads(subprocess.check_output(
    ['cargo', 'metadata', '--no-deps', '--format-version', '1', '--locked'], text=True))
packages = {p['name']: p for p in metadata['packages']}
protected = ['yami-browser', 'yami-node', 'yami-relay-cli', 'yami-testnet',
             'yami-yns', 'yami-identity', 'yami-dht', 'yami-games']
for root in protected:
    pending = [root]
    seen = set()
    while pending:
        name = pending.pop()
        if name in seen:
            continue
        seen.add(name)
        if name.startswith('yami-chain') or name == 'yami-validator':
            raise SystemExit(f'FAIL: {root} depends on {name}')
        if name in packages:
            pending += [d['name'] for d in packages[name]['dependencies'] if d['kind'] != 'dev']
print('PASS: browser, hosted node, relay, testnet, YNS, identity, DHT and games have no chain dependency')
