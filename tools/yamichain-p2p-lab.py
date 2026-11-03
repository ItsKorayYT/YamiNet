#!/usr/bin/env python3
"""Real validators in separate, private network namespaces; no public deployment.

Requires a local Docker daemon. Every container has a two CPU / 2 GiB limit.
Only public roster/hints and the node's own private identity enter its namespace.
Retains stores and measurements; removes only containers/network created here.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

IMAGE = 'python:3.13-slim-trixie@sha256:ae14d4ce8771606477a81446e7ef429edd6762305fc50c84507c1041fa8285d1'


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--binary', type=Path, default=Path('target/release/yami-validator'))
    p.add_argument('--height', type=int, default=96)
    p.add_argument('--quorum-loss', action='store_true')
    p.add_argument('--transactions', type=int, choices=range(65), default=8)
    args = p.parse_args()
    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    binary = args.binary.resolve()
    env = os.environ.copy()
    for key in ['DOCKER_HOST', 'DOCKER_CONTEXT', 'DOCKER_TLS', 'DOCKER_TLS_VERIFY', 'DOCKER_CERT_PATH']:
        env.pop(key, None)
    cli = ['docker', '--host=unix:///var/run/docker.sock']

    def docker(*cmd, check=True):
        return subprocess.run(cli + list(cmd), env=env, capture_output=True, text=True, check=check).stdout.strip()

    name = f'yami-p2p-{os.getpid()}'
    names = [f'{name}-{i}' for i in range(4)]
    attacker = f'{name}-attacker'
    samples, events = [], []
    report = dict(binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), profile='isolated-private-ipv4', cpus_per_node=2, memory_bytes_per_node=2147483648,
                  height=args.height, test_transactions_per_block=args.transactions, quorum_loss=args.quorum_loss, samples=samples, events=events, passed=False)
    started = time.monotonic()
    endpoints = ','.join(f'10.81.0.{i+2}:28100' for i in range(4))
    subprocess.run([str(binary), 'init-isolated', '--dir', str(root/'provision'), '--endpoints', endpoints], check=True)
    # Provisioning keys stay host-local. Each validator sees exactly one identity.
    for i in range(4):
        config = root/f'config-{i}'
        config.mkdir(mode=0o700)
        for file in ['roster.bin', 'bootstrap-0.bin', 'bootstrap-1.bin', f'identity-{i}.bin']:
            shutil.copy2(root/'provision'/file, config/file)
        # Each node starts with self and only two authenticated bootstrap hints.
        # The missing validator endpoint must be learned over signed gossip.
        for source in range(2):
            path = config/f'bootstrap-{source}.bin'
            full = path.read_bytes()
            chosen = [i, (i+1) % 4, (i+2) % 4]
            path.write_bytes(bytes([3])+b''.join(full[1+j*110:1+(j+1)*110] for j in chosen))
        (root/f'store-{i}').mkdir()
    def heights():
        return [max([int(f.stem.split('-')[1]) for f in (root/f'store-{i}').glob('finalized-*.bin')], default=0) for i in range(4)]

    partition_heights = None
    network_created = False
    try:
        docker('network', 'create', '--internal', '--subnet', '10.81.0.0/24', name)
        network_created = True
        report['docker_network_internal'] = json.loads(docker('network', 'inspect', '--format', '{{json .Internal}}', name))
        if not report['docker_network_internal']:
            raise RuntimeError('lab network must be internal-only')
        for i, node in enumerate(names):
            docker('run', '-d', '--name', node, '--network', name, '--ip', f'10.81.0.{i+2}',
                   '--cpus', '2', '--memory', '2g', '--memory-swap', '2g', '--pids-limit', '256',
                   '--ulimit', 'nofile=512:512', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges',
                   '--read-only', '--user', f'{os.getuid()}:{os.getgid()}',
                   '--mount', f'type=bind,src={binary},dst=/validator,readonly',
                   '--mount', f'type=bind,src={root / f"config-{i}"},dst=/config,readonly',
                   '--mount', f'type=bind,src={root / f"store-{i}"},dst=/store',
                   '--env', 'YAMI_VALIDATOR_LOG=warn,malachitebft_network=debug,malachitebft_sync=debug',
                   IMAGE, '/validator', 'run', '--dir', '/store', '--index', str(i),
                   '--isolated-network', '/config', '--height', str(args.height), '--timeout', '180',
                   '--linger', '60', '--test-transactions', str(args.transactions))
        report['enforced_limits'] = []
        for node in names:
            host = json.loads(docker('inspect', '--format', '{{json .HostConfig}}', node))
            limits = {key: host[key] for key in ['NanoCpus', 'Memory', 'MemorySwap', 'PidsLimit', 'ReadonlyRootfs', 'CapDrop', 'Ulimits']}
            report['enforced_limits'].append(limits)
            if limits['NanoCpus'] != 2000000000 or limits['Memory'] != 2147483648:
                raise RuntimeError('validator resource limits were not enforced')
        events.append(dict(at=time.monotonic()-started, event='four_independent_nodes_started'))
        attack = """import socket,time,json
