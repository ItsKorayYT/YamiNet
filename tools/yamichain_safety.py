"""Key-free witnesses for immutable safety bytes and append-only log prefixes."""
import hashlib
from pathlib import Path


def capture_safety(directory, live=False):
    directory = Path(directory)
    result = {}
    for path in directory.rglob('*'):
        if not path.is_file():
            continue
        name = path.name
        if not (name.startswith(('signed-', 'signing-', 'equivocation-')) or (not live and path.parent.name == 'wal')):
            continue
        if live and name in ('signing-tip.bin', 'signing-intent.bin'):
            continue  # observed_anchor pins these instead of treating them as immutable.
        data = path.read_bytes()
        result[str(path.relative_to(directory))] = dict(length=len(data), sha256=hashlib.sha256(data).hexdigest(),
            append=live and (name == 'signing-commits.bin' or name.startswith('signing-segment-')))
    return result


def preserved(directory, witnesses):
    for name, expected in witnesses.items():
        path = Path(directory)/name
        try:
            size = path.stat().st_size
            if size < expected['length'] or (not expected['append'] and size != expected['length']):
                return False
            with path.open('rb') as file:
                data = file.read(expected['length'])
            if hashlib.sha256(data).hexdigest() != expected['sha256']:
                return False
        except FileNotFoundError:
            return False
    return True


def digest(domain, data):
    return hashlib.sha256(len(domain).to_bytes(8, 'big')+domain+len(data).to_bytes(8, 'big')+data).digest()


def decode_commit(data):
    if len(data) != 88 or data[:4] != b'SC01' or digest(b'YC-SIGN-COMMIT-1', data[:56]) != data[56:]:
        raise ValueError('corrupt signing commit witness')
    return int.from_bytes(data[4:12], 'big'), data[24:56].hex()


def observed_anchor(directory):
    """Observe a committed tip OR an irrevocable intent, outside the node volume."""
    directory = Path(directory)
    candidates = []
    for name, limit in [('signing-tip.bin', 88), ('signing-intent.bin', 1024*1024)]:
        try:
            with (directory/name).open('rb') as file:
                data = file.read(limit+1)
        except FileNotFoundError:
            continue  # Intent publication/cleanup may race a live sampler.
        if len(data) > limit:
            raise ValueError('signing witness cap')
        if name == 'signing-tip.bin':
            candidates.append(decode_commit(data))
        else:
            if len(data) < 189 or data[:4] != b'SI01' or digest(b'YC-SIGN-INTENT-1', data[:-32]) != data[-32:]:
                raise ValueError('corrupt signing intent witness')
            count, root = decode_commit(data[44:132])
            frame = data[136:-32]
            if (count != int.from_bytes(data[4:12], 'big')+1 or len(frame) != int.from_bytes(data[132:136], 'big')
                    or digest(b'YC-SIGN-CHAIN-1', data[12:44]+frame).hex() != root):
                raise ValueError('signing intent binding')
            candidates.append((count, root))
    if not candidates:
        return None
    count, root = max(candidates)
    return f'{count}:{root}'
