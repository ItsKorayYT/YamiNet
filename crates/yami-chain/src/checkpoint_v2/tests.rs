use super::*;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static ID: AtomicU64 = AtomicU64::new(0);
fn dir() -> PathBuf {
    std::env::temp_dir().join(format!(
        "yc-inactive-v2-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ))
}
fn fixture(n: usize) -> InactiveState {
    let mut state = State::new(Genesis::local()).unwrap();
    state.height = (n as u64).div_ceil(64).max(1);
    state.tip = hash(b"lab-fixture-tip", &(n as u64).to_be_bytes());
    let keys = [
        development_key("registry", 0).verifying_key().to_bytes(),
        development_key("registry", 1).verifying_key().to_bytes(),
    ];
    for i in 0..n {
        state
            .names
            .register(&format!("test-{i:058}"), keys[i % 2])
            .unwrap();
        let nonce = state.nonce(&keys[i % 2]);
        state.nonces.insert(keys[i % 2], nonce + 1);
    }
    InactiveState::from_verified_state(&state).unwrap()
}
fn chunks(state: &InactiveState) -> (Manifest, Vec<Vec<u8>>) {
    let mut chunks = Vec::new();
    let m = export(state, |_, b| {
        chunks.push(b.to_vec());
        Ok(())
    })
    .unwrap();
    (m, chunks)
}
fn decode(m: &Manifest, c: &[Vec<u8>]) -> io::Result<InactiveState> {
    import(Genesis::local(), m.anchor(), m, |i| {
        Ok(std::io::Cursor::new(&c[i as usize]))
    })
}
fn rehash(m: &mut Manifest, c: &[Vec<u8>]) {
    for (i, entry) in m.chunks.iter_mut().enumerate() {
        entry.digest = entry.digest(i as u32, &c[i]);
    }
}
fn hashes(p: &Path) -> BTreeMap<String, Digest> {
    fs::read_dir(p)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (
                e.file_name().into_string().unwrap(),
                hash(b"files", &fs::read(e.path()).unwrap()),
            )
        })
        .collect()
}
#[test]
fn initial_roundtrip() {
    let state = State::new(Genesis::local()).unwrap();
    let state = state
        .execute(&state.development_workload_block(64).unwrap())
        .unwrap();
    let lab = InactiveState::from_verified_state(&state).unwrap();
    let (m, c) = chunks(&lab);
    let restored = decode(&m, &c).unwrap();
    assert_eq!(restored.anchor(), lab.anchor());
    assert_eq!(
        state.snapshot_bytes().unwrap(),
        restored.v1_snapshot().unwrap()
    );
    assert_eq!(state.root(), restored.uncached_root());
    assert!(State::restore_checkpoint(
        Genesis::local(),
        1,
        state.tip(),
        state.root(),
        &m.canonical()
    )
    .is_err());
}
#[test]
fn malformed_missing_reordered_duplicate_and_forged_chunks_fail_closed() {
    let (m, c) = chunks(&fixture(6000));
    assert!(
        Manifest::decode_transport(&vec![0; MAX_MANIFEST_BYTES + 1], m.anchor().state).is_err()
    );
    assert!(m.chunks.len() > 3);
    for case in 0..12 {
        let mut m = m.clone();
        let mut c = c.clone();
        match case {
            0 => {
                c[1][10] ^= 1;
            }
            1 => {
                c.swap(1, 2);
            }
            2 => {
                let key = c[0][..32].to_vec();
                c[0][40..72].copy_from_slice(&key);
                rehash(&mut m, &c);
            }
            3 => {
                c[0][..32].fill(0);
                rehash(&mut m, &c);
            }
            4 => {
                c[1][..4].copy_from_slice(&u32::MAX.to_be_bytes());
                rehash(&mut m, &c);
            }
            5 => {
                let end = (m.chunks[1].records as usize - 1) * 99 + 4;
                let last = c[1][end..end + 63].to_vec();
                c[2][4..67].copy_from_slice(&last);
                rehash(&mut m, &c);
            }
            6 => {
                c[1].clear();
            }
            7 => {
                c[1].push(0);
            }
            8 => {
                m.chunks[0].bytes = u32::MAX;
            }
            9 => {
                m.chunks[1].first = u32::MAX;
            }
            10 => {
                c[0][32..40].copy_from_slice(&u64::MAX.to_be_bytes());
                rehash(&mut m, &c);
            }
            _ => {
                m.state.root.0[0] ^= 1;
            }
        }
        assert!(decode(&m, &c).is_err(), "case {case}");
    }
    // Even a self-consistent, freshly checksummed transport manifest cannot override
    // an independently retained state/manifest commitment.
    let mut forged = m.clone();
    forged.state.height += 1;
    assert!(Manifest::decode(&forged.canonical(), m.anchor()).is_err());
    let mut over = m.clone();
    over.accounts = MAX_ENTRIES + 1;
    assert!(Manifest::decode(&over.canonical(), over.anchor()).is_err());
    over = m.clone();
    over.total = MAX_PAYLOAD_BYTES + 1;
    assert!(Manifest::decode(&over.canonical(), over.anchor()).is_err());
    over = m.clone();
    over.chunks = vec![m.chunks[0].clone(); MAX_CHUNKS + 1];
    assert!(Manifest::decode(&over.canonical(), over.anchor()).is_err());
    for len in 0..m.canonical().len() {
        assert!(Manifest::decode(&m.canonical()[..len], m.anchor()).is_err());
    }
}
#[test]
fn immutable_stores_pin_state_rollback_and_protect_safety_namespaces() {
    let p = dir();
    let source = p.join("source");
    let target = p.join("target");
    let a = fixture(128);
    let expected = store::save(&a, &source).unwrap();
    let before = hashes(&source);
    let copy = store::transfer(&source, &target, Genesis::local(), expected).unwrap();
    assert_eq!(copy.anchor(), a.anchor());
    assert_eq!(hashes(&source), before);
    assert_eq!(hashes(&source), hashes(&target));
    assert!(store::save(&fixture(129), &source).is_err());
    assert_eq!(hashes(&source), before);
    let newer = store::save(&fixture(129), &p.join("newer")).unwrap();
    assert!(store::recover(&source, Genesis::local(), newer).is_err());
    assert!(store::transfer(&p.join("newer"), &target, Genesis::local(), newer).is_err());
    assert_eq!(hashes(&target), before);
    let safety = p.join("validator");
    fs::create_dir_all(&safety).unwrap();
    for name in [
        "genesis.bin",
        "signing-tip.bin",
        "signed-1.bin",
        "equivocation-1.bin",
    ] {
        fs::write(safety.join(name), name).unwrap();
    }
    let original = hashes(&safety);
    assert!(store::save(&a, &safety).is_err());
    assert_eq!(hashes(&safety), original);
    fs::remove_dir_all(p).unwrap();
}
#[test]
fn child() {
    let Some(path) = std::env::var_os("YAMI_V2_CHILD") else {
        return;
    };
    let mode = std::env::var("YAMI_V2_MODE").unwrap();
    let name = std::env::var("YAMI_V2_NAME").unwrap();
    let stage = std::env::var("YAMI_V2_STAGE").unwrap();
    let p = PathBuf::from(path);
    let a = fixture(128);
    let hook = |file: &str, at: store::Publication| {
        if file == name && format!("{at:?}") == stage {
            std::process::exit(86);
        }
    };
    if mode == "export" {
        store::save_observed(&a, &p.join("target"), hook).unwrap();
    } else {
        let (m, _) = chunks(&a);
        store::transfer_observed(
            &p.join("source"),
            &p.join("target"),
            Genesis::local(),
            m.anchor(),
            hook,
        )
        .unwrap();
    }
    panic!("unreached crash boundary");
}
fn crash(p: &Path, mode: &str, name: &str, stage: &str) {
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "checkpoint_v2::tests::child", "--nocapture"])
        .env("YAMI_V2_CHILD", p)
        .env("YAMI_V2_MODE", mode)
        .env("YAMI_V2_NAME", name)
        .env("YAMI_V2_STAGE", stage)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(86));
}
#[test]
fn export_and_transfer_process_crashes_resume_exactly() {
    for mode in ["export", "transfer"] {
        for (name, stage) in [
            ("state.bin", "Synced"),
            ("chunk-000000.bin", "Synced"),
            ("chunk-000001.bin", "Renamed"),
            ("anchor.bin", "DirectorySynced"),
            ("manifest.bin", "Synced"),
            ("complete.bin", "Renamed"),
        ] {
            let p = dir();
            let a = fixture(128);
            let expected = store::save(&a, &p.join("source")).unwrap();
            let before = hashes(&p.join("source"));
            crash(&p, mode, name, stage);
            if name != "complete.bin" {
                assert!(store::recover(&p.join("target"), Genesis::local(), expected).is_err());
            }
            if mode == "export" {
                assert_eq!(store::save(&a, &p.join("target")).unwrap(), expected);
            } else {
                store::transfer(
                    &p.join("source"),
                    &p.join("target"),
                    Genesis::local(),
                    expected,
                )
                .unwrap();
            }
            assert_eq!(
                store::recover(&p.join("target"), Genesis::local(), expected)
                    .unwrap()
                    .anchor(),
                a.anchor()
            );
            assert_eq!(hashes(&p.join("source")), before);
            fs::remove_dir_all(p).unwrap();
        }
    }
}
#[test]
fn conflicting_partial_and_unknown_tails_are_retained_not_repaired() {
    let p = dir();
    let a = fixture(128);
    let expected = store::save(&a, &p.join("source")).unwrap();
    crash(&p, "transfer", "chunk-000001.bin", "Synced");
    let pending = p.join("target/.pending-chunk-000001.bin");
    let mut bytes = fs::read(&pending).unwrap();
    bytes[20] ^= 1;
    fs::write(&pending, &bytes).unwrap();
    assert!(store::transfer(
        &p.join("source"),
        &p.join("target"),
        Genesis::local(),
        expected
    )
    .is_err());
    assert_eq!(fs::read(pending).unwrap(), bytes);
    fs::remove_dir_all(p).unwrap();
}
#[test]
fn shared_cold_readers_rebuild_and_detect_missing_or_extra_data() {
    let p = dir();
    let a = fixture(128);
    let expected = store::save(&a, &p).unwrap();
    let before = hashes(&p);
    std::thread::scope(|s| {
        for _ in 0..4 {
            let p = &p;
            s.spawn(move || {
                assert_eq!(
                    store::recover(p, Genesis::local(), expected)
                        .unwrap()
                        .anchor(),
                    expected.state
                )
            });
        }
    });
    assert_eq!(hashes(&p), before);
    fs::write(p.join("chunk-000002.bin"), b"extra").unwrap();
    assert!(store::recover(&p, Genesis::local(), expected).is_err());
    fs::remove_file(p.join("chunk-000002.bin")).unwrap();
    fs::remove_file(p.join("chunk-000001.bin")).unwrap();
    assert!(store::recover(&p, Genesis::local(), expected).is_err());
    fs::remove_dir_all(p).unwrap();
}
#[cfg(unix)]
#[test]
fn symlinks_and_hardlinks_cannot_redirect_lab_writes() {
    let p = dir();
    fs::create_dir_all(&p).unwrap();
    let outside = p.join("safety");
    fs::write(&outside, b"").unwrap();
    let target = p.join("lab");
    fs::create_dir(&target).unwrap();
    fs::hard_link(&outside, target.join("lab.lock")).unwrap();
    assert!(store::save(&fixture(1), &target).is_err());
    assert_eq!(fs::read(&outside).unwrap(), b"");
    fs::remove_file(target.join("lab.lock")).unwrap();
    std::os::unix::fs::symlink(&outside, target.join("inactive-checkpoint-v2.bin")).unwrap();
    assert!(store::save(&fixture(1), &target).is_err());
    assert_eq!(fs::read(&outside).unwrap(), b"");
    fs::remove_dir_all(p).unwrap();
}
// Actual bounded-tmpfs pressure, invoked by tools/yamichain-v2-lab.py.
#[test]
fn actual_enospc_child() {
    let Some(path) = std::env::var_os("YAMI_V2_ENOSPC") else {
        return;
    };
    let p = PathBuf::from(path);
    let kind = std::env::var("YAMI_V2_PRESSURE").unwrap();
    let a = fixture(128);
    let expected = store::save(&a, &p.join("source")).unwrap();
    crash(&p, "transfer", "chunk-000000.bin", "Renamed");
    let before = hashes(&p.join("source"));
    let mut fillers = Vec::new();
    if kind == "blocks" {
        use std::io::Write;
        let path = p.join("filler");
        let mut file = fs::File::create(&path).unwrap();
        loop {
            match file.write_all(&[0; 65536]) {
                Ok(()) => (),
                Err(e) => {
                    assert_eq!(e.raw_os_error(), Some(28));
                    break;
                }
            }
        }
        let len = file.metadata().unwrap().len();
        file.set_len(len.saturating_sub(4096)).unwrap();
        fillers.push(path);
    } else {
        for n in 0u64.. {
            let path = p.join(format!("fill-{n}"));
            match fs::File::create(&path) {
                Ok(_) => fillers.push(path),
                Err(e) => {
                    assert_eq!(e.raw_os_error(), Some(28));
                    break;
                }
            }
        }
    }
    let failure = store::transfer(
        &p.join("source"),
        &p.join("target"),
        Genesis::local(),
        expected,
    )
    .unwrap_err();
    assert_eq!(failure.raw_os_error(), Some(28));
    assert!(!p.join("target/complete.bin").exists());
    assert_eq!(hashes(&p.join("source")), before);
    for path in fillers {
        fs::remove_file(path).unwrap();
    }
    let restored = store::transfer(
        &p.join("source"),
        &p.join("target"),
        Genesis::local(),
        expected,
    )
    .unwrap();
    assert_eq!(restored.anchor(), expected.state);
    assert_eq!(hashes(&p.join("source")), before);
}
