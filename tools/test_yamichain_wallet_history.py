import json
import os
import pathlib
import runpy
import tempfile
import unittest
from unittest import mock
from yamichain_wallet_history import History

ROOT = pathlib.Path(__file__).resolve().parents[1]


class HistoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.wallet = pathlib.Path(self.temp.name) / 'alice'
        self.bridge = mock.Mock(side_effect=self.crypto)

    def tearDown(self):
        self.temp.cleanup()

    @staticmethod
    def crypto(op, **kw):
        # Exercise persistence independently; the Rust tests check actual crypto.
        if op == 'wallet-history-seal':
            return {'ciphertext': json.dumps(kw['entries'])}
        return json.loads(kw['envelope']['ciphertext'])

    def journal(self, policy=None):
        return History(self.wallet, policy or {'chain': 24}, self.bridge, [1]*32)

    def test_restart_retains_exact_submission_private_permissions_and_network_isolation(self):
        entry = {'payment': {'transaction': [3]*32}, 'nonce': 5}
        first = self.journal()
        with first.locked():
            first.append(entry)
        self.assertEqual(first.path.stat().st_mode & 0o777, 0o600)
        with self.journal().locked() as second:
            self.assertEqual(second.entries, [entry])
        with self.journal({'chain': 25}).locked() as other:
            self.assertEqual(other.entries, [])

    def test_reject_insecure_history_and_lock_or_symlink(self):
        journal = self.journal()
        journal.path.write_text('{}')
        journal.path.chmod(0o644)
        with self.assertRaises(ValueError), journal.locked():
            pass
        journal.path.unlink()
        journal.path.symlink_to(self.wallet)
        with self.assertRaises(OSError), journal.locked():
            pass

    def test_failed_seal_preserves_previous_journal(self):
        journal = self.journal()
        with journal.locked():
            journal.append({'nonce': 1})
        before = journal.path.read_bytes()
        with journal.locked():
            self.bridge.side_effect = ValueError('encryption failed')
            with self.assertRaises(ValueError):
                journal.append({'nonce': 2})
        self.assertEqual(before, journal.path.read_bytes())

    def test_history_paginates_newest_first_and_bounds_validation_work(self):
        d = runpy.run_path(str(ROOT/'scripts/yamichain-devnet'))
        context = d['transaction_history'].__globals__
        pinned = {'chain':24}
        journal = History(self.wallet,pinned,self.bridge,[1]*32)
        with journal.locked():
            for n in range(25):
                journal.append({'payment':{},'nonce':n})
        verified = mock.Mock(return_value={'status':'unknown'})
        with mock.patch.dict(context, DATA=pathlib.Path(self.temp.name),policy=lambda:pinned,
                             bridge=self.bridge,wallet_secret=lambda _:[1]*32,verified_submission=verified):
            result = d['transaction_history'](self.wallet)
            self.assertEqual(result['total'],25)
            self.assertEqual(result['next_offset'],20)
            self.assertEqual(len(result['transactions']),20)
            self.assertEqual(result['transactions'][0]['nonce'],24)
            self.assertEqual(verified.call_count,20)
            result = d['transaction_history'](self.wallet,20,20)
            self.assertEqual(len(result['transactions']),5)
            self.assertIsNone(result['next_offset'])
            with self.assertRaises(ValueError):
                d['transaction_history'](self.wallet,0,101)

    def test_status_uses_verified_certificate_and_full_native_prefix(self):
        d = runpy.run_path(str(ROOT/'scripts/yamichain-devnet'))
        context = d['verified_submission'].__globals__
        payment = {'committee':[1]*32,'previous':{'epoch':2},'transaction':[3]*32}
        statement = dict(payment)
        receipt = {'finality':{'record':[9]},'certificate':{'untrusted':'wrapper'},'status':'finalized'}
        def http(url):
            height = int(url.rsplit('/',1)[1])
            return receipt if height == 3 else {'finality':{'record':[height]}}
        verifier = mock.Mock(return_value={'certificate':{'certificate':{'statement':statement}},'block':'verified-block'})
        with mock.patch.dict(context, http=http, bridge=verifier):
            status = d['verified_submission']({'native_scope':[5]*32},payment)
            self.assertEqual(status,{'status':'finalized','height':3,'block':'verified-block'})
            self.assertEqual(verifier.call_args.kwargs['records'],[[1],[2]])
            verifier.return_value['certificate']['certificate']['statement'] = dict(statement,transaction=[4]*32)
            self.assertEqual(d['verified_submission']({'native_scope':[5]*32},payment)['status'],'superseded')
            verifier.side_effect = ValueError('unverifiable native certificate')
            with self.assertRaises(ValueError):
                d['verified_submission']({'native_scope':[5]*32},payment)

    def test_offline_history_remains_available_with_unknown_status(self):
        d = runpy.run_path(str(ROOT/'scripts/yamichain-devnet'))
        context = d['transaction_history'].__globals__
        pinned = {'chain':24}
        directory = pathlib.Path(self.temp.name)
        (directory/'policy.json').write_text(json.dumps(pinned))
        journal = History(self.wallet,pinned,self.bridge,[1]*32)
        with journal.locked():
            journal.append({'payment':{'transaction':[3]*32},'amount':10,'transaction':'ab'})
        def bridge(op, **kw):
            return {} if op == 'validate-policy' else self.crypto(op, **kw)
        with mock.patch.dict(context, DATA=directory, bridge=bridge,
                             wallet_secret=lambda _: [1]*32,
                             policy=mock.Mock(side_effect=TimeoutError()),
                             verified_submission=mock.Mock(side_effect=TimeoutError())):
            result = d['transaction_history'](self.wallet)['transactions']
        self.assertEqual(result[0]['status'],'unknown')
        self.assertEqual(result[0]['amount'],10)
        self.assertNotIn('payment',result[0])

    def test_no_submission_when_journal_cannot_be_saved_and_identical_retry(self):
        d = runpy.run_path(str(ROOT/'scripts/yamichain-devnet'))
        context = d['send'].__globals__
        pinned = {'committee': {'chain': 24}}
        payment = {'previous': {'epoch': 1}, 'transaction': [3]*32}
        bridge = mock.Mock(side_effect=lambda op, **kw: payment if op == 'payment' else self.crypto(op, **kw))
        http = mock.Mock(return_value={'head': payment['previous']})
        with mock.patch.dict(context, policy=lambda: pinned, wallet_secret=lambda _: [1]*32,
                             balance=lambda _: {'nonce': 5}, fee_estimate=lambda _: {'fee':2, 'economics_enabled':True},
                             bridge=bridge, http=http):
            with mock.patch.object(History, 'append', side_effect=OSError('disk full')):
                with self.assertRaises(OSError):
                    d['send'](self.wallet, 'bob', 10)
            self.assertFalse(any(call.args[0].endswith('/send') for call in http.call_args_list))
            # Timeout retains exact randomized envelopes for a subsequent process.
            http.side_effect = lambda url, *args, **kw: (_ for _ in ()).throw(TimeoutError()) if url.endswith('/send') else {'head': payment['previous']}
            with self.assertRaisesRegex(RuntimeError, 'retained transaction'):
                d['send'](self.wallet, 'bob', 10)
            with History(self.wallet,pinned,bridge,[1]*32).locked() as saved:
                self.assertEqual(saved.entries[0]['payment'], payment)
            submitted = mock.Mock(return_value={'status':'finalized'})
            with mock.patch.dict(context, submit_recorded=submitted):
                d['retry_submission'](self.wallet, bytes([3]*32).hex())
            self.assertEqual(submitted.call_args.args[1]['payment'], payment)
            # A changed head must verify finality instead of resubmitting.
            http.side_effect = None
            http.return_value = {'head': {'epoch': 2}}
            verified = mock.Mock(return_value={'status':'superseded'})
            with mock.patch.dict(context, verified_submission=verified, submit_recorded=submitted):
                d['retry_submission'](self.wallet, bytes([3]*32).hex())
            self.assertEqual(submitted.call_count, 1)


if __name__ == '__main__':
    unittest.main()
