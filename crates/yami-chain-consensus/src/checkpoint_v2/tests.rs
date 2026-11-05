static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
use super::*;
use crate::{compact::*, signing::vote_preimage_for, types::*};
use malachite::app::types::core as m;
use yami_chain::{development_key, Signer as _};
use yami_chain_storage::Journal;
struct Fixture {
    path: std::path::PathBuf,
    journal: Journal,
    genesis: Genesis,
}
impl Fixture {
    fn new() -> Self {
        static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "yc-inactive-context-{}-{}",
            std::process::id(),
            ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let journal = Journal::open(&path).unwrap();
        let genesis = Genesis::development_schedule(&[(1, 4), (4, 7), (8, 4)]).unwrap();
        let mut state = journal.replay(genesis.clone(), |_| Ok(())).unwrap();
        for h in 1..=12 {
            let block = state.development_workload_block(2).unwrap();
            let mut signatures = Vec::new();
            for member in genesis.committee_at(h).unwrap().members() {
                let key = (0..7)
                    .map(|i| development_key("consensus", i))
                    .find(|k| k.verifying_key().to_bytes() == member.consensus_key)
                    .unwrap();
                let vote = Vote {
                    height: Height(h),
                    round: m::Round::ZERO,
                    value: m::NilOrVal::Val(Value(block.id().unwrap())),
                    step: m::VoteType::Precommit,
                    address: Address(Digest(member.consensus_key)),
                    extension: None,
                };
                signatures.push(m::CommitSignature::new(
                    vote.address,
                    key.sign(&vote_preimage_for(&genesis, &vote).unwrap())
                        .to_bytes(),
                ));
            }
            let certificate = m::CommitCertificate {
                height: Height(h),
                round: m::Round::ZERO,
                value_id: Value(block.id().unwrap()),
                commit_signatures: signatures,
            };
            let record = FinalizedRecord {
                block: block.clone(),
                certificate: crate::encode_certificate(&genesis, &certificate).unwrap(),
            };
            journal.append(&record).unwrap();
            state = state.execute(&block).unwrap();
        }
        Self {
            path,
            journal,
            genesis,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

type Prepared = (Vec<u8>, Vec<Vec<u8>>, Vec<Vec<u8>>, StateAnchor);
fn prepare(f: &Fixture) -> Prepared {
    let state = replay_certified(&f.journal, &f.genesis).unwrap();
    let inactive = InactiveState::from_verified_state(&state).unwrap();
    let mut chunks = Vec::new();
    let manifest = codec::export(&inactive, |_, b| {
        chunks.push(b.to_vec());
        Ok(())
    })
    .unwrap();
    let terminal = f.journal.record(12).unwrap().unwrap();
    let env = envelope(&f.genesis, &terminal, &manifest).unwrap();
    let history = (1..=12)
        .map(|h| f.journal.record(h).unwrap().unwrap().canonical().unwrap())
        .collect();
    let initial = State::new(f.genesis.clone()).unwrap();
    let minimum = StateAnchor {
        genesis: f.genesis.id(),
        height: 0,
        tip: initial.tip(),
        root: initial.root(),
    };
    (env, history, chunks, minimum)
}
#[test]
fn certified_ancestry_epoch_transitions_and_root_completion() {
    let _serial = SERIAL.lock().unwrap();
    let f = Fixture::new();
    let (env, history, chunks, minimum) = prepare(&f);
    let context =
        verify_context(&f.genesis, minimum, &env, history.iter().map(Vec::as_slice)).unwrap();
    let result = context
        .complete(|i| Ok(std::io::Cursor::new(&chunks[i as usize])))
        .unwrap();
    assert_eq!(result.anchor(), context.anchor());
    assert_eq!(result.names(), 24);
    let state = replay_certified(&f.journal, &f.genesis).unwrap();
    let v1 = export_compact_checkpoint(&f.journal, &f.genesis).unwrap();
    assert_eq!(
        result.v1_snapshot().unwrap(),
        state.snapshot_bytes().unwrap()
    );
    assert!(verify_compact_checkpoint(&f.genesis, &env, v1.anchor()).is_err());
    assert!(verify_context(&f.genesis, minimum, v1.bytes(), std::iter::empty::<&[u8]>()).is_err());
    assert!(verify_context(
        &f.genesis,
        minimum,
        b"YC-LAB-STATE2-1",
        std::iter::empty::<&[u8]>()
    )
    .is_err());
    // A pruned V1 observer can independently pin its current checkpoint and
    // inspect V2 without changing its base or retained history.
    prune_certified_before(&f.journal, &f.genesis, 7).unwrap();
    let before = std::fs::read(f.path.join("compact-base.bin")).unwrap();
    assert!(f.journal.record(7).unwrap().is_none());
    // Derive the independent V1 floor from history already authenticated above,
    // then verify the actual retained compact base and only its certified suffix.
    let record = FinalizedRecord::decode(&history[6]).unwrap();
    let floor = CheckpointAnchor {
        height: 7,
        block: record.block.id().unwrap(),
        state: record.block.header.state,
    };
    let base = verify_compact_checkpoint(&f.genesis, &before, floor).unwrap();
    let pinned = StateAnchor {
        genesis: f.genesis.id(),
        height: base.state().height(),
        tip: base.state().tip(),
        root: base.state().root(),
    };
    let upgraded = verify_context(
        &f.genesis,
        pinned,
        &env,
        history[7..].iter().map(Vec::as_slice),
    )
    .unwrap();
    assert_eq!(
        upgraded
            .complete(|i| Ok(std::io::Cursor::new(&chunks[i as usize])))
            .unwrap()
            .anchor(),
        context.anchor()
    );
    assert!(verify_context(
        &f.genesis,
        pinned,
        &env,
        history[8..].iter().map(Vec::as_slice)
    )
    .is_err());
    assert!(verify_context(
        &f.genesis,
        context.anchor(),
        &env,
        std::iter::empty::<&[u8]>()
    )
    .is_ok());
    assert_eq!(replay_certified(&f.journal, &f.genesis).unwrap(), state);
    assert_eq!(
        std::fs::read(f.path.join("compact-base.bin")).unwrap(),
        before
    );
}
#[test]
fn rejects_rollback_gaps_conflicts_corrupt_certificates_and_unknown_versions() {
    let _serial = SERIAL.lock().unwrap();
    let f = Fixture::new();
    let (env, history, chunks, minimum) = prepare(&f);
    let verify = |b: &[u8], h: &[Vec<u8>], min| {
        verify_context(&f.genesis, min, b, h.iter().map(Vec::as_slice))
    };
    assert!(verify(&env, &history[1..], minimum).is_err());
    assert!(verify(&env, &history[..11], minimum).is_err());
    let mut extra = history.clone();
    extra.push(history[11].clone());
    assert!(verify(&env, &extra, minimum).is_err());
    let mut hostile = history.clone();
    *hostile[5].last_mut().unwrap() ^= 1;
    assert!(verify(&env, &hostile, minimum).is_err());
    for offset in [0, DOMAIN.len(), DOMAIN.len() + 32] {
        let mut bad = env.clone();
        bad[offset] ^= 1;
        assert!(verify(&bad, &history, minimum).is_err());
    }
    // Peer framing hashes carry no finality authority: changing only a digest
    // is detected when the corresponding payload is read, not as a forged BFT
    // certificate during context verification.
    let mut framing = env.clone();
    *framing.last_mut().unwrap() ^= 1;
    let framed = verify(&framing, &history, minimum).unwrap();
    assert!(framed
        .complete(|i| Ok(std::io::Cursor::new(&chunks[i as usize])))
        .is_err());
    let mut trailing = env.clone();
    trailing.push(0);
    assert!(verify(&trailing, &history, minimum).is_err());
    let target = verify(&env, &history, minimum).unwrap().anchor();
    assert!(verify(&env, &[], target).is_ok());
    let mut bad = target;
    bad.height += 1;
    assert!(verify(&env, &[], bad).is_err());
    let mut bad = target;
    bad.tip = Digest([1; 32]);
    assert!(verify(&env, &[], bad).is_err());
    let mut bad = target;
    bad.root = Digest([1; 32]);
    assert!(verify(&env, &[], bad).is_err());
    let mut bad = minimum;
    bad.genesis = Digest([1; 32]);
    assert!(verify(&env, &history, bad).is_err());
    assert!(verify_context(
        &Genesis::local(),
        minimum,
        &env,
        std::iter::empty::<&[u8]>()
    )
    .is_err());
}
#[test]
fn hostile_peer_rehashes_cannot_authenticate_a_false_state() {
    let _serial = SERIAL.lock().unwrap();
    let f = Fixture::new();
    let (env, history, chunks, minimum) = prepare(&f);
    let context =
        verify_context(&f.genesis, minimum, &env, history.iter().map(Vec::as_slice)).unwrap();
    let mut bad = chunks.clone();
    bad[0][0] ^= 1;
    assert!(context
        .complete(|i| Ok(std::io::Cursor::new(&bad[i as usize])))
        .is_err());
    // A self-consistent different transport manifest/state still cannot bind to
    // the independently certified root, even when its hashes all match.
    let state = replay_certified(&f.journal, &f.genesis).unwrap();
    let next = state
        .execute(&state.development_workload_block(1).unwrap())
        .unwrap();
    let other = InactiveState::from_verified_state(&next).unwrap();
    let manifest = codec::export(&other, |_, _| Ok(())).unwrap();
    assert!(envelope(
        &f.genesis,
        &f.journal.record(12).unwrap().unwrap(),
        &manifest
    )
    .is_err());
    let _guard = Admission::acquire().unwrap();
    assert!(context
        .complete(|i| Ok(std::io::Cursor::new(&chunks[i as usize])))
        .is_err());
}

#[test]
fn peer_can_rehash_framing_but_complete_root_still_rejects() {
    let _serial = SERIAL.lock().unwrap();
    let f = Fixture::new();
    let (_, history, _, minimum) = prepare(&f);
    let state = replay_certified(&f.journal, &f.genesis).unwrap();
    let inactive = InactiveState::from_verified_state(&state).unwrap();
    let mut chunks = Vec::new();
    let manifest = codec::export(&inactive, |_, b| {
        chunks.push(b.to_vec());
        Ok(())
    })
    .unwrap();
    let mut encoded = manifest.canonical();
    // Codec v1 framing: state (104), counts (8), total (8), chunk count (4).
    let start = b"YC-INACTIVE-CHECKPOINT2-1".len() + 124;
    let mut name_chunk = None;
    for i in 0..manifest.chunks() {
        let pos = start + i * 45;
        if encoded[pos] == 1 {
            name_chunk = Some((i, pos));
        }
    }
    let (i, pos) = name_chunk.unwrap();
    // Alter the final label while preserving sorted order, size, ownership and
    // registration counts. Rehash the chunk exactly as a malicious peer can.
    let chunk = &mut chunks[i];
    let mut at = 0;
    let mut last = 0;
    while at < chunk.len() {
        let len = u32::from_be_bytes(chunk[at..at + 4].try_into().unwrap()) as usize;
        last = at + 4 + len - 1;
        at += 4 + len + 32;
    }
    assert_ne!(chunk[last], b'z');
    chunk[last] = b'z';
    let mut preimage = (i as u32).to_be_bytes().to_vec();
    preimage.extend_from_slice(&encoded[pos..pos + 13]);
    preimage.extend_from_slice(chunk);
    let digest = yami_chain::hash(b"YC-INACTIVE-CHECKPOINT2-CHUNK-1", &preimage);
    encoded[pos + 13..pos + 45].copy_from_slice(&digest.0);
    let mut w = Writer::default();
    w.raw(DOMAIN);
    w.raw(&f.genesis.id().0);
    w.raw(&f.genesis.committee_at(13).unwrap().hash().0);
    w.bytes(&history[11]).unwrap();
    w.bytes(&encoded).unwrap();
    let context =
        verify_context(&f.genesis, minimum, &w.0, history.iter().map(Vec::as_slice)).unwrap();
    let error = context
        .complete(|n| Ok(std::io::Cursor::new(&chunks[n as usize])))
        .unwrap_err();
    assert!(error.to_string().contains("flat state root"), "{error}");
}

#[test]
fn destination_readback_and_recovery_need_verified_context() {
    let _serial = SERIAL.lock().unwrap();
    let f = Fixture::new();
    let (env, history, _, minimum) = prepare(&f);
    let state = replay_certified(&f.journal, &f.genesis).unwrap();
    let source = f.path.join("inactive-source");
    let destination = f.path.join("inactive-destination");
    codec::store::save(
        &InactiveState::from_verified_state(&state).unwrap(),
        &source,
    )
    .unwrap();
    let context =
        verify_context(&f.genesis, minimum, &env, history.iter().map(Vec::as_slice)).unwrap();
    #[cfg(feature = "large-state-lab")]
    {
        let lab_path = f.path.join("lab-source");
        let rejected = f.path.join("lab-must-not-be-converted");
        yami_chain::snapshot_v2::store::save(
            &yami_chain::snapshot_v2::ExperimentalState::from_verified_state(&state).unwrap(),
            &lab_path,
        )
        .unwrap();
        assert!(context.transfer(&lab_path, &rejected).is_err());
        assert!(!rejected.exists());
    }
    assert!(Journal::open(&source).is_err());
    assert!(!source.join("store.lock").exists());
    assert_eq!(
        context.transfer(&source, &destination).unwrap().anchor(),
        context.anchor()
    );
    assert_eq!(
        context.recover(&destination).unwrap().anchor(),
        context.anchor()
    );
    let chunk = destination.join("chunk-000000.bin");
    let mut bytes = std::fs::read(&chunk).unwrap();
    bytes[0] ^= 1;
    std::fs::write(chunk, bytes).unwrap();
    assert!(context.recover(&destination).is_err());
    assert!(context.transfer(&source, &destination).is_err());
    assert_eq!(replay_certified(&f.journal, &f.genesis).unwrap(), state);
}
