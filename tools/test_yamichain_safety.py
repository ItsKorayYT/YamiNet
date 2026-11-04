import tempfile
from pathlib import Path
import struct
import unittest
from yamichain_safety import capture_safety, preserved, observed_anchor, digest


def commit(count, end, root):
    data = b'SC01'+struct.pack('>QIQ', count, 0, end)+root
    return data+digest(b'YC-SIGN-COMMIT-1', data)


class SafetyWitnessTests(unittest.TestCase):
    def test_live_append_prefixes_preserve_old_authorizations_but_not_rewrites(self):
        with tempfile.TemporaryDirectory() as directory:
            p = Path(directory)
            (p/'signing-segment-00000000.bin').write_bytes(b'authorization-one')
            (p/'signing-commits.bin').write_bytes(b'commit-one')
            (p/'signed-legacy.bin').write_bytes(b'legacy')
            witnesses = capture_safety(p, live=True)
            for name in ['signing-segment-00000000.bin', 'signing-commits.bin']:
                with (p/name).open('ab') as file:
                    file.write(b'next')
            self.assertTrue(preserved(p, witnesses))
            (p/'signed-legacy.bin').write_bytes(b'legacy-extra')
            self.assertFalse(preserved(p, witnesses))
            (p/'signed-legacy.bin').write_bytes(b'legacy')
            (p/'signing-segment-00000000.bin').write_bytes(b'changed')
            self.assertFalse(preserved(p, witnesses))

    def test_offline_wal_and_tip_must_stay_byte_exact(self):
        with tempfile.TemporaryDirectory() as directory:
            p = Path(directory)
            (p/'wal').mkdir()
            (p/'wal/consensus.wal').write_bytes(b'locks')
            (p/'signing-tip.bin').write_bytes(b'tip')
            witnesses = capture_safety(p)
            (p/'wal/consensus.wal').write_bytes(b'other')
            self.assertFalse(preserved(p, witnesses))

    def test_pending_authorization_is_pinned_even_before_tip_publication(self):
        with tempfile.TemporaryDirectory() as directory:
            p = Path(directory)
            frame1 = b'SR01'+struct.pack('>QIBI', 1, 0, 2, 3)+b'one'
            frame2 = b'SR01'+struct.pack('>QIBI', 2, 0, 2, 3)+b'two'
            root1 = digest(b'YC-SIGN-CHAIN-1', bytes(32)+frame1)
            root2 = digest(b'YC-SIGN-CHAIN-1', root1+frame2)
            (p/'signing-tip.bin').write_bytes(commit(1, len(frame1), root1))
            intent = b'SI01'+struct.pack('>Q', 1)+root1+commit(2, len(frame1+frame2), root2)+struct.pack('>I', len(frame2))+frame2
            intent += digest(b'YC-SIGN-INTENT-1', intent)
            (p/'signing-intent.bin').write_bytes(intent)
            self.assertEqual(observed_anchor(p), f'2:{root2.hex()}')
            (p/'signing-intent.bin').write_bytes(intent[:-1]+b'!')
            with self.assertRaises(ValueError):
                observed_anchor(p)
