#!/usr/bin/env python3
"""Locked release build first. Linux wait4 measures each child independently."""
import datetime
import json
import os
import pathlib
import platform
import statistics
import subprocess
import sys
import tempfile

from observer import observe

ROOT = pathlib.Path(__file__).resolve().parent
BINARY = ROOT / "target/release/yamichain-ypp-oblivious"


def optional(path):
    try:
        return pathlib.Path(path).read_text().strip()
    except OSError:
        return None


def run(*args):
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        proc = subprocess.Popen([str(BINARY), *args], stdout=stdout, stderr=stderr)
        _, status, usage = os.wait4(proc.pid, 0)
        proc.returncode = os.waitstatus_to_exitcode(status)
        stderr.seek(0)
        error = stderr.read().decode()
        if proc.returncode:
            raise RuntimeError(f"{args}: {error}")
        stdout.seek(0)
        result = json.load(stdout)
    result["whole_harness_process_peak_rss_kib"] = usage.ru_maxrss
    return result


def main():
    rows = []
    for n in [2, 4, 8, 16, 32, 64, 128, 256]:
        print(f"measuring {n} vaults", file=sys.stderr, flush=True)
        row = run("measure", str(n), "3")
        row["median_us"] = {key: statistics.median(sample[key] for sample in row["samples"])
                            for key in row["samples"][0] if key.endswith("_us")}
        rows.append(row)
    with tempfile.TemporaryDirectory(prefix="ypp4-public-") as directory:
        demo = run("demo", directory)
        verifier = run("verify", directory)
        observer = observe(pathlib.Path(directory) / "downloadable-snapshots.json")
        assert observer["positions_from_downloadable_ciphertexts"] == [[i] for i in demo["observer_labels_separate_from_inputs"]]
        assert observer["positions_from_downloadable_ciphertexts"] == verifier["downloadable_snapshot_positions"]
    print(json.dumps({
        "measured_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "architecture": platform.machine(), "kernel": platform.release(),
        "cpu_model": next((line.split(":", 1)[1].strip() for line in
                           (optional("/proc/cpuinfo") or "").splitlines()
                           if line.startswith("model name")), None),
        "cpu_quota": optional("/sys/fs/cgroup/cpu.max"),
        "memory_limit_bytes": optional("/sys/fs/cgroup/memory.max"),
        "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
        "timing_scope": "single-process native release CPU, three samples; not distributed execution",
        "byte_scope": "canonical fixed-width payload formulas plus separately measured JSON packet bytes; excludes transport/framing/consensus replication",
        "rss_scope": "whole child harness including wallets, both servers, proof and JSON; not client-only memory",
        "sizes": rows, "fresh_demo": demo, "fresh_verifier": verifier,
        "independent_python_observer": observer,
    }, indent=2))


if __name__ == "__main__":
    main()
