import hashlib
import importlib.util
import pathlib
import subprocess
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location('package', pathlib.Path(__file__).with_name('package.py'))
PACK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACK)


from verify import verify

class BundleTests(unittest.TestCase):
    def test_layout_manifest_and_upgrade_leave_identity_untouched(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            binaries = root / 'bin'
            binaries.mkdir()
            for name in PACK.BINS:
                (binaries / name).write_bytes(b'fixture executable')
            user = root / 'user-data'
            user.mkdir()
            (user / 'node.key').write_bytes(b'private fixture key')
            (user / 'library.tsv').write_bytes(b'existing bookmarks')
            for release in ('first', 'upgrade'):
                stage = PACK.package('linux', binaries, root / release)
                self.assertTrue((stage / 'fonts/OpenSans-Regular.ttf').is_file())
                self.assertTrue((stage / 'sites/showcase/assets/cover.png').is_file())
                self.assertTrue((stage / 'launch-browser.sh').is_file())
                self.assertFalse((stage / 'node.key').exists())
                for line in (stage / 'MANIFEST.sha256').read_text().splitlines():
                    digest, name = line.split('  ', 1)
                    self.assertEqual(digest, hashlib.sha256((stage / name).read_bytes()).hexdigest())
            self.assertEqual((user / 'node.key').read_bytes(), b'private fixture key')
            self.assertEqual((user / 'library.tsv').read_bytes(), b'existing bookmarks')

    def test_windows_zip_contains_only_release_files_and_launch_method(self):
        import zipfile
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            binaries = root / 'bin'
            binaries.mkdir()
            for name in PACK.BINS:
                (binaries / (name + '.exe')).write_bytes(b'fixture executable')
            stage = PACK.package('windows', binaries, root / 'output')
            self.assertTrue((stage / 'Launch Browser.cmd').is_file())
            with zipfile.ZipFile(next((root / 'output').glob('*.zip'))) as archive:
                names = archive.namelist()
                self.assertTrue(any(n.endswith('/fonts/DejaVu-LICENSE.txt') for n in names))
                executables = sorted(pathlib.PurePosixPath(n).name for n in names if n.endswith('.exe'))
                self.assertEqual(executables, sorted(n + '.exe' for n in PACK.BINS))

    def test_linux_launcher_lifecycle_with_spaces_retains_user_data(self):
        import os
        import shutil
        if os.name == 'nt':
            self.skipTest('Linux desktop launcher')
        with tempfile.TemporaryDirectory(prefix='yami bundle ') as tmp:
            root = pathlib.Path(tmp)
            for name in ('install-desktop.sh', 'uninstall-desktop.sh'):
                shutil.copy2(PACK.ROOT / 'packaging/linux' / name, root / name)
            private = root / 'node.key'
            private.write_bytes(b'identity fixture')
            env = dict(os.environ, XDG_DATA_HOME=str(root / 'desktop data'))
            subprocess.run(['sh', str(root / 'install-desktop.sh')], env=env, check=True, capture_output=True)
            launcher = root / 'desktop data/applications/yaminet.desktop'
            self.assertIn('Exec="', launcher.read_text())
            subprocess.run(['sh', str(root / 'uninstall-desktop.sh')], env=env, check=True, capture_output=True)
            self.assertFalse(launcher.exists())
            self.assertEqual(private.read_bytes(), b'identity fixture')

    def test_completeness_rejects_deleted_required_file_even_with_new_manifest(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp); binaries = root / 'bin'; binaries.mkdir()
            for name in PACK.BINS: (binaries / name).write_bytes(b'fixture executable')
            stage = PACK.package('linux', binaries, root / 'output')
            verify(stage, execute=False)
            missing = 'fonts/OpenSans-Regular.ttf'
            (stage / missing).unlink()
            manifest = stage / 'MANIFEST.sha256'
            manifest.write_text(''.join(line+'\n' for line in manifest.read_text().splitlines() if not line.endswith('  '+missing)))
            with self.assertRaisesRegex(ValueError, 'missing required resources'):
                verify(stage, execute=False)

    def test_extra_and_unsafe_manifest_entries_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp); binaries = root / 'bin'; binaries.mkdir()
            for name in PACK.BINS: (binaries / name).write_bytes(b'fixture executable')
            stage = PACK.package('linux', binaries, root / 'output')
            (stage / 'unexpected.txt').write_text('not manifested')
            with self.assertRaisesRegex(ValueError, 'exactly cover'):
                verify(stage, execute=False)
            (stage / 'MANIFEST.sha256').write_text('0'*64+'  ../outside\n')
            with self.assertRaisesRegex(ValueError, 'unsafe'):
                verify(stage, execute=False)

    def test_missing_binary_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            with self.assertRaises(ValueError):
                PACK.package('linux', root, root / 'output')
