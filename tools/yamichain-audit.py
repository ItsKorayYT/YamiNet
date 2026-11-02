#!/usr/bin/env python3
"""Record an official cargo-audit result and fail on findings; never suppress it."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--audit', default='cargo-audit')
p.add_argument('--db', type=Path, required=True)
p.add_argument('--report', type=Path, required=True)
a = p.parse_args()
command = [a.audit, 'audit', '--db', str(a.db), '--no-fetch', '--json']
scan = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
result = json.loads(scan.stdout)  # Tool/setup failures cannot be reported as a clean audit.
result['yamichain_provenance'] = {
    'tool': subprocess.check_output([a.audit, '--version'], text=True).strip(),
    'database_commit': subprocess.check_output(['git', '-C', str(a.db), 'rev-parse', 'HEAD'], text=True).strip(),
    'lock_sha256': hashlib.sha256(Path('Cargo.lock').read_bytes()).hexdigest(),
    'command': command, 'exit_code': scan.returncode, 'suppressed_advisories': []}
a.report.parent.mkdir(parents=True, exist_ok=True)
a.report.write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps({'vulnerabilities': result['vulnerabilities']['count'],
                  'warnings': {k: len(v) for k, v in result.get('warnings', {}).items()},
                  'exit_code': scan.returncode}))
raise SystemExit(scan.returncode or int(result['vulnerabilities']['count'] > 0))
