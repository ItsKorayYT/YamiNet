#!/usr/bin/env python3
"""Reproduce the official Intel QVL build and run mandatory native tests, never fake QVL."""
import argparse, hashlib, json, os, pathlib, subprocess
PIN = 'd12717e3f1f2ab81313f88001fd902b5ef5f9c8c'
URL = 'https://github.com/intel/SGX-TDX-DCAP-QuoteVerificationLibrary.git'
p = argparse.ArgumentParser()
p.add_argument('--work', required=True, type=pathlib.Path)
p.add_argument('--cmake', default='cmake')
p.add_argument('--cargo', default='cargo')
p.add_argument('--jobs', default='3')
a = p.parse_args()
work = a.work.resolve(); work.mkdir(parents=True, exist_ok=True)
source = work/'intel-qvl'; build = work/'native-build'
def run(args, **kw): subprocess.run([str(v) for v in args], check=True, **kw)
if not source.exists():
    run(['git', 'init', source])
    run(['git', '-C', source, 'remote', 'add', 'origin', URL])
    run(['git', '-C', source, 'fetch', '--depth', '1', 'origin', PIN])
    run(['git', '-C', source, 'checkout', '--detach', 'FETCH_HEAD'])
actual = subprocess.check_output(['git', '-C', str(source), 'rev-parse', 'HEAD'], text=True).strip()
if actual != PIN:
    raise RuntimeError('Intel QVL checkout is not the pinned commit')
run(['git', '-C', source, 'diff', '--exit-code'])
run(['git', '-C', source, 'diff', '--cached', '--exit-code'])
# Include ignored files too: source globs must not compile injected ignored .cpp files.
extra = set(subprocess.check_output(['git', '-C', str(source), 'ls-files', '--others'], text=True).splitlines())
allowed = {'Src/AttestationLibrary/src/Version/Version.h', 'Src/AttestationParsers/src/Version/Version.h'}
if extra - allowed:
    raise RuntimeError('Unreviewed extra files in Intel source checkout')
native = pathlib.Path(__file__).resolve().parent
run([a.cmake, '-S', native, '-B', build, f'-DINTEL_QVL_SOURCE={source}', '-DCMAKE_BUILD_TYPE=Release'])
run([a.cmake, '--build', build, '-j', a.jobs])
library = build/'libyami_intel_qvl.so'
digest = hashlib.sha256(library.read_bytes()).hexdigest()
metadata = {'intel_source': URL, 'intel_commit': PIN, 'library': str(library), 'sha256': digest,
            'hardware_activation': False, 'mode': 'native wallet-local QVL; archived signed fixtures'}
(work/'native-build.json').write_text(json.dumps(metadata, indent=2)+'\n')
print(json.dumps(metadata), flush=True)
env = os.environ.copy(); env.update(YAMI_QVL_LIBRARY=str(library), YAMI_QVL_SHA256=digest)
run([a.cargo, 'test', '--locked', '--manifest-path', native.parent/'Cargo.toml', '--test', 'attestation_native', '--', '--ignored', '--nocapture'], env=env)

run([a.cargo, "test", "--locked", "--manifest-path", native.parent/"Cargo.toml", "--lib", "native_wallet_rounds", "--", "--ignored", "--nocapture"], env=env)
