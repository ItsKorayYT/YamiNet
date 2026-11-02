"""An early successful container exit must never become benchmark evidence."""
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch


@unittest.skipUnless(sys.platform == 'linux', 'Linux Docker lab wrapper')
class LabExitTests(unittest.TestCase):
    def test_zero_exit_without_measurements_fails(self):
        spec = importlib.util.spec_from_file_location(
            'chain_container_lab', Path(__file__).with_name('yamichain-container-lab.py'))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            with patch.object(sys, 'argv', ['lab', '--case', 'smoke4', '--output', directory]), patch.object(
                    module.subprocess, 'run', return_value=SimpleNamespace(returncode=0)):
                with self.assertRaises(SystemExit) as stopped:
                    module.main()
            self.assertEqual(stopped.exception.code, 1)
            report = json.loads((Path(directory) / 'smoke4.json').read_text())
            self.assertEqual(report['result'], 'FAIL')
            self.assertEqual(report['container_exit_code'], 0)


if __name__ == '__main__':
    unittest.main()
