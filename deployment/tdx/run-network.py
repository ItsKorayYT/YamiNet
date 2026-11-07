#!/usr/bin/env python3
"""Run continuous native validators + gateway against already provisioned remote TDX guests.
YAMI_DEVNET_DIR and YAMI_DEVNET_CONFIG are mandatory. This never starts software executors.
"""
import os,pathlib,signal,subprocess,sys,time
ROOT=pathlib.Path(__file__).resolve().parents[2]
for k in ('YAMI_DEVNET_DIR','YAMI_DEVNET_CONFIG'):
    if k not in os.environ:raise SystemExit(k+' required; preserve the existing software devnet')
children=[]
def cleanup(*_):
    for p in children:p.terminate()
    for p in children:
        try:p.wait(timeout=5)
        except subprocess.TimeoutExpired:p.kill()
signal.signal(signal.SIGTERM,lambda *_:(cleanup(),sys.exit(0)));signal.signal(signal.SIGINT,lambda *_:(cleanup(),sys.exit(0)))
try:
    for i in range(4):children.append(subprocess.Popen([str(ROOT/'scripts/yamichain-devnet'),'validator',str(i)]))
    time.sleep(2)
    if any(p.poll() is not None for p in children):raise RuntimeError('validator startup failed')
    children.append(subprocess.Popen([str(ROOT/'scripts/yamichain-devnet'),'gateway']))
    while all(p.poll() is None for p in children):time.sleep(1)
    raise RuntimeError('a native network service stopped')
finally:cleanup()
