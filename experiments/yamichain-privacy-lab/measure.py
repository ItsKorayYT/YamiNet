#!/usr/bin/env python3
"""Linux offline-process measurements; build the release example before running.

Print five fresh-process samples per input count. RSS covers the complete process
(including a recovered onward spend), not just verification. No hardware claims.
"""
import json
import os
from pathlib import Path
import resource
import subprocess
import tempfile
import time


def main():
    binary = Path(__file__).parent / "target/release/examples/offline"
    samples = []
    for inputs in (1, 2):
        for _ in range(5):
            with tempfile.TemporaryFile() as output:
                start = time.perf_counter()
                pid = os.fork()
                if pid == 0:
                    # Bound the whole child process to 2 GiB of virtual address space.
                    resource.setrlimit(resource.RLIMIT_AS, (2 * 1024**3,) * 2)
                    os.dup2(output.fileno(), 1)
                    os.execv(str(binary), [str(binary), str(inputs)])
                _, status, usage = os.wait4(pid, 0)
                if status != 0:
                    raise RuntimeError(f"offline example failed: wait status {status}")
                wall = time.perf_counter() - start
                output.seek(0)
                fields = dict(x.split("=", 1) for x in output.read().decode().split())
            samples.append({
                **{k: int(v) if v.isdigit() else v for k, v in fields.items()},
                "process_wall_seconds": wall,
                "process_cpu_seconds": usage.ru_utime + usage.ru_stime,
                "process_peak_rss_kib": usage.ru_maxrss,
            })
    print(json.dumps({
        "architecture": os.uname().machine,
        "cpu": subprocess.check_output(["lscpu"], text=True).split("Model name:")[1].splitlines()[0].strip(),
        "cpu_quota": Path("/sys/fs/cgroup/cpu.max").read_text().strip(),
        "environment_memory_max": Path("/sys/fs/cgroup/memory.max").read_text().strip(),
        "child_address_space_limit_bytes": 2 * 1024**3,
        "scope": "offline experiment; RSS and CPU cover complete process, not verifier alone",
        "samples": samples,
    }, indent=2))


if __name__ == "__main__":
    main()