sockets=[]; opened=0
u=socket.socket(socket.AF_INET,socket.SOCK_DGRAM)
for n in range(2048):u.sendto(b'\\xff'*1762,('10.81.0.2',28101))
u.close()
for n in range(256):
 try:
  s=socket.create_connection(('10.81.0.2',28100),timeout=.05)
  if n%2: s.sendall(b'\\xff'*1024)
  sockets.append(s); opened+=1
 except OSError: pass
time.sleep(4)
alive=0
for s in sockets:
 try:
  s.settimeout(.05)
  if s.recv(1): alive+=1
 except socket.timeout: alive+=1
 except OSError: pass
for s in sockets:s.close()
print(json.dumps(dict(attempted=256,tcp_opened=opened,still_open_after_4_seconds=alive)))
"""
        docker('run', '-d', '--name', attacker, '--network', name, '--ip', '10.81.0.200',
               '--cpus', '1', '--memory', '128m', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges',
               IMAGE, 'python', '-c', attack)
        for tick in range(160):
            elapsed = time.monotonic()-started
            raw = docker('stats', '--no-stream', '--format', '{{json .}}', *names, check=False)
            sample = dict(at=elapsed, nodes=[json.loads(line) for line in raw.splitlines() if line.startswith('{')])
            if tick % 3 == 0:
                telemetry = []
                for node in names:
                    raw_process = docker('exec', node, 'python', '-c',
                        "import os,json; s=open('/proc/1/status').read(); t=open('/proc/1/stat').read().split(); print(json.dumps(dict(cpu_ticks=int(t[13])+int(t[14]),clock_ticks=os.sysconf('SC_CLK_TCK'),fds=len(os.listdir('/proc/1/fd')),memory=[x for x in s.splitlines() if x.startswith(('VmRSS:','VmHWM:'))],tcp_sockets=len(open('/proc/net/tcp').readlines())-1)))", check=False)
                    if raw_process.startswith('{'):
                        telemetry.append(dict(node=node, **json.loads(raw_process)))
                sample['processes'] = telemetry
            sample['heights'] = heights()
            sample['heights_at'] = time.monotonic()-started
            samples.append(sample)
            if tick == 1:
                partition_heights = heights()
                docker('network', 'disconnect', name, names[3])
                if args.quorum_loss:
                    docker('network', 'disconnect', name, names[2])
                events.append(dict(at=time.monotonic()-started, event='partition_node_3'))
            if tick == 2:
                # Malformed discovery from a known peer IP must reach signature
                # checks but must not replace any pinned hint or exhaust the loop.
                inject = "import socket; b=bytearray(open('/config/bootstrap-1.bin','rb').read()); b[-1]^=1; s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM); [s.sendto(b,('10.81.0.3',28101)) for _ in range(512)]"
                docker('exec', names[0], 'python', '-c', inject)
                events.append(dict(at=time.monotonic()-started, event='512_forged_discovery_pages_from_known_ip'))
            if tick == 3 and args.quorum_loss:
                stalled = heights()
                if max(stalled) > max(partition_heights)+1:
                    raise RuntimeError('quorum-loss partition advanced beyond in-flight finality')
                report['quorum_loss_stall'] = dict(before=partition_heights, after=stalled)
            if tick == 4:
                docker('kill', '--signal', 'KILL', names[3])
                subprocess.run([str(binary), 'announce-isolated', '--dir', str(root/'config-3'),
                                '--index', '3', '--endpoint', '10.81.0.6:28100'], check=True)
                for i in range(1):
                    target = root/f'config-{i}'/'bootstrap-0.bin'
                    temporary = target.with_suffix('.next')
                    shutil.copy2(root/'config-3'/'bootstrap-0.bin', temporary)
                    temporary.replace(target)
                docker('network', 'connect', '--ip', '10.81.0.6', name, names[3])
                if args.quorum_loss:
                    docker('network', 'connect', '--ip', '10.81.0.4', name, names[2])
                docker('start', names[3])
                events.append(dict(at=time.monotonic()-started, event='discover_new_endpoint_and_restart_node_3_with_original_wal'))
            states = [json.loads(docker('inspect', '--format', '{{json .State}}', node)) for node in names]
            if all(not state['Running'] for state in states):
                report['exit_codes'] = [s['ExitCode'] for s in states]
                break
            if time.monotonic()-started > 210:
                raise RuntimeError('campaign exceeded deadline')
            time.sleep(.5)
        report['attack'] = json.loads(docker('logs', attacker))
        if report['attack']['still_open_after_4_seconds'] > 4:
            raise RuntimeError('per-IP pending connection bound exceeded')
        statuses = []
        for i, node in enumerate(names):
            logs = docker('logs', node, check=False)
            (root/f'node-{i}.log').write_text(logs)
            lines = [json.loads(line) for line in logs.splitlines() if line.startswith('{')]
            if not lines:
                raise RuntimeError(f'node {i} produced no finalized status')
            statuses.append(lines[-1])
        report['statuses'] = statuses
        discovery = []
        for i in range(4):
            data = (root/f'store-{i}'/'network-hints.bin').read_bytes()
            addresses = [list(data[33+j*110+32:33+j*110+36]) for j in range(data[32])]
            discovery.append(dict(count=data[32], new_endpoint_present=[10,81,0,6] in addresses))
        report['discovery'] = discovery
        if any(d['count'] != 4 or not d['new_endpoint_present'] for d in discovery):
            raise RuntimeError('signed gossip failed to discover all pinned endpoints or propagate replacement')
        healed = next(e['at'] for e in events if e['event'].startswith('discover_new'))
        equal = [s['heights_at'] for s in samples if s['heights_at'] > healed and len(set(s['heights'])) == 1]
        report['recovery_observed_after_seconds'] = min(equal)-healed if equal else None
        inspected = []
        for i in range(4):
            result = subprocess.run([str(binary), 'inspect', '--dir', str(root/f'store-{i}'),
                                     '--isolated-network', str(root/f'config-{i}')], capture_output=True, text=True, check=True)
            inspected.append(json.loads(result.stdout))
        report['authenticated_disk_replay'] = inspected
        if inspected != statuses:
            raise RuntimeError('durable authenticated replay differs from process finality')
        report['passed'] = (report.get('exit_codes') == [0]*4 and all(s['height'] == args.height for s in statuses)
                            and len({s['block'] for s in statuses}) == 1 and len({s['state_root'] for s in statuses}) == 1)
        if not report['passed']:
            raise RuntimeError('finalized states did not converge')
    finally:
        report['elapsed_seconds'] = time.monotonic()-started
        (root/'report.json').write_text(json.dumps(report, indent=2)+'\n')
        for node in names+[attacker]:
            # Preserve stderr diagnostics as well as final status before removing owned containers.
            result = subprocess.run(cli+['logs', node], env=env, capture_output=True, text=True)
            (root/f'{node}.log').write_text(result.stdout+result.stderr)
            docker('rm', '-f', node, check=False)
        if network_created:
            docker('network', 'rm', name, check=False)
    print(json.dumps(dict(passed=report['passed'], seconds=report['elapsed_seconds'], report=str(root/'report.json'))))


if __name__ == '__main__':
    main()
