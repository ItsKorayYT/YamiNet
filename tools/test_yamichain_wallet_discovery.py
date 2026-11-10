import unittest
from unittest import mock
from yamichain_wallet_discovery import discover


class DiscoveryTests(unittest.TestCase):
    def test_offline_seed_and_foreign_chain_cannot_replace_pin(self):
        rpc = mock.Mock()
        pin = {'chain': 24}
        rpc.request.side_effect = [TimeoutError(), {'chain': 25}, pin]
        url, policy = discover(rpc, ['https://one:1234','https://two:1234','https://three:1234'], pin, lambda _: None)
        self.assertEqual(url, 'https://three:1234')
        self.assertEqual(policy, pin)
        self.assertEqual(rpc.request.call_count, 3)
        self.assertTrue(all(c.kwargs['timeout'] == 3 for c in rpc.request.call_args_list))

    def test_discovery_cannot_bootstrap_trust_or_enable_plaintext(self):
        rpc = mock.Mock()
        for seeds, pin in [(['https://one:1234','https://two:1234'], None),
                           (['https://one:1234','http://127.0.0.1:1234'], {}),
                           (['https://one:1234']*9, {}),
                           (['https://one:1234/path'], {}), ([], {})]:
            with self.assertRaises(ValueError):
                discover(rpc, seeds, pin, lambda _: None)
        rpc.request.assert_not_called()

    def test_invalid_policy_or_unavailable_network_fails_closed(self):
        rpc = mock.Mock()
        rpc.request.return_value = {'invalid': True}
        validate = mock.Mock(side_effect=ValueError('bad signature keys'))
        with self.assertRaises(RuntimeError):
            discover(rpc,['https://one:1234'],None,validate)
        with self.assertRaises(ValueError):
            discover(rpc,['https://one:1234'],{},validate)


if __name__ == '__main__':
    unittest.main()
