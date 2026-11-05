#!/usr/bin/env python3
"""Audit a stopped LOCAL live campaign through inactive V2 without changing safety data."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import runpy
import shutil
import subprocess

IMAGE = runpy.run_path(str(Path(__file__).with_name('yamichain-v2-integration.py')))['IMAGE']
AUDIT = '''import json,resource,subprocess,sys,time
from pathlib import Path
state=json.loads(sys.argv[1]);at=time.monotonic()
r=subprocess.run(['/bench','live-audit','/audit/store','/config',str(state['height']),state['block'],state['state_root']],capture_output=True,text=True,timeout=900)
assert r.returncode==0,r.stderr
result=json.loads(r.stdout);assert result['live_engine_history_verified'] and result['inactive_v2']['entries']==state['test_tlds'] and result['inactive_v2']['production_active'] is False
u=resource.getrusage(resource.RUSAGE_CHILDREN);c=Path('/sys/fs/cgroup')
result.update(cpu_user_seconds=u.ru_utime,cpu_system_seconds=u.ru_stime,peak_rss_kib=u.ru_maxrss,seconds=time.monotonic()-at,memory_peak_bytes=int((c/'memory.peak').read_text()),cpu_max=(c/'cpu.max').read_text().strip(),memory_max=(c/'memory.max').read_text().strip())
print(json.dumps(result))
'''


def hashes(path):
    return {str(p.relative_to(path)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(path.rglob('*')) if p.is_file()}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--source', type=Path, required=True, help='completed stopped local live campaign')
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--cpuset', default='0,1')
    a = p.parse_args()
    source = a.source.resolve()
    live = json.loads((source / 'report.json').read_text())
    assert live['passed'] and live['live_engine_generated'] and live['signing_prefix_preserved']
    states = live['verified_states']
    assert len({(s['height'], s['block'], s['state_root']) for s in states}) == 1
    root = a.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    binary = root / 'bench'
    shutil.copy2(a.binary, binary)
    before = hashes(source / 'store-0')
    report = dict(passed=False, binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                  independently_retained_live_target=states[0], source_files=len(before))
    env = os.environ.copy()
    for key in ('DOCKER_HOST', 'DOCKER_CONTEXT', 'DOCKER_TLS', 'DOCKER_TLS_VERIFY', 'DOCKER_CERT_PATH'):
        env.pop(key, None)
    try:
        # Only node 0's own original identity plus public roster/bootstrap files
        # are visible. Application outputs live in disposable tmpfs siblings;
        # no signing, WAL, archive or identity file is copied into a checkpoint.
        command = ['docker', '--host=unix:///var/run/docker.sock', 'run', '--rm',
                   '--network', 'none', '--cpuset-cpus', a.cpuset, '--cpus', '2', '--memory', '2g',
                   '--memory-swap', '2g', '--read-only', '--pids-limit', '128', '--cap-drop', 'ALL',
                   '--security-opt', 'no-new-privileges', '--user', f'{os.getuid()}:{os.getgid()}',
                   '--tmpfs', f'/audit:size=512m,nr_inodes=8192,uid={os.getuid()},gid={os.getgid()}',
                   '--mount', f'type=bind,src={binary},dst=/bench,readonly',
                   '--mount', f'type=bind,src={source / "store-0"},dst=/audit/store',
                   '--mount', f'type=bind,src={source / "config-0"},dst=/config,readonly',
                   IMAGE, 'python', '-c', AUDIT, json.dumps(states[0])]
        result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=960)
        assert result.returncode == 0, result.stderr + result.stdout
        report['measurement'] = json.loads(result.stdout)
        assert hashes(source / 'store-0') == before
        report['original_archive_wal_signing_bytes_unchanged'] = True
        report['source_tree_sha256'] = hashlib.sha256(json.dumps(before, sort_keys=True).encode()).hexdigest()
        report['passed'] = True
    finally:
        (root / 'report.json').write_text(json.dumps(report, indent=2) + '\n')

if __name__ == '__main__':
    main()
