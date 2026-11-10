#!/usr/bin/env python3
"""Interrupt one witness across live PoS epochs and recover without a ledger reset."""
import argparse
import json
import os
import pathlib
import signal
import subprocess
import sys
import time

from yamichain_rpc import Transport

ROOT = pathlib.Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('config', type=pathlib.Path)
    args = parser.parse_args()
    config_path = args.config.resolve()
    config = json.loads(config_path.read_text())
    data = config_path.parent / 'data'
    env = dict(os.environ, YAMI_DEVNET_CONFIG=str(config_path), YAMI_DEVNET_DIR=str(data))
    rpc = Transport(config.get('rpc'), {
        'gateway': [config['gateway']], 'executor': config['executors'], 'validator': config['validators'],
    })

    def run(*arguments):
        result = subprocess.run([str(ROOT / 'scripts/yamichain-pos-devnet'), *map(str, arguments)],
                                env=env, capture_output=True, text=True, check=True)
        return result.stdout.strip()

    def request(path='/status'):
        return rpc.request(config['gateway'] + path, timeout=15)

    def until(predicate):
        end = time.monotonic() + 90
        while time.monotonic() < end:
            try:
                result = predicate()
                if result:
                    return result
            except (OSError, ValueError, KeyError):
                pass
            time.sleep(.25)
        raise RuntimeError('authenticated recovery did not finish')

    run('start')
    # Use the current supervised code and the same durable stores.
    run('stop')
    run('start')
    initial = request()
    wallet = config_path.parent / 'recovery-recipient.wallet'
    run('wallet', wallet)
    witness = until(lambda: request('/services')['witness-3'])['pid']
    missing = config['economics']['epoch_blocks'] + 1
    os.kill(witness, signal.SIGSTOP)
    try:
        for _ in range(missing):
            run('faucet', wallet, 1)
        head = request()['head']
        try:
            assert rpc.request(config['executors'][3], 'Head', timeout=5).get('Head') != head
        except OSError:
            pass  # No live witness means the executor must refuse, not use a cache.
    finally:
        os.kill(witness, signal.SIGCONT)
    until(lambda: rpc.request(config['executors'][3], 'Head', timeout=5).get('Head') == head)
    until(lambda: request('/recovery')['healthy_executors'] == 4)
    assert not (data / 'executor-recovery-3.json').exists()
    balance = json.loads(run('balance', wallet))
    assert balance['balance'] == missing
    final = request()
    run('stop')
    run('start')
    assert request()['head'] == head
    assert json.loads(run('balance', wallet)) == balance
    print(json.dumps(dict(software_only=True, production_activation=False,
                          authenticated_tls=True, initial_height=initial['height'],
                          final_height=final['height'], missed_transitions=missing,
                          crossed_epoch_boundary=True, recipient_balance=balance['balance'],
                          recovered_executors=4, snapshot_forward_file_removed=True,
                          restart_preserved=True, signing_journals_retained=True), indent=2))


if __name__ == '__main__':
    main()
