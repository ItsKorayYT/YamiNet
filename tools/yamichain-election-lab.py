#!/usr/bin/env python3
"""Isolated signed-input election/rotation and discovery campaign, loopback only."""
import argparse
import asyncio
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import resource
import signal
import subprocess
import time

spec = importlib.util.spec_from_file_location('stress', Path(__file__).with_name('yamichain-stress.py'))
stress = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stress)


def hashes(path):
    return {str(p.relative_to(path)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in path.rglob('*') if p.is_file()}


async def run(args):
    root = args.directory.resolve()
    if root.exists() and any(root.iterdir()):
        raise RuntimeError('fresh campaign directory required')
    root.mkdir(parents=True, exist_ok=True)
    binary = args.binary.resolve()
    plan = root / 'plan.json'
    pin = subprocess.check_output([str(binary), 'fixture', str(plan), str(args.base_port)], text=True).strip()
    description = json.loads(subprocess.check_output([str(binary), 'describe', str(plan), pin], text=True))
    indices = description['indices']
    initial = description['committees'][0]
    assert len(initial) == 4 and len(description['committees']) == 3
    assert description['committees'][0] != description['committees'][1], 'fixture did not exercise churn'
    groups = {i: i % 2 for i in range(8)}
    groups.update({i: int(n >= 2) for n, i in enumerate(initial)})
    proxy = stress.Proxies(8, args.base_port, args.proxy_base, 0)
    proxy.group = lambda i: groups[i]
    await proxy.start()
    processes, logs, offsets, states, observed = {}, [], {}, {}, {}
    discoveries = set()
    peaks = {i: 0 for i in indices}
    faults = []
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    started = time.monotonic()
    barrier = int((time.time() + 3) * 1000)
    partition = settled = killed = None
    partition_done = restart_done = False
    probe_task = None
    result = dict(result='FAIL', scope='isolated signed simulated backing/entropy; loopback development-key validators; no live PoS',
                  generated_at=datetime.now(timezone.utc).isoformat(), indices=indices, committees=description['committees'],
                  plan_hash=pin, genesis=description['genesis'], binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                  driver_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                  cgroup={n: stress.limit(n) for n in ['cpu.max', 'memory.max']},
                  per_process_address_space_limit_bytes=2147483648, memory_limit_scope='RLIMIT_AS, not cgroup RSS',
                  public_deployment=False, economically_secure_pos=False, adversarial=args.adversarial)

    def command(index, fixture=plan, fixture_pin=pin):
        return stress.validator_command(binary, 2048) + ['run', str(fixture), fixture_pin, str(root / f'node-{index}'),
                    str(index), str(args.base_port), str(args.proxy_base + groups[index] * 100),
                    str(args.discovery_base), '12', '120', '1', str(barrier)]

    def launch(index):
        path = root / f'node-{index}-{len(logs)}.log'
        log = open(path, 'w', encoding='utf-8')
        logs.append((path, log))
        processes[index] = subprocess.Popen(command(index), stdout=log, stderr=subprocess.STDOUT)
        offsets[path] = (index, 0, '')

    def progress():
        for path, (index, offset, carry) in list(offsets.items()):
            with open(path, encoding='utf-8') as f:
                f.seek(offset)
                text = carry + f.read()
                offset = f.tell()
            lines = text.split('\n')
            offsets[path] = (index, offset, lines.pop())
            for line in lines:
                if line.startswith('{') and line.endswith('}'):
                    d = json.loads(line)
                    if d.get('discovery_verified'):
                        discoveries.add(index)
                    if 'height' in d:
                        assert not d['payments_enabled'] and not d['production_ready']
                        pair = (d['block'], d['state_root'])
                        assert observed.setdefault(d['height'], pair) == pair, 'conflicting finality'
                        states[index] = d

    async def probes():
        raw = json.loads(plan.read_text())['rounds'][0]['announcements'][0]
        forged = bytearray(raw)
        forged[-1] ^= 1
        payload = (1).to_bytes(4, 'big') + len(forged).to_bytes(4, 'big') + forged
        for name, response in [('oversized-length', b'\xff' * 4), ('forged-signature', len(payload).to_bytes(4, 'big') + payload)]:
            async def reply(reader, writer, response=response):
                try:
                    await asyncio.wait_for(reader.readexactly(9), 1)
                    writer.write(response)
                    await writer.drain()
                finally:
                    writer.close()
            server = await asyncio.start_server(reply, '127.0.0.1', 0)
            port = server.sockets[0].getsockname()[1]
            try:
                child = await asyncio.create_subprocess_exec(*stress.validator_command(binary, 2048), 'probe', str(plan), pin, str(port), '1', stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
                _, error = await asyncio.wait_for(child.communicate(), 5)
                assert child.returncode == 1, name
                assert ('cap' in error.decode() if name == 'oversized-length' else 'Signature' in error.decode()), error
                faults.append(dict(kind=name, rejected=True))
            finally:
                server.close()
                await server.wait_closed()
        # Bound in-flight idle clients, then demonstrate service recovery.
        readers_writers = []
        for _ in range(16):
            readers_writers.append(await asyncio.open_connection('127.0.0.1', args.discovery_base + indices[0]))
        await asyncio.sleep(1.2)
        for _, writer in readers_writers:
            writer.close()
        child = await asyncio.create_subprocess_exec(*stress.validator_command(binary, 2048), 'probe', str(plan), pin,
                    str(args.discovery_base + indices[0]), '1', stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
        _, error = await asyncio.wait_for(child.communicate(), 5)
        assert child.returncode == 0, error
        faults.append(dict(kind='idle-discovery-clients', clients=16, service_recovered=True))

    try:
        for i in indices:
            launch(i)
        shutdown = False
        while True:
            await asyncio.sleep(0.05)
            progress()
            elapsed = time.monotonic() - started
            for i in indices:
                peaks[i] = max(peaks[i], stress.proc_sample(processes[i].pid)['peak'])
            ready = len(states) == len(indices) and min(d['height'] for d in states.values()) >= 2
            if args.adversarial and ready and not partition_done:
                assert max(d['height'] for d in states.values()) < 5, 'missed initial-committee partition window'
                proxy.partition()
                partition, partition_done = elapsed, True
                faults.append(dict(kind='partition', seconds=round(elapsed, 3)))
                probe_task = asyncio.create_task(probes())
            if partition is not None:
                if elapsed - partition >= 2 and settled is None:
                    settled = max(d['height'] for d in states.values())
                if elapsed - partition >= 7:
                    assert settled == max(d['height'] for d in states.values()), 'partition bypassed quorum'
                    proxy.partitioned = False
                    partition = None
                    victim = initial[0]
                    processes[victim].kill()
                    processes[victim].wait()
                    killed = elapsed
                    faults.append(dict(kind='partition-healed-and-SIGKILL', height=settled, index=victim, seconds=round(elapsed, 3)))
            if killed is not None and not restart_done and elapsed - killed >= 2:
                launch(initial[0])
                restart_done = True
                faults.append(dict(kind='restart-entire-store', seconds=round(elapsed, 3)))
            all_target = len(states) == len(indices) and all(d['height'] == 12 for d in states.values())
            faults_done = not args.adversarial or (restart_done and probe_task.done())
            if all_target and faults_done and not shutdown:
                if probe_task:
                    await probe_task
                result['seconds_to_all_target'] = round(elapsed, 3)
                for p in processes.values():
                    if p.poll() is None:
                        p.send_signal(signal.SIGINT)
                shutdown = True
            if all(p.poll() is not None for p in processes.values()):
                break
            if elapsed > 150:
                raise RuntimeError('campaign deadline')
        assert all(p.returncode == 0 for p in processes.values()), {i: p.returncode for i, p in processes.items()}
        assert discoveries == set(indices), 'a validator did not discover authenticated candidates'
        final = [json.loads(subprocess.check_output([str(binary), 'inspect', str(plan), pin, str(root / f'node-{i}')], text=True)) for i in indices]
        assert all(d['height'] == 12 for d in final)
        assert len({(d['block'], d['state_root']) for d in final}) == 1
        if args.adversarial:
            fork = root / 'conflicting-plan.json'
            fork_pin = subprocess.check_output([str(binary), 'fixture', str(fork), str(args.base_port)], text=True).strip()
            fork_description = json.loads(subprocess.check_output([str(binary), 'describe', str(fork), fork_pin], text=True))
            index = next(i for i in indices if i in fork_description['indices'])
            before_hashes = hashes(root / f'node-{index}')
            rejected = subprocess.run(command(index, fork, fork_pin), capture_output=True, text=True, timeout=5)
            assert rejected.returncode == 1 and 'conflicting durable record' in rejected.stderr
            assert before_hashes == hashes(root / f'node-{index}')
            faults.append(dict(kind='conflicting-election-plan-restart', rejected_before_signing=True))
            snapshot = root / f'node-{index}' / 'discovery-00000000000000000001.bin'
            saved = snapshot.read_bytes()
            snapshot.write_bytes(saved[:-1] + bytes([saved[-1] ^ 1]))
            try:
                rejected = subprocess.run(command(index), capture_output=True, text=True, timeout=5)
                assert rejected.returncode == 1 and 'Encoding' in rejected.stderr
            finally:
                snapshot.write_bytes(saved)
            assert before_hashes == hashes(root / f'node-{index}')
            faults.append(dict(kind='corrupt-discovery-restart', rejected_before_signing=True))
        after = resource.getrusage(resource.RUSAGE_CHILDREN)
        result.update(result='PASS', final_height=12, finalized_block=final[0]['block'], finalized_state=final[0]['state_root'],
                      discovered_nodes=sorted(discoveries), finalized_heights_observed=sorted(observed),
                      validator_and_probe_cpu_seconds=round(after.ru_utime + after.ru_stime - before.ru_utime - before.ru_stime, 3))
    except Exception as e:
        result['failure'] = str(e)
        result['log_tails'] = {p.name: p.read_text()[-2500:] for p, _ in logs}
    finally:
        if probe_task and not probe_task.done():
            probe_task.cancel()
            await asyncio.gather(probe_task, return_exceptions=True)
        for p in processes.values():
            if p.poll() is None:
                p.kill()
            p.wait()
        await proxy.close()
        for _, log in logs:
            log.close()
        result.update(elapsed_seconds=round(time.monotonic() - started, 3), faults=faults,
                      sampled_per_node_peak_rss_bytes=peaks, stores={i: stress.usage(root / f'node-{i}') for i in indices})
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps(result, indent=2))
    return result['result'] == 'PASS'


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, default=Path('target/release/yamichain-validator-lab'))
    p.add_argument('--directory', type=Path, required=True)
    p.add_argument('--report', type=Path, required=True)
    p.add_argument('--adversarial', action='store_true')
    p.add_argument('--base-port', type=int, default=20000)
    p.add_argument('--proxy-base', type=int, default=30000)
    p.add_argument('--discovery-base', type=int, default=40000)
    args = p.parse_args()
    for port in [args.base_port, args.proxy_base, args.discovery_base]:
        if not 1024 <= port <= 64000:
            p.error('ports must be 1024..64000')
    raise SystemExit(0 if asyncio.run(run(args)) else 1)


if __name__ == '__main__':
    main()
