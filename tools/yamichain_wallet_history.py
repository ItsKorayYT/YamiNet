"""Wallet-local encrypted outgoing submissions; finality always comes from Rust verification."""
import contextlib
import fcntl
import hashlib
import json
import os
import pathlib
import stat
import tempfile


def private_fd(path, flags):
    fd = os.open(path, flags | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600)
    info = os.fstat(fd)
    if not stat.S_ISREG(info.st_mode) or info.st_mode & 0o077:
        os.close(fd)
        raise ValueError('wallet history must be a private regular file (mode 0600)')
    return fd


class History:
    def __init__(self, wallet, policy, bridge, secret):
        network = hashlib.sha256(json.dumps(policy, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
        self.path = pathlib.Path(str(pathlib.Path(wallet).resolve()) + '.history-' + network + '.json')
        self.policy, self.bridge, self.secret = policy, bridge, secret
        self.entries = []

    @contextlib.contextmanager
    def locked(self):
        fd = private_fd(str(self.path) + '.lock', os.O_WRONLY | os.O_CREAT)
        try:
            fcntl.flock(fd, fcntl.LOCK_EX)
            try:
                source = private_fd(self.path, os.O_RDONLY)
            except FileNotFoundError:
                self.entries = []
            else:
                with os.fdopen(source, 'rb') as stream:
                    raw = stream.read(16 * 1024 * 1024 + 1)
                if len(raw) > 16 * 1024 * 1024:
                    raise ValueError('wallet history byte limit')
                self.entries = self.bridge('wallet-history-open', secret=self.secret,
                                           policy=self.policy, envelope=json.loads(raw))
                if not isinstance(self.entries, list) or len(self.entries) > 1024:
                    raise ValueError('invalid wallet history')
            yield self
        finally:
            os.close(fd)

    def append(self, entry):
        if len(self.entries) >= 1024:
            raise ValueError('wallet history is full; archive the encrypted history before submitting')
        entries = self.entries + [entry]
        sealed = self.bridge('wallet-history-seal', secret=self.secret, policy=self.policy, entries=entries)
        fd, temporary = tempfile.mkstemp(prefix=self.path.name + '.', dir=self.path.parent)
        try:
            with os.fdopen(fd, 'w') as stream:
                json.dump(sealed, stream)
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(temporary, self.path)
            directory = os.open(self.path.parent, os.O_RDONLY | os.O_DIRECTORY)
            try:
                os.fsync(directory)
                self.entries = entries
            finally:
                os.close(directory)
        finally:
            if os.path.exists(temporary):
                os.unlink(temporary)
