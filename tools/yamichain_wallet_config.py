"""Provision a least-privilege wallet profile from independently trusted inputs."""
import json
import os
import pathlib
import ssl
from yamichain_rpc import endpoint, secret


def export_wallet_config(network, policy, output, validate):
    # Validate everything before creating output; never probe or trust a server here.
    validate(policy)
    seeds = network.get('wallet_gateway_seeds', [network['gateway']])
    if not isinstance(seeds, list) or not 1 <= len(seeds) <= 8 or len(set(seeds)) != len(seeds):
        raise ValueError('need 1..8 unique trusted gateway origins')
    for seed in seeds:
        parsed = endpoint(seed)
        if parsed.scheme != 'https' or parsed.path not in ('', '/'):
            raise ValueError('exported wallet profiles require HTTPS gateway origins')
    ca = pathlib.Path(network['rpc']['ca_file']).read_bytes()
    if len(ca) > 1024 * 1024 or b'PRIVATE KEY' in ca:
        raise ValueError('CA must contain public certificates only, at most 1 MiB')
    ssl.create_default_context(cadata=ca.decode('ascii'))
    token = secret(network['rpc']['tokens']['gateway_file'])
    output = pathlib.Path(output).resolve()
    output.mkdir(mode=0o700, parents=True, exist_ok=False)

    def write(name, value):
        fd = os.open(output/name, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, 'wb') as stream:
            stream.write(value)
            stream.flush()
            os.fsync(stream.fileno())

    write('ca.pem', ca)
    write('gateway.token', token.encode('ascii') + b'\n')
    write('policy.json', json.dumps(policy).encode())
    profile = dict(gateway=seeds[0], wallet_gateway_seeds=seeds,
                   wallet_policy_file=str(output/'policy.json'),
                   rpc=dict(ca_file=str(output/'ca.pem'),
                            tokens=dict(gateway_file=str(output/'gateway.token'))))
    write('wallet-network.json', json.dumps(profile, indent=2).encode() + b'\n')
    fd = os.open(output, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)
    return output/'wallet-network.json'
