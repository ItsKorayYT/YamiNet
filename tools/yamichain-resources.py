#!/usr/bin/env python3
"""Linux-only measured, non-financial loopback workload; no target attestation."""
import argparse
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import resource
import subprocess
import tempfile
import time


def read_limit(name):
    path = Path('/sys/fs/cgroup') / name
    return path.read_text().strip() if path.is_file() else None


def sample(pid):
    try:
        fields = dict(line.split(':', 1) for line in Path(f'/proc/{pid}/status').read_text().splitlines() if ':' in line)
        return {key: int(fields.get(key, '0 kB').strip().split()[0]) * 1024 for key in ('VmRSS', 'VmHWM')}
    except (FileNotFoundError, ProcessLookupError):
        return {'VmRSS': 0, 'VmHWM': 0}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/debug/yami-validator'))
    parser.add_argument('--report', type=Path, default=Path('work/v02-resources.json'))
    parser.add_argument('--height', type=int, default=12)
    args = parser.parse_args()
    if platform.system() != 'Linux' or not 1 <= args.height <= 1000:
        parser.error('Linux and height 1..1000 required')
    spec = importlib.util.spec_from_file_location('rehearsal', Path(__file__).with_name('yamichain-local.py'))
    rehearsal = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(rehearsal)
    binary = args.binary.resolve()
    args.report.parent.mkdir(parents=True, exist_ok=True)
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    with tempfile.TemporaryDirectory(prefix='yami-resource-') as temp:
        root = Path(temp)
        logs = [open(root / f'node-{i}.log', 'w', encoding='utf-8') for i in range(4)]
        procs = []
        peak = [0] * 4
        combined = 0
        started = time.monotonic()
        base = rehearsal.free_ports()
        try:
            for i in range(4):
                procs.append(subprocess.Popen([str(binary), 'run', '--dir', str(root / str(i)), '--index', str(i), '--base-port', str(base), '--height', str(args.height), '--timeout', '180', '--linger', '8'], stdout=logs[i], stderr=subprocess.STDOUT))
            while any(p.poll() is None for p in procs):
                if time.monotonic() - started > 200:
                    raise RuntimeError('resource workload deadline')
                snapshots = [sample(p.pid) for p in procs]
                combined = max(combined, sum(s['VmRSS'] for s in snapshots))
                peak = [max(old, s['VmHWM']) for old, s in zip(peak, snapshots)]
                time.sleep(0.05)
            elapsed = time.monotonic() - started
            if any(p.returncode != 0 for p in procs):
                raise RuntimeError({i: (root / f'node-{i}.log').read_text()[-2000:] for i in range(4)})
            after = resource.getrusage(resource.RUSAGE_CHILDREN)
            cpu = (after.ru_utime + after.ru_stime) - (before.ru_utime + before.ru_stime)
            states = [json.loads(subprocess.check_output([str(binary), 'inspect', '--dir', str(root / str(i))], text=True)) for i in range(4)]
            assert all(s['height'] == args.height and not s['payments_enabled'] for s in states)
            assert len({(s['block'], s['state_root']) for s in states}) == 1
            allocated = [sum(p.stat().st_blocks * 512 for p in (root / str(i)).rglob('*') if p.is_file()) for i in range(4)]
            report = {'result': 'PASS', 'generated_at': datetime.now(timezone.utc).isoformat(), 'architecture': platform.machine(),
                      'scope': '4 public-development-key processes, empty blocks after first test-name block; no proofs, payments, full pruning or large-state workload',
                      'profile': 'debug (default unoptimized)', 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                      'cgroup_cpu_max': read_limit('cpu.max'), 'cgroup_memory_max': read_limit('memory.max'),
                      'height': args.height, 'elapsed_seconds_including_8_second_history_linger': round(elapsed, 3),
                      'total_validator_cpu_seconds': round(cpu, 3), 'sample_interval_seconds': 0.05,
                      'sampled_per_validator_peak_rss_bytes': peak, 'sampled_combined_peak_rss_bytes': combined,
                      'per_validator_allocated_store_bytes': allocated, 'targets_verified': False}
            args.report.write_text(json.dumps(report, indent=2) + '\n')
            print(json.dumps(report, indent=2))
        finally:
            for p in procs:
                if p.poll() is None:
                    p.kill()
                p.wait()
            for log in logs:
                log.close()


if __name__ == '__main__':
    main()
