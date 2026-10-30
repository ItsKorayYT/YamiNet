import hashlib
import importlib.util
import pathlib
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location('package', pathlib.Path(__file__).with_name('package.py'))
PACK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACK)


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

    def test_missing_binary_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            with self.assertRaises(ValueError):
                PACK.package('linux', root, root / 'output')
