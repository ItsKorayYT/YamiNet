import json
import pathlib
import subprocess
import tempfile
import unittest
from yamichain_wallet_config import export_wallet_config


class WalletConfigTests(unittest.TestCase):
    def test_export_has_only_public_pins_and_gateway_credential_and_never_overwrites(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            ca, key, token = root/'ca.pem', root/'server.key', root/'gateway.token'
            subprocess.run(['openssl','req','-x509','-newkey','rsa:2048','-nodes','-days','1',
                            '-keyout',str(key),'-out',str(ca),'-subj','/CN=wallet export test'],
                           check=True,capture_output=True)
            token.write_text('a'*64);token.chmod(0o600)
            network = dict(gateway='https://localhost:1234',
                           witnesses=['private-backend'],
                           rpc=dict(ca_file=str(ca),key_file=str(key),certificate_file=str(ca),
                                    tokens=dict(gateway_file=str(token),executor_file='executor-secret',validator_file='validator-secret')))
            policy = {'chain':24}
            path = export_wallet_config(network,policy,root/'wallet',lambda _:None)
            profile = json.loads(path.read_text())
            self.assertEqual(set(profile['rpc']),{'ca_file','tokens'})
            self.assertEqual(set(profile['rpc']['tokens']),{'gateway_file'})
            self.assertNotIn('witnesses',profile)
            self.assertNotIn('server.key',path.read_text())
            self.assertEqual(json.loads((root/'wallet/policy.json').read_text()),policy)
            for file in path.parent.iterdir():
                self.assertEqual(file.stat().st_mode & 0o777,0o600)
            with self.assertRaises(FileExistsError):
                export_wallet_config(network,policy,root/'wallet',lambda _:None)
            network['wallet_gateway_seeds']=['http://127.0.0.1:1234']
            with self.assertRaises(ValueError):
                export_wallet_config(network,policy,root/'bad',lambda _:None)
            self.assertFalse((root/'bad').exists())


if __name__ == '__main__':
    unittest.main()
