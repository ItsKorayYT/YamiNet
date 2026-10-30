#!/usr/bin/env python3
"""Run package/installer lifecycle in isolated paths containing spaces."""
import argparse
import hashlib
import json
import os
import pathlib
import platform
import shutil
import subprocess
import tarfile
import tempfile
import time
import zipfile
from verify import runtime_env, verify


def run(cmd, env, cwd, timeout=45):
    return subprocess.run([str(p) for p in cmd], env=env, cwd=cwd, capture_output=True, text=True, encoding='utf-8', errors='replace', timeout=timeout, check=True)


def extract(archive, target):
    if archive.name.endswith('.zip'):
        with zipfile.ZipFile(archive) as bundle:
            for item in bundle.infolist():
                path = (target / item.filename).resolve()
                if not path.is_relative_to(target.resolve()) or '\\' in item.filename:
                    raise ValueError('unsafe ZIP member')
            bundle.extractall(target)
    else:
        with tarfile.open(archive) as bundle:
            bundle.extractall(target, filter='data')
    dirs = list(target.iterdir())
    if len(dirs) != 1 or not dirs[0].is_dir():
        raise ValueError('package needs one top-level application directory')
    return dirs[0]


def lifecycle(args):
    report = {'schema': 'yaminet-package-lifecycle-1', 'platform': platform.platform(), 'checks': {}, 'gui': 'NOT RUN', 'cross_version_upgrade': 'NOT RUN'}
    with tempfile.TemporaryDirectory(prefix='Yami lifecycle path with spaces ') as tmp:
        root = pathlib.Path(tmp).resolve()
        app = extract(args.archive.resolve(), root / 'portable application')
        env = runtime_env()
        data = root / 'persistent user data'
        data.mkdir()
        env.update(LOCALAPPDATA=str(data), XDG_DATA_HOME=str(data), YAMINET_PEER_DIR=str(data / 'peers'), YAMINET_BROWSER_DATA_DIR=str(data / 'library'))
        cwd = root / 'non development working directory'; cwd.mkdir()
        report['checks']['portable'] = verify(app)
        suffix = '.exe' if os.name == 'nt' else ''
        keys = data / 'identity'
        run([app / ('yami-node' + suffix), 'identity', 'new', '--dir', keys], env, cwd)
        library = data / 'library'; library.mkdir()
        (library / 'library.tsv').write_text('YAMILIBRARY1\nhistory\t0\nhome\t\nb\t\tExisting bookmark\tlocal:welcome\n', encoding='utf-8')
        # Use the actual peer CLI and a real generated public identity pin.
        pin = (keys / 'node.id').read_text().strip()
        run([app / ('yami-node' + suffix), 'peer', 'add', pin, '127.0.0.1:7443', '--dir', data / 'peers'], env, cwd)
        protected = [keys / 'node.key', keys / 'node.id', library / 'library.tsv', data / 'peers/peers.tsv']
        hashes = {p: hashlib.sha256(p.read_bytes()).hexdigest() for p in protected}
        def unchanged():
            if any(not p.is_file() or hashlib.sha256(p.read_bytes()).hexdigest() != digest for p, digest in hashes.items()):
                raise ValueError('persistent identity, bookmarks or peer configuration changed')
        if os.name == 'nt':
            import winreg
            key = r'Software\Microsoft\Windows\CurrentVersion\Uninstall\YamiNet'
            try:
                with winreg.OpenKey(winreg.HKEY_CURRENT_USER, key):
                    raise ValueError('existing YamiNet install: use a disposable Windows user profile')
            except FileNotFoundError:
                pass
            menu = pathlib.Path(os.environ['APPDATA']) / 'Microsoft/Windows/Start Menu/Programs/YamiNet'
            if menu.exists():
                raise ValueError('existing Start Menu folder: use a disposable user profile')
            if args.installer is None:
                raise ValueError('native Windows lifecycle requires an NSIS installer')
            dest = root / 'installed application with spaces'
            previous = args.previous_installer or args.installer
            run([previous.resolve(), '/S', f'/D={dest}'], env, cwd)
            if not (dest / 'Uninstall.exe').is_file() or not (menu / 'Yami Browser.lnk').is_file():
                raise ValueError('installer failed to create uninstall program or shortcut')
            old_version = run([dest / 'yami-browser.exe', '--version'], env, cwd).stdout.strip().rsplit(' ', 1)[-1]
            if args.previous_installer and old_version == report['checks']['portable']['version']:
                raise ValueError('previous installer must contain an actually older version')
            report['upgrade_from'] = old_version
            unchanged()
            run([args.installer.resolve(), '/S', f'/D={dest}'], env, cwd)
            report['checks']['installed'] = verify(dest, expected_version=report['checks']['portable']['version'], installed=True)
            unchanged()
            report['cross_version_upgrade'] = 'PASS' if args.previous_installer else 'NOT RUN (same-build reinstall only)'
            # Normal NSIS self-copy spawns a child; verify actual removal, not only parent exit.
            run([dest / 'Uninstall.exe', '/S'], env, cwd)
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline and ((dest / 'yami-browser.exe').exists() or (dest / 'Uninstall.exe').exists() or menu.exists()):
                time.sleep(0.1)
            if dest.exists() or menu.exists():
                raise ValueError('uninstall left application directory or shortcuts')
            try:
                with winreg.OpenKey(winreg.HKEY_CURRENT_USER, key):
                    raise ValueError('uninstall registry entry remains')
            except FileNotFoundError:
                pass
            unchanged()
            report['checks']['installer_upgrade_uninstall'] = 'PASS'
        else:
            run(['sh', app / 'install-desktop.sh'], env, cwd)
            launcher = data / 'applications/yaminet.desktop'
            if not launcher.is_file() or str(app) not in launcher.read_text():
                raise ValueError('desktop launcher does not name the extracted application')
            run(['sh', app / 'uninstall-desktop.sh'], env, cwd)
            if launcher.exists():
                raise ValueError('desktop launcher remains after uninstall')
            unchanged()
            deps = run(['ldd', app / 'yami-browser'], env, cwd).stdout
            if 'not found' in deps:
                raise ValueError('missing runtime shared library')
            report['checks']['desktop_launcher_and_dependencies'] = 'PASS'
            report['uid'] = os.getuid()
        report['checks']['persistent_user_data_survived'] = 'PASS'
        if args.gui:
            # Separate fresh profile: smoke actions may deliberately save test bookmarks.
            env['YAMINET_BROWSER_DATA_DIR'] = str(data / 'gui profile')
            for launch in range(2):
                result = root / f'gui-{launch}.json'
                run([app / ('yami-browser' + suffix), '--desktop-smoke', result], env, cwd, timeout=45)
                observed = json.loads(result.read_text())
                if observed['result'] != 'PASS' or observed['reopened'] != bool(launch):
                    raise ValueError('native GUI smoke/persistence check failed')
                screenshot = args.report.with_name(f'{args.report.stem}-window-{launch}.png')
                shutil.copy2(result.with_suffix('.png'), screenshot)
                if launch == 0 and (root / 'onboarding-smoke.png').exists():
                    shutil.copy2(root / 'onboarding-smoke.png', args.report.with_name(f'{args.report.stem}-onboarding.png'))
                report.setdefault('gui_runs', []).append(observed)
            report['gui'] = 'PASS (native window, logical test actions; no physical keyboard/IME claim)'
        unchanged()
    report['result'] = 'PASS'
    return report


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--archive', type=pathlib.Path, required=True)
    p.add_argument('--installer', type=pathlib.Path)
    p.add_argument('--previous-installer', type=pathlib.Path)
    p.add_argument('--report', type=pathlib.Path, required=True)
    p.add_argument('--gui', action='store_true')
    args = p.parse_args()
    try:
        report = lifecycle(args)
    except Exception as error:
        report = {'schema': 'yaminet-package-lifecycle-1', 'result': 'FAIL', 'error_class': type(error).__name__, 'gui': 'NOT VERIFIED'}
        args.report.write_text(json.dumps(report, indent=2), encoding='utf-8')
        raise
    args.report.write_text(json.dumps(report, indent=2), encoding='utf-8')
    print(json.dumps(report, indent=2))
