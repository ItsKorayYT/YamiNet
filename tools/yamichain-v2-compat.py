#!/usr/bin/env python3
"""Offline previous/current V1 archive/pruned readers and inactive V2 boundaries."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def hashes(path):
    return {str(p.relative_to(path)): sha(p) for p in sorted(path.rglob('*')) if p.is_file()}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--previous', type=Path, required=True)
    p.add_argument('--current', type=Path, required=True)
    p.add_argument('--reader', type=Path, required=True)
    p.add_argument('--history', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    previous, current, reader = [p.resolve() for p in (a.previous, a.current, a.reader)]
    history = a.history.resolve()
    evidence = json.loads((history / 'report.json').read_text())
    assert evidence['passed'] and not evidence['live_engine_generated']
    cases = {c['entries']: c for c in evidence['cases']}
    small = cases[1024]
    root = a.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    report = dict(passed=False, previous_ref='f3fb721', previous_sha256=sha(previous),
                  current_sha256=sha(current), reader_sha256=sha(reader), cases=[])
    assert report['previous_sha256'] != report['current_sha256']

    def call(binary, command, directory, case=small, extra=(), success=True):
        plan = '1:4,' + str(case['future_epoch']) + ':5'
        result = subprocess.run([str(binary), command, '--dir', str(directory), '--committee-plan', plan, *map(str, extra)],
                                capture_output=True, text=True, timeout=300)
        assert (result.returncode == 0) == success, result.stderr + result.stdout
        return json.loads(result.stdout) if success else result.stderr.strip()

    def state_matches(state, case=small):
        assert (state['height'], state['block'], state['state_root'], state['test_tlds']) == (
            case['height'], case['tip'], case['root'], case['entries'])

    source = history / 'state-1024' / 'archive'
    before = hashes(source)
    try:
        # No identity rebinding: inspect/export are observer operations. Never
        # run a signer on the offline fixture archive or copy its safety stores.
        for binary in (previous, current):
            state_matches(call(binary, 'inspect', source))
        snapshots = []
        for name, binary in [('previous', previous), ('current', current)]:
            snapshot = root / (name + '-v1.bin')
            exported = call(binary, 'export-checkpoint', source, extra=['--out', snapshot])
            assert exported['anchor'] == f"{small['height']}:{small['tip']}:{small['root']}"
            snapshots.append(snapshot)
        assert snapshots[0].read_bytes() == snapshots[1].read_bytes()
        anchor = f"{small['height']}:{small['tip']}:{small['root']}"
        for name, importer, inspector, snapshot in [
                ('old-to-new', current, previous, snapshots[0]),
                ('new-to-old', previous, current, snapshots[1])]:
            destination = root / name
            state_matches(call(importer, 'import-checkpoint', destination,
                               extra=['--snapshot', snapshot, '--anchor', anchor]))
            state_matches(call(inspector, 'inspect', destination))
            assert (destination / 'compact-base.bin').exists()
            assert not (destination / 'wal').exists()
            report['cases'].append(dict(mode=name, v1_pruned_recovery=True, passed=True))
        generation = history / 'state-1024' / 'inactive-v2'
        original = hashes(generation)
        for name, binary in [('previous', previous), ('current', current)]:
            error = call(binary, 'export-checkpoint', generation, extra=['--out', root / (name + '-inactive-must-not-export.bin')], success=False)
            assert 'experimental snapshot directories cannot be production journals' in error
            assert hashes(generation) == original and not (generation / 'store.lock').exists()
            report['cases'].append(dict(mode=name + '-refuses-inactive-directory', passed=True, error=error))
            destination = root / (name + '-inactive-import')
            error = call(binary, 'import-checkpoint', destination,
                         extra=['--snapshot', history / 'state-1024' / 'inactive-envelope.bin', '--anchor', anchor], success=False)
            assert not (destination / 'compact-base.bin').exists() and not (destination / 'compacted.bin').exists()
            report['cases'].append(dict(mode=name + '-refuses-inactive-envelope', passed=True, error=error))
        # A real old V1 writer still refuses the 250k recovery boundary without
        # deleting history, installing a base or switching storage modes.
        large = history / 'state-250000' / 'archive'
        large_before = hashes(large)
        failed_export = root / 'large-v1-must-not-exist.bin'
        error = call(previous, 'export-checkpoint', large, case=cases[250000],
                     extra=['--out', failed_export], success=False)
        assert not failed_export.exists() and hashes(large) == large_before
        report['cases'].append(dict(mode='previous-v1-250k-refusal-history-preserved', passed=True, error=error))
        # Valid new envelope plus lab files cannot be silently translated into
        # inactive state; rejection must leave the copied application files intact.
        lab = root / 'lab-is-not-inactive'
        lab.mkdir()
        shutil.copy2(history / 'state-1024' / 'inactive-envelope.bin', lab / 'inactive-envelope.bin')
        shutil.copytree(history / 'state-1024' / 'v2', lab / 'inactive-v2')
        lab_before = hashes(lab)
        result = subprocess.run([str(reader), str(lab), '1024', str(small['height']), small['tip'], small['root']],
                                capture_output=True, text=True, timeout=30)
        assert result.returncode != 0 and hashes(lab) == lab_before
        report['cases'].append(dict(mode='inactive-reader-refuses-real-lab-store', passed=True, error=result.stderr.strip()))
        assert hashes(source) == before
        report['source_archive_bytes_unchanged'] = True
        report['passed'] = True
    finally:
        (root / 'report.json').write_text(json.dumps(report, indent=2) + '\n')

if __name__ == '__main__':
    main()
