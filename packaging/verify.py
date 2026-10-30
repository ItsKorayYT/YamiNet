#!/usr/bin/env python3
"""Verify staged checksums and real native CLI execution (no network)."""
import hashlib
import pathlib
import subprocess
import sys

stage = pathlib.Path(sys.argv[1]).resolve()
for line in (stage / 'MANIFEST.sha256').read_text(encoding='utf-8').splitlines():
    expected, name = line.split('  ', 1)
    path = (stage / name).resolve()
    if not path.is_relative_to(stage) or hashlib.sha256(path.read_bytes()).hexdigest() != expected:
        raise SystemExit(f'Checksum failed: {name}')
if '--no-execute' not in sys.argv:
    suffix = '.exe' if sys.platform == 'win32' else ''
    for name in ('yami-browser', 'yami-node', 'yami-relay'):
        subprocess.run([str(stage / (name + suffix)), '--version'], cwd=stage, check=True)
    for site in ('alice', 'bob', 'personal', 'blog', 'showcase'):
        subprocess.run([str(stage / ('yami-node' + suffix)), 'site', 'check', str(stage / 'sites' / site)], cwd=stage, check=True)
print('Package manifest passed; executable checks NOT RUN (cross-built).' if '--no-execute' in sys.argv else 'Package manifest and selected native checks passed.')
