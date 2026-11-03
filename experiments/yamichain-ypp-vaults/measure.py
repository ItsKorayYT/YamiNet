#!/usr/bin/env python3
"""Run after a locked release build. Measures native code; never cross-ISA claims."""
import datetime
import json
import os
import pathlib
import platform
import resource
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent
BINARY = ROOT / "target/release/yamichain-ypp-vaults"


def read_optional(path):
    try:
        return pathlib.Path(path).read_text().strip()
    except OSError:
        return None


def verifier_memory_limit():
    resource.setrlimit(resource.RLIMIT_AS, (2 * 1024**3, 2 * 1024**3))


def run(*args):
    result = subprocess.run(
        [str(BINARY), *args], check=True, text=True, capture_output=True,
        env={**os.environ, "RAYON_NUM_THREADS": "2"},
        preexec_fn=verifier_memory_limit if args[0] == "verify" else None,
    )
    print(result.stderr, file=sys.stderr, end="")
    return json.loads(result.stdout)


with tempfile.TemporaryDirectory(prefix="ypp-public-benchmark-") as fixture:
    report = {
        "measured_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "architecture": platform.machine(),
        "kernel": platform.release(),
        "cpu_model": next((line.split(":", 1)[1].strip() for line in
                           (read_optional("/proc/cpuinfo") or "").splitlines()
                           if line.startswith("model name")), None),
        "cpu_quota": read_optional("/sys/fs/cgroup/cpu.max"),
        "memory_limit_bytes": read_optional("/sys/fs/cgroup/memory.max"),
        "rayon_threads": 2,
        "verifier_virtual_address_limit_bytes": 2 * 1024**3,
        "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
        "prover_and_recovery": run("demo", "3", fixture),
        "verifier_only": run("verify", fixture),
    }
print(json.dumps(report, indent=2))
