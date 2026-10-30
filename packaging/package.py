#!/usr/bin/env python3
"""Stage only an explicit allowlist; never copy user state or identities."""
import argparse
import hashlib
import json
import pathlib
import shutil
import subprocess
import tarfile
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
BINS = ('yami-browser', 'yami-node', 'yami-relay')


def package(platform, binaries, output):
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--no-deps', '--format-version=1', '--locked'], cwd=ROOT))
    version = next(p['version'] for p in metadata['packages'] if p['name'] == 'yami-browser')
    name = f'YamiNet-{version}-{platform}-x86_64'
    output.mkdir(parents=True, exist_ok=True)
    stage = output / name
    if stage.exists():
        raise ValueError(f'{stage} already exists; choose a fresh output directory')
    stage.mkdir()
    suffix = '.exe' if platform == 'windows' else ''
    for binary in BINS:
        source = binaries / (binary + suffix)
        if not source.is_file():
            raise ValueError(f'missing release executable: {source}')
        shutil.copy2(source, stage / source.name)
    for filename in ('README.md', 'SECURITY.md', 'LICENSE'):
        shutil.copy2(ROOT / filename, stage / filename)
    for directory in ('docs', 'examples', 'sites'):
        shutil.copytree(ROOT / directory, stage / directory)
    shutil.copytree(ROOT / 'crates/yami-render/assets/fonts', stage / 'fonts')
    shutil.copytree(ROOT / 'deploy', stage / 'config-examples')
    for path in (ROOT / f'packaging/{platform}').iterdir():
        if path.is_file() and path.suffix != '.nsi':
            shutil.copy2(path, stage / path.name)
    (stage / 'VERSION.txt').write_text(
        f'YamiNet {version}\n{platform} x86_64\nExperimental alpha; unsigned.\n', encoding='utf-8')
    files = sorted(p for p in stage.rglob('*') if p.is_file())
    forbidden = {'node.key', 'peers.tsv', 'library.tsv', 'preferences.conf'}
    assert not any(p.name in forbidden or p.suffix in ('.lock', '.cache') for p in files)
    (stage / 'MANIFEST.sha256').write_text(''.join(
        f'{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.relative_to(stage).as_posix()}\n'
        for p in files), encoding='utf-8')
    if platform == 'windows':
        archive = output / f'{name}.zip'
        with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as bundle:
            for p in sorted(stage.rglob('*')):
                if p.is_file():
                    bundle.write(p, p.relative_to(output))
    else:
        archive = output / f'{name}.tar.gz'
        with tarfile.open(archive, 'w:gz') as bundle:
            bundle.add(stage, arcname=name)
    (output / (archive.name + '.sha256')).write_text(
        f'{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n', encoding='utf-8')
    print(stage)
    return stage


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--platform', choices=('windows', 'linux'), required=True)
    parser.add_argument('--binaries', type=pathlib.Path, default=ROOT / 'target/release')
    parser.add_argument('--output', type=pathlib.Path, default=ROOT / 'dist')
    args = parser.parse_args()
    package(args.platform, args.binaries.resolve(), args.output.resolve())
