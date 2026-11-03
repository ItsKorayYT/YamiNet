#!/usr/bin/env python3
"""Independent keyless observer. Reads snapshots ONLY, never labels or keys."""
import json
import pathlib
import sys

ORDER = 2**252 + 27742317777372353535851937790883648493


def ciphertexts(pair):
    if len(pair) != 2 or len(pair[0]["cells"]) != len(pair[1]["cells"]):
        raise ValueError("snapshot dimensions")
    return [tuple((int.from_bytes(bytes(a), "little") +
                   int.from_bytes(bytes(b), "little")) % ORDER
                  for a, b in zip(left, right))
            for left, right in zip(pair[0]["cells"], pair[1]["cells"])]


def observe(path):
    snapshots = json.loads(pathlib.Path(path).read_text())
    positions = []
    for before, after in zip(snapshots, snapshots[1:]):
        old, new = ciphertexts(before), ciphertexts(after)
        if len(old) != len(new):
            raise ValueError("snapshot dimensions changed")
        positions.append([i for i, (a, b) in enumerate(zip(old, new)) if a != b])
    return {"positions_from_downloadable_ciphertexts": positions,
            "same_slot_pairs": [[i, j] for i in range(len(positions))
                                for j in range(i + 1, len(positions))
                                if positions[i] == positions[j]],
            "wallet_keys_or_target_labels_used": False}


if __name__ == "__main__":
    print(json.dumps(observe(sys.argv[1]), indent=2))
