#!/usr/bin/env python3
"""Bounded process stress and measurements for the local non-financial chain.

Encrypted TCP is forwarded unchanged. Two proxy banks identify source groups,
allow a genuine network partition, and count transport bytes by destination.
This is loopback fault injection, not a WAN or shielded-payment benchmark.
"""
import argparse
import asyncio
from datetime import datetime, timezone
import errno
import hashlib
from yamichain_safety import capture_safety, preserved, observed_anchor
import json
import os
from pathlib import Path
import platform
import resource
import random
import signal
import shutil
import subprocess
import time


def proc_sample(pid):
    try:
        fields = dict(line.split(':', 1) for line in Path(f'/proc/{pid}/status').read_text().splitlines() if ':' in line)
        try:
            io = dict(line.split(':', 1) for line in Path(f'/proc/{pid}/io').read_text().splitlines())
        except PermissionError:
            io = {}  # Respect process isolation; report unavailable counters.
        return {'rss': int(fields.get('VmRSS', '0 kB').split()[0]) * 1024,
                'peak': int(fields.get('VmHWM', '0 kB').split()[0]) * 1024,
                'read_bytes': int(io.get('read_bytes', 0)), 'write_bytes': int(io.get('write_bytes', 0)), 'io_available': bool(io)}
    except (FileNotFoundError, ProcessLookupError):
        return dict(rss=0, peak=0, read_bytes=0, write_bytes=0, io_available=False)


def limit(name):
    p = Path('/sys/fs/cgroup') / name
    return p.read_text().strip() if p.exists() else None


def network_bytes():
    return {kind: int(Path(f'/sys/class/net/lo/statistics/{kind}_bytes').read_text())
            for kind in ('rx', 'tx')}


def usage(path):
    files = []
    for p in path.rglob('*'):
        try:
            st = p.stat()
            if p.is_file():
                files.append((p, st))
        except FileNotFoundError:
            pass  # Candidate cleanup can race a measurement, never verification.
    return {'logical_bytes': sum(st.st_size for p, st in files),
            'allocated_bytes': sum(st.st_blocks * 512 for p, st in files), 'files': len(files),
            'wal_bytes': sum(st.st_size for p, st in files if p.suffix == '.wal'),
            'signing_bytes': sum(st.st_size for p, st in files if p.name.startswith(('signed-', 'signing-')))}


def validator_command(binary, address_space_mib):
    """Apply the limit before exec, including to restarted validators.

    RLIMIT_AS limits virtual address space, not RSS or a cgroup memory budget.
    Keep binary hashing independent of the launcher.
    """
    if not address_space_mib:
        return [str(binary)]
    if not 128 <= address_space_mib <= 65536:
        raise ValueError('address-space limit must be 128..65536 MiB, or zero')
    launcher = shutil.which('prlimit')
    if not launcher:
        raise RuntimeError('prlimit is required to enforce the address-space limit')
    return [launcher, f'--as={address_space_mib * 1024 * 1024}', '--', str(binary)]


