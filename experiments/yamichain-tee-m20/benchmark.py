"""Run after cargo build --release --locked; Linux wait4 reports per-child RSS."""
import json, os, pathlib, platform, subprocess, tempfile
root = pathlib.Path(__file__).resolve().parent
rows = []
for count in (2, 1000, 10000):
    with tempfile.TemporaryDirectory() as directory, tempfile.TemporaryFile() as output:
        child = subprocess.Popen([str(root/'target/release/measure'), directory, str(count)], stdout=output)
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
        if child.returncode:
            raise RuntimeError(child.returncode)
        output.seek(0)
        row = json.load(output)
        row['peak_rss_kib'] = usage.ru_maxrss
        rows.append(row)
print(json.dumps({'platform': platform.platform(), 'runs': rows, 'repetitions': 1,
    'warning': 'SOFTWARE ONLY; full snapshot per transfer; no TDX or production capacity claim'}, indent=2))
