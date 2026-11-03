"""Checks that the resource launcher enforces limits rather than labeling them."""
import importlib.util
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('stress', Path(__file__).with_name('yamichain-stress.py'))
stress = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stress)


class ValidatorLimitTests(unittest.TestCase):
    def test_no_limit_and_invalid_limits(self):
        self.assertEqual(stress.validator_command(Path('/a binary'), 0), ['/a binary'])
        for value in [-1, 1, 127, 65537]:
            with self.assertRaises(ValueError):
                stress.validator_command(Path('/binary'), value)
        with patch.object(stress.shutil, 'which', return_value=None):
            with self.assertRaises(RuntimeError):
                stress.validator_command(Path('/binary'), 2048)

    @unittest.skipUnless(stress.shutil.which('prlimit'), 'requires Linux prlimit')
    def test_child_observes_exact_limit_before_running(self):
        command = stress.validator_command(Path(sys.executable), 2048)
        actual = subprocess.check_output(command + ['-c',
            'import resource; print(resource.getrlimit(resource.RLIMIT_AS))'], text=True)
        self.assertEqual(actual.strip(), '(2147483648, 2147483648)')


if __name__ == '__main__':
    unittest.main()