class Proxies:
    def __init__(self, count, base, proxy_base, delay, distinct=False, jitter=0, reset_percent=0, seed=17):
        self.count, self.base, self.proxy_base, self.delay = count, base, proxy_base, delay
        self.jitter, self.reset_percent = jitter, reset_percent
        # Fault scheduling only; this PRNG is never an election or consensus input.
        self.rng = random.Random(seed)
        self.reset_count = 0
        self.distinct = distinct
        self.partitioned = False
        self.forwarded = [0] * count
        self.observed_source_ips = set()
        self.connections = 0
        self.rejected = 0
        self.servers = []
        self.links = set()
        self.tasks = set()

    def group(self, index):
        return int(index >= self.count // 2)

    async def start(self):
        for group in (range(self.count) if self.distinct else (0, 1)):
            stride = int(self.count ** 0.5)
            destinations = {(group+d)%self.count for d in [1,-1,stride,-stride]} if self.distinct else range(self.count)
            for destination in destinations:
                server = await asyncio.start_server(
                    lambda r, w, g=group, d=destination: self.accept(r, w, g, d),
                    '127.0.0.1', self.proxy_base + 100 * group + destination, limit=65536)
                self.servers.append(server)

    async def accept(self, reader, writer, group, destination):
        task = asyncio.current_task()
        self.tasks.add(task)
        cross = (self.group(group) if self.distinct else group) != self.group(destination)
        target_writer = None
        entry = None
        try:
            if (self.partitioned and cross) or len(self.links) >= self.count * self.count * 2:
                self.rejected += 1
                return
            target_reader, target_writer = await asyncio.open_connection('127.0.0.1', self.base + destination, limit=65536, local_addr=(f'127.1.0.{group+1}',0) if self.distinct else None)
            self.observed_source_ips.add(target_writer.get_extra_info("sockname")[0])
            entry = (writer, target_writer, cross)
            self.links.add(entry)
            self.connections += 1

            async def pump(source, target):
                while data := await source.read(65536):
                    if self.partitioned and cross:
                        break
                    if self.reset_percent and self.rng.random()*100 < self.reset_percent:
                        self.reset_count += 1
                        break  # Close the entire encrypted TCP stream; never drop ciphertext bytes.
                    delay=max(0,self.delay+self.rng.uniform(-self.jitter,self.jitter))
                    if delay:
                        await asyncio.sleep(delay)
                    target.write(data)
                    await target.drain()
                    self.forwarded[destination] += len(data)

            pipes = [asyncio.create_task(pump(reader, target_writer)), asyncio.create_task(pump(target_reader, writer))]
            try:
                await asyncio.wait(pipes, return_when=asyncio.FIRST_COMPLETED)
            finally:
                for pipe in pipes:
                    pipe.cancel()
                await asyncio.gather(*pipes, return_exceptions=True)
        except (OSError, ConnectionError):
            pass
        finally:
            if entry:
                self.links.discard(entry)
            for stream in [writer, target_writer]:
                if stream:
                    stream.close()
            self.tasks.discard(task)

    def partition(self):
        self.partitioned = True
        for a, b, cross in list(self.links):
            if cross:
                a.close()
                b.close()

    async def close(self):
        for server in self.servers:
            server.close()
        await asyncio.gather(*(s.wait_closed() for s in self.servers))
        for task in list(self.tasks):
            task.cancel()
        await asyncio.gather(*list(self.tasks), return_exceptions=True)


async def run(args):
    root = args.directory.resolve()
    if root.exists() and any(root.iterdir()):
        entries = list(root.iterdir())
        empty_tmpfs_mount = ('disk-full' in args.faults and len(entries) == 1
                             and entries[0].name == 'node-0' and entries[0].is_dir()
                             and not any(entries[0].iterdir()))
        if not empty_tmpfs_mount:
            raise RuntimeError('case directory must be empty; existing stores are never overwritten')
    root.mkdir(parents=True, exist_ok=True)
    binary = args.binary.resolve()
    plan = args.committee_plan or f'1:{args.validators}'
    proxy = Proxies(args.validators, args.base_port, args.proxy_base_port, args.delay_ms / 1000, args.distinct_source_ips, args.jitter_ms/1000, args.stream_reset_percent, args.seed)
    if not args.direct:
        await proxy.start()
    processes, logs, offsets, states, observed = {}, [], {}, {}, {}
    peak = [0] * args.validators
    io_peaks = {}
    combined = 0
    io_observed_pids = set()
    samples, faults = [], []
    network_before = network_bytes()
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    own_before = resource.getrusage(resource.RUSAGE_SELF)
    started = time.monotonic()
    pending_faults = args.faults.split(',') if args.faults else []
    partition_time = stable_height = crash_time = None
    first_all_height = all_target_time = None
    filler = None
    reserved = {}
    minimum_anchors = {}
    completed_faults = set()
    fuzz_task = None
    shutdown_requested = False
    history_linger = 60 if args.validators >= 25 else (30 if 'disk-full' in pending_faults else 12)

    barrier_ms = int((time.time() + args.startup_barrier) * 1000)

    def launch(index):
        states.pop(index, None)
        log_path = root / f'node-{index}-{len(logs)}.log'
        log = open(log_path, 'w', encoding='utf-8')
        logs.append((log_path, log))
        processes[index] = subprocess.Popen(validator_command(binary, args.address_space_mib) + [
            'run', '--dir', str(root / f'node-{index}'), '--index', str(index),
            '--committee-plan', plan, '--base-port', str(args.base_port),
            '--peer-base-port', str(args.base_port if args.direct else args.proxy_base_port + 100 * (index if args.distinct_source_ips else proxy.group(index))),
            '--height', str(args.height), '--timeout', str(args.timeout), '--linger', str(history_linger),
            '--round-timeout-ms',str(args.round_timeout_ms), '--start-at-ms', str(barrier_ms), '--test-transactions', str(args.transactions), '--sparse-peers', str(args.sparse).lower()] + (['--signing-anchor', minimum_anchors[index]] if index in minimum_anchors else []), env=(dict(os.environ, YAMI_VALIDATOR_LOG='arc_malachitebft_network=debug,arc_malachitebft_engine=debug,arc_malachitebft_discovery=debug') if args.trace_one and index == 0 else None), stdout=log, stderr=subprocess.STDOUT)
        offsets[log_path] = (index, 0, '')

    def progress():
        for log_path, (index, offset, carry) in list(offsets.items()):
            with open(log_path, encoding='utf-8') as f:
                f.seek(offset)
                text = carry + f.read()
                offset = f.tell()
            lines = text.split('\n')
            offsets[log_path] = (index, offset, lines.pop())
            for line in lines:
                if line.startswith('{') and line.endswith('}'):
                    state = json.loads(line)
                    if state['payments_enabled'] or state['production_ready']:
                        raise RuntimeError('financial/production boundary changed')
                    pair = (state['block'], state['state_root'])
                    if observed.setdefault(state['height'], pair) != pair:
                        raise RuntimeError('conflicting finalized state')
                    states[index] = state

    async def socket_fuzz():
        semaphore = asyncio.Semaphore(8)
        payloads = [b'\x13/multistream/1.0.0\n', b'\x80' * 512,
                    b'\xff' * 65536, b'YC-MSG-1' + b'\x00' * 256]
        sent = 0
        async def attempt(index):
            nonlocal sent
            async with semaphore:
                writer = None
                try:
                    _, writer = await asyncio.wait_for(asyncio.open_connection('127.0.0.1', args.base_port + index % args.validators), 1)
                    payload = payloads[index % len(payloads)]
                    writer.write(payload)
                    await asyncio.wait_for(writer.drain(), 1)
                    sent += len(payload)
                except (OSError, asyncio.TimeoutError):
                    pass  # Early transport rejection is expected.
                finally:
                    if writer:
                        writer.close()
        await asyncio.gather(*(attempt(i) for i in range(64)))
        faults.append({'kind': 'malformed-TCP-probes-completed', 'attempts': 64, 'bytes_drained_to_sockets': sent})

    result = {'result': 'FAIL', 'scope': 'real Malachite processes, public development keys, loopback encrypted TCP; non-financial naming workload',
              'generated_at': datetime.now(timezone.utc).isoformat(), 'architecture': platform.machine(),
              'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'driver_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              'visible_host_logical_cpus': os.cpu_count(),
              'cpu_model': next((line.split(':', 1)[1].strip() for line in Path('/proc/cpuinfo').read_text().splitlines() if line.startswith('model name')), 'not reported'),
              'per_validator_address_space_limit_bytes': args.address_space_mib * 1024 * 1024 if args.address_space_mib else None,
              'memory_limit_scope': 'RLIMIT_AS virtual address space, not a per-node cgroup/RSS limit',
              'network_path': 'direct loopback' if args.direct else 'encrypted TCP fault proxy',
              'round_timeout_ms':args.round_timeout_ms, 'startup_barrier_seconds': args.startup_barrier, 'distinct_loopback_source_ips': args.distinct_source_ips, 'sparse_authenticated_neighbors': args.sparse, 'validators': args.validators, 'committee_plan': plan, 'target_height': args.height,
              'transactions_per_block': args.transactions, 'delay_ms_per_stream_read': args.delay_ms, 'jitter_ms_per_read':args.jitter_ms, 'stream_reset_percent_per_read':args.stream_reset_percent, 'fault_seed':args.seed, 'partition_seconds':args.partition_seconds,
              'cgroup': {name: limit(name) for name in ['cpu.max', 'memory.max', 'memory.swap.max']},
              'unverified_targets': ['20 GB SSD capacity and physical SSD performance', 'large/pruned state', 'native ARM64 unless architecture reports aarch64', 'shielded payment/proof workload']}
    try:
        for index in range(args.validators):
            launch(index)
        last_sample = -5
        while True:
            await asyncio.sleep(0.1)
            elapsed = time.monotonic() - started
            progress()
            if len(states) == args.validators:
                if first_all_height is None:
                    first_all_height = elapsed
                if all(s['height'] == args.height for s in states.values()) and all_target_time is None:
                    all_target_time = elapsed
            snapshots = [proc_sample(processes[i].pid) for i in range(args.validators)]
            combined = max(combined, sum(s['rss'] for s in snapshots))
            peak = [max(old, s['peak']) for old, s in zip(peak, snapshots)]
            for i, s in enumerate(snapshots):
                pid = processes[i].pid
                if s['io_available']:
                    io_observed_pids.add(pid)
                io_peaks[pid] = {k: max(io_peaks.get(pid, {}).get(k, 0), s[k]) for k in ['read_bytes', 'write_bytes']}
            if elapsed - last_sample >= 5:
                last_sample = elapsed
                store_samples=[usage(root/f'node-{i}') for i in range(args.validators)]
                samples.append({'seconds': round(elapsed, 3), 'height_min': min((s['height'] for s in states.values()), default=0),
                                'height_max': max((s['height'] for s in states.values()), default=0),
                                'sampled_combined_rss_bytes': sum(s['rss'] for s in snapshots),
                                'forwarded_transport_bytes': sum(proxy.forwarded),
                                'store_logical_bytes': sum(s['logical_bytes'] for s in store_samples),
                                'wal_bytes':sum(s['wal_bytes'] for s in store_samples),
                                'signing_bytes':sum(s['signing_bytes'] for s in store_samples)})
                # A host/container deadline must not erase a stalled campaign's
                # last measured progress. Only independent final replay sets PASS.
                result.update(in_progress=True, elapsed_seconds=round(elapsed, 3),
                              last_observed_heights={str(i): s['height'] for i, s in states.items()},
                              sampled_combined_peak_rss_bytes=combined,
                              sampled_per_validator_peak_rss_bytes=peak,
                              cgroup_memory_peak_bytes=limit('memory.peak'),
                              cgroup_cpu_stat=limit('cpu.stat'), samples=samples, faults=faults)
                args.report.parent.mkdir(parents=True, exist_ok=True)
                args.report.write_text(json.dumps(result, indent=2) + '\n')
            ready = len(states) == args.validators and min(s['height'] for s in states.values()) >= args.trigger_height
            if 'partition' in pending_faults and 'partition' not in completed_faults and ready:
                assert max(s['height'] for s in states.values()) < args.height, 'partition started after target; no halt claim is valid'
                proxy.partition()
                partition_time = elapsed
                completed_faults.add('partition')
                faults.append({'kind': 'partition', 'seconds': round(elapsed, 3)})
            if partition_time is not None:
                if elapsed - partition_time >= 2 and stable_height is None:
                    stable_height = max(s['height'] for s in states.values())
                if elapsed - partition_time >= args.partition_seconds:
                    assert max(s['height'] for s in states.values()) == stable_height, 'partition obtained a quorum or bypassed proxy'
                    proxy.partitioned = False
                    faults.append({'kind': 'partition-healed', 'seconds': round(elapsed, 3), 'no_quorum_height': stable_height})
                    partition_time = None
            can_crash = ready and partition_time is None and ('partition' not in pending_faults or 'partition' in completed_faults)
            for kind in ['crash', 'disk-full']:
                if kind in pending_faults and kind not in completed_faults and can_crash and crash_time is None:
                    if kind == 'disk-full':
                        store = root / 'node-0'
                        reserved = capture_safety(store, live=True)
                        if anchor := observed_anchor(store):
                            minimum_anchors[0] = anchor
                        filler = store / 'fault-filler.data'
                        # Keep enough room for the 64-MiB refusal reserve before
                        # injection; fill only this bounded tmpfs, never the host.
                        if os.statvfs(store).f_blocks * os.statvfs(store).f_frsize > 512 * 1024 * 1024:
                            raise RuntimeError('disk-full requires a dedicated tmpfs <=512 MiB')
                        with open(filler, 'wb', buffering=0) as f:
                            try:
                                while True:
                                    f.write(bytes(4096))
                            except OSError as error:
                                if error.errno != errno.ENOSPC:
                                    raise
                        faults.append({'kind': 'real-ENOSPC', 'seconds': round(elapsed, 3), 'retained_reservations': len(reserved)})
                    else:
                        processes[0].kill()
                        processes[0].wait()
                        faults.append({'kind': 'SIGKILL', 'seconds': round(elapsed, 3)})
                    completed_faults.add(kind)
                    crash_time = elapsed
            if 'socket-fuzz' in pending_faults and 'socket-fuzz' not in completed_faults and can_crash:
                fuzz_task = asyncio.create_task(socket_fuzz())
                completed_faults.add('socket-fuzz')
            if crash_time is not None and elapsed - crash_time >= (12 if filler else 2):
                if filler:
                    assert processes[0].poll() is not None and processes[0].returncode != 0, 'full-disk node did not fail closed before restart'
                    faults.append({'kind': 'ENOSPC-node-failed-closed', 'exit_code': processes[0].returncode,
                                   'last_finalized_height': states[0]['height']})
                if processes[0].poll() is None:
                    processes[0].kill()
                processes[0].wait()
                if filler:
                    filler.unlink()
                    assert preserved(root/'node-0', reserved), 'signing reservation changed under ENOSPC'
                    filler = None
                launch(0)
                faults.append({'kind': 'restart-preserving-entire-store', 'seconds': round(elapsed, 3)})
                crash_time = None
            if (not shutdown_requested and len(states) == args.validators
                    and all(s['height'] == args.height for s in states.values())
                    and completed_faults == set(pending_faults)
                    and (fuzz_task is None or fuzz_task.done())):
                # All targets were observed only after each node's authenticated
                # durable finalization. Graceful shutdown keeps history providers
                # alive for stragglers and never counts a partial height as success.
                for process in processes.values():
                    if process.poll() is None:
                        process.send_signal(signal.SIGINT)
                shutdown_requested = True
            if all(p.poll() is not None for p in processes.values()):
                break
            if elapsed > args.timeout + 30:
                raise RuntimeError('process campaign deadline')
        if fuzz_task:
            await fuzz_task
        if args.stream_reset_percent:
            assert proxy.reset_count > 0, 'requested TCP reset distribution produced no resets'
        assert completed_faults == set(pending_faults), 'requested faults were not reached'
        assert all(p.returncode == 0 for p in processes.values()), {i: p.returncode for i, p in processes.items()}
        after = resource.getrusage(resource.RUSAGE_CHILDREN)
        elapsed = time.monotonic() - started
        inspect_started = time.monotonic()
        replayed = [json.loads(subprocess.check_output([str(binary), 'inspect', '--dir', str(root / f'node-{i}'), '--committee-plan', plan], text=True)) for i in range(args.validators)]
        assert all(s['height'] == args.height and not s['payments_enabled'] for s in replayed)
        assert len({(s['block'], s['state_root']) for s in replayed}) == 1
        expected_names = args.height * args.transactions if args.transactions else 1
        assert all(s['test_tlds'] == expected_names for s in replayed)
        own = resource.getrusage(resource.RUSAGE_SELF)
        result.update(result='PASS', in_progress=False, elapsed_seconds_including_serving_and_shutdown=round(elapsed, 3),
                      maximum_history_linger_seconds=history_linger,
                      validator_cpu_seconds=round((after.ru_utime + after.ru_stime) - (before.ru_utime + before.ru_stime), 3),
                      proxy_and_monitor_cpu_seconds=round((own.ru_utime + own.ru_stime) - (own_before.ru_utime + own_before.ru_stime), 3),
                      sequential_authenticated_replay_seconds=round(time.monotonic() - inspect_started, 3),
                      final_height=args.height, total_test_transactions=args.height * args.transactions,
                      seconds_to_all_target=round(all_target_time, 3),
                      seconds_from_first_all_height_to_target=round(all_target_time - first_all_height, 3),
                      finalized_block=replayed[0]['block'], finalized_state=replayed[0]['state_root'])
    except Exception as error:
        result['failure'] = str(error)
        result['bounded_log_tails'] = {str(p.name): p.read_text(encoding='utf-8')[-2000:] for p, _ in logs[:8]}
        result['last_observed_heights'] = {str(i): s['height'] for i, s in states.items()}
        result['samples'] = samples
        result['faults'] = faults
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(result, indent=2) + '\n')
    finally:
        for p in processes.values():
            if p.poll() is None:
                p.kill()
            p.wait()
        await proxy.close()
        for _, log in logs:
            log.close()
        net_after = network_bytes()
        result.update(in_progress=False, elapsed_seconds=round(time.monotonic() - started, 3),
                      last_observed_heights={str(i): s['height'] for i, s in states.items()},
                      cgroup_memory_peak_bytes=limit('memory.peak'), cgroup_memory_events=limit('memory.events'), cgroup_cpu_stat=limit('cpu.stat'),
                      namespace_loopback_bytes_including_proxy_hops_and_TCP_overhead={k: net_after[k] - network_before[k] for k in net_after},
                      faults=faults, sampled_per_validator_peak_rss_bytes=peak, sampled_combined_peak_rss_bytes=combined,
                      per_process_block_io_counters_available=all(pid in io_observed_pids for pid in io_peaks),
                      sampled_processes=len(io_peaks), processes_with_observed_io_counters=len(io_observed_pids),
                      sampled_block_io_bytes={'read': sum(v['read_bytes'] for v in io_peaks.values()), 'write': sum(v['write_bytes'] for v in io_peaks.values())},
                      per_destination_forwarded_transport_bytes=proxy.forwarded, forwarded_transport_bytes=sum(proxy.forwarded),
                      stream_resets=proxy.reset_count, observed_proxy_source_ips=sorted(proxy.observed_source_ips), proxy_connections=proxy.connections, partition_rejected_connections=proxy.rejected,
                      stores=[usage(root / f'node-{i}') for i in range(args.validators)], samples=samples)
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps({k: v for k, v in result.items() if k not in ['bounded_log_tails', 'stores', 'samples', 'per_destination_forwarded_transport_bytes', 'sampled_per_validator_peak_rss_bytes']}, indent=2))
    return result['result'] == 'PASS'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/release/yami-validator'))
    parser.add_argument('--directory', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--validators', type=int, choices=[4, 25, 50, 100], default=4)
    parser.add_argument('--height', type=int, default=256)
    parser.add_argument('--transactions', type=int, default=16)
    parser.add_argument('--address-space-mib', type=int, default=0,
                        help='enforce per-validator RLIMIT_AS before exec; 0 disables (not an RSS limit)')
    parser.add_argument('--timeout', type=int, default=600)
    parser.add_argument('--partition-seconds', type=int, choices=range(7,301), default=7)
    parser.add_argument('--jitter-ms', type=float, default=0)
    parser.add_argument('--stream-reset-percent', type=float, default=0)
    parser.add_argument('--seed', type=int, default=17)
    parser.add_argument('--delay-ms', type=float, default=0)
    parser.add_argument('--faults', default='')
    parser.add_argument('--distinct-source-ips', action='store_true')
    parser.add_argument('--round-timeout-ms', type=int, default=800)
    parser.add_argument('--startup-barrier', type=int, choices=range(0,61), default=0)
    parser.add_argument('--trace-one', action='store_true')
    parser.add_argument('--sparse', action='store_true')
    parser.add_argument('--direct', action='store_true')
    parser.add_argument('--trigger-height', type=int, default=4)
    parser.add_argument('--committee-plan')
    parser.add_argument('--base-port', type=int, default=20000)
    parser.add_argument('--proxy-base-port', type=int, default=30000)
    args = parser.parse_args()
    try:
        validator_command(args.binary, args.address_space_mib)
    except (ValueError, RuntimeError) as error:
        parser.error(str(error))
    if not 0 <= args.transactions <= 64 or not 1 <= args.height <= 100000 or not 0 <= args.delay_ms <= 1000:
        parser.error('workload bounds exceeded')
    if not 0 <= args.jitter_ms <= 1000 or not 0 <= args.stream_reset_percent <= 10:
        parser.error('fault bounds exceeded')
    if args.direct and (args.jitter_ms or args.stream_reset_percent):
        parser.error('TCP proxy faults require proxy mode')
    if args.distinct_source_ips and (args.direct or not args.sparse):
        parser.error('distinct source IPs require the sparse proxy topology')
    if args.direct and ('partition' in args.faults or args.delay_ms):
        parser.error('direct mode cannot claim a proxy partition or delay')
    if set(filter(None, args.faults.split(','))) - {'partition', 'crash', 'disk-full', 'socket-fuzz'}:
        parser.error('unknown fault')
    raise SystemExit(0 if asyncio.run(run(args)) else 1)


if __name__ == '__main__':
    main()
