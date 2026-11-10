"""Bounded HTTPS seed discovery. Endpoints supply availability, never trust policy."""
from yamichain_rpc import endpoint


def discover(transport, seeds, pinned, validate):
    if not isinstance(seeds, list) or not 1 <= len(seeds) <= 8 or len(set(seeds)) != len(seeds):
        raise ValueError('wallet discovery needs 1..8 unique configured gateway seeds')
    for seed in seeds:
        url = endpoint(seed)
        if url.path not in ('', '/'):
            raise ValueError('gateway seeds must be service origins')
        if len(seeds) > 1 and url.scheme != 'https':
            raise ValueError('multiple gateway discovery seeds require HTTPS')
    if len(seeds) > 1 and pinned is None:
        raise ValueError('gateway discovery requires an independently provisioned policy pin')
    if pinned is not None:
        validate(pinned)
    for seed in seeds:
        try:
            remote = transport.request(seed.rstrip('/') + '/policy', timeout=3)
            validate(remote)
            if pinned is not None and remote != pinned:
                continue
            return seed.rstrip('/'), remote
        except Exception:
            continue
    raise RuntimeError('no configured gateway seed supplied the pinned network policy')
