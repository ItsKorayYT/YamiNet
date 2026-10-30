#!/usr/bin/env python3
"""Check complete bundles, then run native binaries with no Cargo on PATH."""
import argparse
import hashlib
import os
import pathlib
import re
import shutil
import subprocess
import tempfile

BINS = ('yami-browser', 'yami-node', 'yami-relay')
FONTS = ('OpenSans-Regular.ttf', 'OpenSans-Semibold.ttf', 'DejaVuSans-Oblique.ttf', 'DejaVuSansMono.ttf', 'LICENSE.txt', 'Apache-2.0.txt', 'DejaVu-LICENSE.txt')


def runtime_env():
    env = dict(os.environ)
    env['PATH'] = os.pathsep.join([str(pathlib.Path(os.environ['SystemRoot']) / 'System32'), os.environ['SystemRoot']]) if os.name == 'nt' else os.defpath
    if shutil.which('cargo', path=env['PATH']) or shutil.which('rustc', path=env['PATH']):
        raise ValueError('runtime verification PATH unexpectedly contains Rust tools')
    return env


def verify(stage, execute=True, expected_version=None, installed=False):
    stage = pathlib.Path(stage).resolve()
    manifest = stage / 'MANIFEST.sha256'
    if manifest.is_symlink() or not manifest.is_file() or manifest.stat().st_size > 1024 * 1024:
        raise ValueError('missing or oversized regular manifest')
    listed = set()
    for line in manifest.read_text(encoding='utf-8').splitlines():
        digest, name = line.split('  ', 1)
        rel = pathlib.PurePosixPath(name)
        if not re.fullmatch('[0-9a-f]{64}', digest) or rel.is_absolute() or '..' in rel.parts or '\\' in name or name in listed:
            raise ValueError('unsafe or duplicate manifest entry')
        path = stage / rel
        if any(parent.is_symlink() for parent in (path, *path.parents) if parent != stage.parent):
            raise ValueError('symlinks are not bundle resources')
        if not path.resolve().is_relative_to(stage) or not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            raise ValueError(f'checksum failed: {name}')
        listed.add(name)
    actual = set()
    for path in stage.rglob('*'):
        if path.is_symlink():
            raise ValueError('symlink in bundle')
        if path.is_file() and path != manifest and not (installed and path.name == 'Uninstall.exe' and path.parent == stage):
            actual.add(path.relative_to(stage).as_posix())
    if listed != actual:
        raise ValueError('manifest does not exactly cover the bundle')
    windows = (stage / 'yami-browser.exe').is_file()
    suffix = '.exe' if windows else ''
    required = {n + suffix for n in BINS} | {f'fonts/{n}' for n in FONTS}
    required |= {'README.md', 'SECURITY.md', 'LICENSE', 'VERSION.txt', 'docs/installation.md', 'examples/welcome.ylang', 'config-examples/browser.conf.example'}
    required |= {'Launch Browser.cmd'} if windows else {'launch-browser.sh', 'install-desktop.sh', 'uninstall-desktop.sh'}
    required |= {f'sites/{site}/home.ylang' for site in ('alice', 'bob', 'personal', 'blog', 'showcase')}
    required |= {f'sites/{site}/assets/cover.png' for site in ('personal', 'blog', 'showcase')}
    if not required <= listed:
        raise ValueError('missing required resources: ' + ', '.join(sorted(required - listed)))
    version_line = (stage / 'VERSION.txt').read_text(encoding='utf-8').splitlines()[0]
    version = version_line.removeprefix('YamiNet ')
    if version_line == version or not re.fullmatch(r'\d+\.\d+\.\d+-(alpha|rc)\.\d+', version) or (expected_version and expected_version != version):
        raise ValueError('invalid or unexpected alpha/RC version')
    if execute:
        if windows != (os.name == 'nt'):
            raise ValueError('native executable verification requires the target OS')
        env = runtime_env()
        with tempfile.TemporaryDirectory(prefix='Yami non development directory ') as cwd:
            for name, label in zip(BINS, ('Yami Browser', 'Yami Node', 'Yami Relay')):
                proc = subprocess.run([str(stage / (name + suffix)), '--version'], cwd=cwd, env=env, capture_output=True, text=True, encoding='utf-8', timeout=20, check=True)
                if proc.stdout.strip() != f'{label} {version}':
                    raise ValueError('packaged executable version mismatch')
            for site in ('alice', 'bob', 'personal', 'blog', 'showcase'):
                subprocess.run([str(stage / ('yami-node' + suffix)), 'site', 'check', str(stage / 'sites' / site)], cwd=cwd, env=env, capture_output=True, timeout=20, check=True)
    return {'manifest': 'PASS', 'completeness': 'PASS', 'version': version, 'native_cli': 'PASS' if execute else 'NOT RUN', 'rust_on_path': False if execute else None}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('stage', type=pathlib.Path)
    parser.add_argument('--no-execute', action='store_true')
    parser.add_argument('--expected-version')
    args = parser.parse_args()
    print(verify(args.stage, not args.no_execute, args.expected_version))
