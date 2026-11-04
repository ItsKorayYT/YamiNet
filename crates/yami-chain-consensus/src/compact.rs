//! Compact recovery under an explicit independently pinned checkpoint trust model.
//! Peer-provided hashes/anchors are never independent authority. Persisted local
//! bases are trusted metadata only after authenticated export or explicit import.
use crate::wire::verify_record;
use eyre::{eyre, Result};
use yami_chain::{
    codec::{Reader, Writer},
    Digest, Genesis, State,
};
use yami_chain_storage::{
    read_bounded, FinalizedRecord, Journal, ReplayBudget, MAX_COMPACT_BASE_BYTES, MAX_RECORD_BYTES,
};
const DOMAIN: &[u8] = b"YC-COMPACT-1";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckpointAnchor {
    pub height: u64,
    pub block: Digest,
    pub state: Digest,
}
impl CheckpointAnchor {
    pub fn from_state(state: &State) -> Self {
        Self {
            height: state.height(),
            block: state.tip(),
            state: state.root(),
        }
    }
}
pub struct VerifiedCompactCheckpoint {
    state: State,
    record: FinalizedRecord,
    bytes: Vec<u8>,
}
impl VerifiedCompactCheckpoint {
    pub fn state(&self) -> &State {
        &self.state
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn anchor(&self) -> CheckpointAnchor {
        CheckpointAnchor::from_state(&self.state)
    }
}
/// The caller obtains `anchor` out of band, independently of the untrusted
/// snapshot supplier, and pins the complete genesis/committee schedule locally.
pub fn verify_compact_checkpoint(
    genesis: &Genesis,
    bytes: &[u8],
    anchor: CheckpointAnchor,
) -> Result<VerifiedCompactCheckpoint> {
    genesis.validate()?;
    let mut r = Reader::new(bytes, MAX_COMPACT_BASE_BYTES)?;
    if r.take(DOMAIN.len())? != DOMAIN
        || r.array::<32>()? != genesis.id().0
        || r.u64()? != anchor.height
        || r.array::<32>()? != anchor.block.0
        || r.array::<32>()? != anchor.state.0
        || anchor.height == 0
        || anchor.height >= 1_000_000
    {
        return Err(eyre!(
            "compact checkpoint does not match independently pinned anchor/genesis"
        ));
    }
    if r.array::<32>()? != genesis.committee_at(anchor.height + 1)?.hash().0 {
        return Err(eyre!("checkpoint next committee mismatch"));
    }
    let record = FinalizedRecord::decode(r.bytes(MAX_RECORD_BYTES)?)?;
    verify_record(genesis, &record)?;
    if record.block.header.height != anchor.height
        || record.block.id()? != anchor.block
        || record.block.header.state != anchor.state
    {
        return Err(eyre!("checkpoint certificate/state binding"));
    }
    let state = State::restore_checkpoint(
        genesis.clone(),
        anchor.height,
        anchor.block,
        anchor.state,
        r.bytes(yami_chain::snapshot::MAX_STATE_SNAPSHOT_BYTES)?,
    )?;
    r.end()?;
    Ok(VerifiedCompactCheckpoint {
        state,
        record,
        bytes: bytes.to_vec(),
    })
}
fn marker_bytes(state: &State) -> Vec<u8> {
    let mut w = Writer::default();
    w.raw(&state.genesis().id().0);
    w.u64(state.height());
    w.raw(&state.tip().0);
    w.raw(&state.root().0);
    w.0
}
fn local_base(journal: &Journal, genesis: &Genesis) -> Result<Option<VerifiedCompactCheckpoint>> {
    let bytes = match read_bounded(
        &journal.path().join("compact-base.bin"),
        MAX_COMPACT_BASE_BYTES,
    ) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if journal.path().join("compacted.bin").exists() {
                return Err(eyre!("compact base missing; refusing rollback"));
            }
            return Ok(None);
        }
        Err(e) => return Err(e.into()),
    };
    // Only LOCAL, already authorized metadata may supply this restart anchor.
    // This helper is deliberately private; network imports use the API above.
    let mut r = Reader::new(&bytes, MAX_COMPACT_BASE_BYTES)?;
    if r.take(DOMAIN.len())? != DOMAIN || r.array::<32>()? != genesis.id().0 {
        return Err(eyre!("local checkpoint domain/genesis"));
    }
    let anchor = CheckpointAnchor {
        height: r.u64()?,
        block: Digest(r.array()?),
        state: Digest(r.array()?),
    };
    let verified = verify_compact_checkpoint(genesis, &bytes, anchor)?;
    match read_bounded(&journal.path().join("compacted.bin"), 104) {
        Ok(marker) => {
            let mut r = Reader::new(&marker, 104)?;
            if r.array::<32>()? != genesis.id().0 {
                return Err(eyre!("pruning marker genesis mismatch"));
            }
            let height = r.u64()?;
            let block = Digest(r.array()?);
            let root = Digest(r.array()?);
            r.end()?;
            if verified.state.height() < height
                || (verified.state.height() == height
                    && (verified.state.tip() != block || verified.state.root() != root))
            {
                return Err(eyre!(
                    "compact base rollback/conflict with durable trust marker"
                ));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    Ok(Some(verified))
}
pub fn replay_certified(journal: &Journal, genesis: &Genesis) -> Result<State> {
    let base = local_base(journal, genesis)?;
    if let Some(base) = base {
        Ok(
            journal.replay_from_state(base.state, ReplayBudget::default(), |r| {
                verify_record(genesis, r)
            })?,
        )
    } else {
        Ok(journal.replay(genesis.clone(), |r| verify_record(genesis, r))?)
    }
}
// An independently imported base need not align with the periodic interval.
// Wait until the proposed floor reaches it; explicit backward pruning still fails.
pub(crate) fn periodic_prune_floor(height: u64, interval: u64, history_min: u64) -> Option<u64> {
    if interval == 0 || height <= interval || !height.is_multiple_of(interval) {
        return None;
    }
    let floor = height - interval;
    (floor >= history_min.saturating_sub(1)).then_some(floor)
}
pub fn history_min_height(journal: &Journal, genesis: &Genesis) -> Result<u64> {
    Ok(local_base(journal, genesis)?.map_or(1, |b| b.state.height() + 1))
}
/// Sequential replay (or the existing authorized local base plus certified tail)
/// authenticates exports. A self-generated hash does not authorize fresh imports.
pub fn export_compact_checkpoint(
    journal: &Journal,
    genesis: &Genesis,
) -> Result<VerifiedCompactCheckpoint> {
    let state = replay_certified(journal, genesis)?;
    export_state(journal, genesis, state)
}
/// Only states obtained from a completed authenticated replay enter here.
fn export_state(
    journal: &Journal,
    genesis: &Genesis,
    state: State,
) -> Result<VerifiedCompactCheckpoint> {
    if state.height() == 0 {
        return Err(eyre!("no finalized checkpoint"));
    }
    let record = match journal.record(state.height())? {
        Some(r) => r,
        None => {
            local_base(journal, genesis)?
                .filter(|b| b.state == state)
                .ok_or_else(|| eyre!("checkpoint tip unavailable"))?
                .record
        }
    };
    let mut w = Writer::default();
    w.raw(DOMAIN);
    w.raw(&genesis.id().0);
    w.u64(state.height());
    w.raw(&state.tip().0);
    w.raw(&state.root().0);
    w.raw(&genesis.committee_at(state.height() + 1)?.hash().0);
    w.bytes(&record.canonical()?)?;
    w.u32(state.snapshot_len()?.try_into()?);
    state.append_snapshot(&mut w.0)?;
    verify_compact_checkpoint(genesis, &w.0, CheckpointAnchor::from_state(&state))
}
/// Fresh-node bootstrap only. This is NOT a validator identity backup. A key
/// previously used elsewhere must retain its own WAL and signing reservations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompactStage {
    BaseDurable,
    MarkerDurable,
    HistoryRecordRemoved,
}
pub fn import_compact_checkpoint(
    journal: &Journal,
    genesis: &Genesis,
    bytes: &[u8],
    anchor: CheckpointAnchor,
) -> Result<State> {
    import_compact_checkpoint_observed(journal, genesis, bytes, anchor, |_| {})
}
pub fn import_compact_checkpoint_observed(
    journal: &Journal,
    genesis: &Genesis,
    bytes: &[u8],
    anchor: CheckpointAnchor,
    mut observe: impl FnMut(CompactStage),
) -> Result<State> {
    let incoming = verify_compact_checkpoint(genesis, bytes, anchor)?;
    for entry in std::fs::read_dir(journal.path())? {
        let name = entry?.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("archive-")
            || name.starts_with("signed-")
            || name.starts_with("signing-")
            || name.starts_with("equivocation-")
            || name == "wal"
            || name == "validator-index.bin"
        {
            return Err(eyre!("checkpoint import is not validator identity recovery; retain original safety store"));
        }
    }
    let local = replay_certified(journal, genesis)?;
    if local.height() != 0 && local != incoming.state {
        return Err(eyre!(
            "compact import would replace/roll back initialized state"
        ));
    }
    if local.height() == 0 && journal.record(1)?.is_some() {
        return Err(eyre!("nonempty archive"));
    }
    journal.install_compact_base(&incoming.bytes)?;
    observe(CompactStage::BaseDurable);
    journal.install_compact_marker(&marker_bytes(&incoming.state))?;
    observe(CompactStage::MarkerDurable);
    Ok(incoming.state)
}
/// Opt-in local pruning capability. Replays and verifies the exact durable base
/// before deleting anything. The base and marker are fsynced before unlinking;
/// interrupted deletion leaves a valid mixture of retained/removed old records.
pub fn prune_certified_history(journal: &Journal, genesis: &Genesis) -> Result<State> {
    let current = replay_certified(journal, genesis)?;
    prune_certified_before(journal, genesis, current.height())
}
pub fn prune_certified_before(journal: &Journal, genesis: &Genesis, floor: u64) -> Result<State> {
    prune_certified_before_observed(journal, genesis, floor, |_| {})
}
pub fn prune_certified_before_observed(
    journal: &Journal,
    genesis: &Genesis,
    floor: u64,
    mut observe: impl FnMut(CompactStage),
) -> Result<State> {
    if journal.is_segmented_archive() {
        return Err(eyre!(
            "segmented archive retains finalized history; export to a fresh pruned store"
        ));
    }
    let seed = local_base(journal, genesis)?.map_or(State::new(genesis.clone())?, |b| b.state);
    if floor < seed.height() {
        return Err(eyre!("checkpoint rollback below local base"));
    }
    let mut checkpoint_state = None;
    // Capture the floor while verifying the ENTIRE suffix once. Do not publish
    // anything if a later certificate, receipt, gap or transition fails.
    let current = journal.replay_from_state_observed(
        seed,
        ReplayBudget::default(),
        |r| verify_record(genesis, r),
        |state| {
            if state.height() == floor {
                checkpoint_state = Some(state.clone());
            }
        },
    )?;
    let checkpoint = export_state(
        journal,
        genesis,
        checkpoint_state.ok_or_else(|| eyre!("checkpoint beyond finalized head"))?,
    )?;
    journal.install_compact_base(checkpoint.bytes())?;
    observe(CompactStage::BaseDurable);
    journal.install_compact_marker(&marker_bytes(&checkpoint.state))?;
    observe(CompactStage::MarkerDurable);
    journal.remove_checkpoint_history_observed(
        |bytes| {
            let verified = verify_compact_checkpoint(genesis, bytes, checkpoint.anchor())
                .map_err(std::io::Error::other)?;
            Ok(verified.state)
        },
        || observe(CompactStage::HistoryRecordRemoved),
    )?;
    checkpoint_prune_candidates(journal, &checkpoint)?;
    Ok(current)
}
fn checkpoint_prune_candidates(
    journal: &Journal,
    checkpoint: &VerifiedCompactCheckpoint,
) -> Result<()> {
    journal.remove_finalized_candidates(checkpoint.state.height())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{signing::vote_preimage_for, types::*};
    use malachite::app::types::core as m;
    use yami_chain::{development_key, Signer as _};
    struct Fixture {
        path: std::path::PathBuf,
        journal: Journal,
        genesis: Genesis,
    }
    impl Fixture {
        fn new() -> Self {
            static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "yc-compact-{}-{}",
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
    #[test]
    fn experimental_v2_bytes_never_enter_production_checkpoint_or_pruning_paths() {
        let f = Fixture::new();
        let checkpoint = export_compact_checkpoint(&f.journal, &f.genesis).unwrap();
        let bogus = b"YC-LAB-STATE2-1";
        assert!(verify_compact_checkpoint(&f.genesis, bogus, checkpoint.anchor()).is_err());
        let path = f.path.with_extension("v2-boundary");
        let fresh = Journal::open(&path).unwrap();
        assert!(import_compact_checkpoint(&fresh, &f.genesis, bogus, checkpoint.anchor()).is_err());
        assert!(!path.join("compact-base.bin").exists());
        assert!(!path.join("compacted.bin").exists());
        drop(fresh);
        std::fs::remove_dir_all(path).unwrap();
        assert_eq!(
            checkpoint.bytes(),
            export_compact_checkpoint(&f.journal, &f.genesis)
                .unwrap()
                .bytes()
        );
    }
    #[test]
    fn segmented_history_preserves_authenticated_exports_sync_and_pruned_interoperability() {
        let f = Fixture::new();
        let before = export_compact_checkpoint(&f.journal, &f.genesis).unwrap();
        let state = replay_certified(&f.journal, &f.genesis).unwrap();
        f.journal
            .migrate_archive(f.genesis.clone(), |r| verify_record(&f.genesis, r))
            .unwrap();
        let after = export_compact_checkpoint(&f.journal, &f.genesis).unwrap();
        assert_eq!(before.bytes(), after.bytes());
        assert_eq!(history_min_height(&f.journal, &f.genesis).unwrap(), 1);
        assert!(prune_certified_before(&f.journal, &f.genesis, 8).is_err());
        assert!(!f.path.join("compact-base.bin").exists());
        let cp = crate::authenticate_checkpoint(&f.journal, &f.genesis, 8, 12).unwrap();
        assert_eq!(cp.height(), 8);
        let target = Journal::open(&f.path.with_extension("pruned-peer")).unwrap();
        target.replay(f.genesis.clone(), |_| Ok(())).unwrap();
        for h in 1..=8 {
            target
                .append(&f.journal.record(h).unwrap().unwrap())
                .unwrap();
        }
        prune_certified_before(&target, &f.genesis, 8).unwrap();
        for h in 9..=12 {
            let record = f.journal.record(h).unwrap().unwrap();
            verify_record(&f.genesis, &record).unwrap();
            target.append(&record).unwrap();
        }
        assert_eq!(replay_certified(&target, &f.genesis).unwrap(), state);
        let fresh_path = f.path.with_extension("compact-peer");
        let fresh = Journal::open(&fresh_path).unwrap();
        let mut hostile = after.bytes().to_vec();
        let last = hostile.len() - 1;
        hostile[last] ^= 1;
        assert!(import_compact_checkpoint(&fresh, &f.genesis, &hostile, after.anchor()).is_err());
        assert_eq!(
            import_compact_checkpoint(&fresh, &f.genesis, after.bytes(), after.anchor()).unwrap(),
            state
        );
        let history = crate::export_history_snapshot(&f.journal, &f.genesis, 12).unwrap();
        let sync_path = f.path.with_extension("segmented-sync");
        let mut sync = Journal::open(&sync_path).unwrap();
        sync.migrate_archive(f.genesis.clone(), |r| verify_record(&f.genesis, r))
            .unwrap();
        let mut hostile_history = history.clone();
        let last = hostile_history.len() - 1;
        hostile_history[last] ^= 1;
        let hostile_digest = crate::history_snapshot_digest(&hostile_history);
        assert!(crate::restore_history_snapshot(
            &mut sync,
            &f.genesis,
            &hostile_history,
            hostile_digest
        )
        .is_err());
        assert!(sync.record(1).unwrap().is_none());
        assert_eq!(
            crate::restore_history_snapshot(
                &mut sync,
                &f.genesis,
                &history,
                crate::history_snapshot_digest(&history)
            )
            .unwrap(),
            state
        );
        drop(sync);
        std::fs::remove_dir_all(sync_path).unwrap();
        let target_path = target.path().to_owned();
        drop(target);
        drop(fresh);
        std::fs::remove_dir_all(target_path).unwrap();
        std::fs::remove_dir_all(fresh_path).unwrap();
    }
    #[test]
    fn periodic_pruning_waits_for_unaligned_checkpoint_without_rolling_it_back() {
        let f = Fixture::new();
        let current = replay_certified(&f.journal, &f.genesis).unwrap();
        prune_certified_before(&f.journal, &f.genesis, 7).unwrap();
        let base = std::fs::read(f.path.join("compact-base.bin")).unwrap();
        let minimum = history_min_height(&f.journal, &f.genesis).unwrap();
        assert_eq!(periodic_prune_floor(9, 3, minimum), None);
        assert!(prune_certified_before(&f.journal, &f.genesis, 6).is_err());
        assert_eq!(
            std::fs::read(f.path.join("compact-base.bin")).unwrap(),
            base
        );
        let floor = periodic_prune_floor(12, 3, minimum).unwrap();
        assert_eq!(
            prune_certified_before(&f.journal, &f.genesis, floor).unwrap(),
            current
        );
        assert_eq!(history_min_height(&f.journal, &f.genesis).unwrap(), 10);
        assert_eq!(replay_certified(&f.journal, &f.genesis).unwrap(), current);
    }
    #[test]
    fn captured_floor_is_never_published_when_later_history_fails() {
        for corrupt in ["receipt", "finalized"] {
            let f = Fixture::new();
            let target = f.path.join(format!("{corrupt}-00000000000000000012.bin"));
            std::fs::write(target, b"corrupt suffix after captured floor").unwrap();
            assert!(prune_certified_before(&f.journal, &f.genesis, 7).is_err());
            assert!(!f.path.join("compact-base.bin").exists());
            assert!(!f.path.join("compacted.bin").exists());
            assert!(f.path.join("finalized-00000000000000000001.bin").exists());
        }
        let f = Fixture::new();
        assert!(prune_certified_before(&f.journal, &f.genesis, 13).is_err());
        assert!(!f.path.join("compact-base.bin").exists());
    }
    #[test]
    fn compact_rotations_tail_and_all_safety_files_survive() {
        let f = Fixture::new();
        let before = replay_certified(&f.journal, &f.genesis).unwrap();
        for name in [
            "signed-00000000000000000002-0000000000-1.bin",
            "equivocation-00000000000000000003-0000000000.bin",
            "signed-00000000000000000013-0000000000-1.bin",
            "candidate-00000000000000000013-pending.bin",
        ] {
            f.journal.write_once(name, b"do not delete").unwrap();
        }
        std::fs::create_dir(f.path.join("wal")).unwrap();
        std::fs::write(f.path.join("wal/consensus.wal"), b"retain original WAL").unwrap();
        assert_eq!(
            prune_certified_before(&f.journal, &f.genesis, 7).unwrap(),
            before
        );
        assert_eq!(history_min_height(&f.journal, &f.genesis).unwrap(), 8);
        assert!(f.journal.record(1).unwrap().is_none());
        assert!(f.journal.record(8).unwrap().is_some());
        assert_eq!(replay_certified(&f.journal, &f.genesis).unwrap(), before);
        assert!(f
            .journal
            .replay(f.genesis.clone(), |r| verify_record(&f.genesis, r))
            .is_err());
        for name in [
            "signed-00000000000000000002-0000000000-1.bin",
            "equivocation-00000000000000000003-0000000000.bin",
            "signed-00000000000000000013-0000000000-1.bin",
            "candidate-00000000000000000013-pending.bin",
            "wal/consensus.wal",
        ] {
            assert!(f.path.join(name).is_file(), "{name}");
        }
        assert!(prune_certified_before(&f.journal, &f.genesis, 6).is_err());
        assert_eq!(
            prune_certified_before(&f.journal, &f.genesis, 10).unwrap(),
            before
        );
        assert_eq!(replay_certified(&f.journal, &f.genesis).unwrap(), before);
    }
    #[test]
    fn durable_minimum_anchor_rejects_replacement_by_older_valid_base() {
        let f = Fixture::new();
        prune_certified_before(&f.journal, &f.genesis, 7).unwrap();
        let old = std::fs::read(f.path.join("compact-base.bin")).unwrap();
        prune_certified_history(&f.journal, &f.genesis).unwrap();
        assert!(f.journal.record(11).unwrap().is_none());
        let marker = std::fs::read(f.path.join("compacted.bin")).unwrap();
        std::fs::write(f.path.join("compact-base.bin"), old).unwrap();
        assert!(replay_certified(&f.journal, &f.genesis).is_err());
        assert!(prune_certified_history(&f.journal, &f.genesis).is_err());
        assert_eq!(std::fs::read(f.path.join("compacted.bin")).unwrap(), marker);
    }
    #[test]
    fn no_peer_hash_authority_forgery_corruption_and_wrong_committees() {
        let f = Fixture::new();
        let snapshot = export_compact_checkpoint(&f.journal, &f.genesis).unwrap();
        let a = snapshot.anchor();
        assert!(verify_compact_checkpoint(&f.genesis, snapshot.bytes(), a).is_ok());
        for len in [0, 1, 11, 42, 147, snapshot.bytes().len() - 1] {
            assert!(verify_compact_checkpoint(&f.genesis, &snapshot.bytes()[..len], a).is_err());
        }
        for offset in [0, 12, 44, 52, 84, 116, 150, snapshot.bytes().len() - 1] {
            let mut b = snapshot.bytes().to_vec();
            b[offset] ^= 1;
            assert!(
                verify_compact_checkpoint(&f.genesis, &b, a).is_err(),
                "{offset}"
            );
        }
        let mut wrong = a;
        wrong.height -= 1;
        assert!(verify_compact_checkpoint(&f.genesis, snapshot.bytes(), wrong).is_err());
        let mut wrong = a;
        wrong.state = Digest([42; 32]);
        assert!(verify_compact_checkpoint(&f.genesis, snapshot.bytes(), wrong).is_err());
        assert!(verify_compact_checkpoint(&Genesis::local(), snapshot.bytes(), a).is_err());
        let mut trailing = snapshot.bytes().to_vec();
        trailing.push(0);
        assert!(verify_compact_checkpoint(&f.genesis, &trailing, a).is_err());
        // Even a caller echoing a forged peer's advertised root cannot pass its BFT certificate.
        let mut forged = snapshot.bytes().to_vec();
        forged[84..116].copy_from_slice(&[42; 32]);
        assert!(verify_compact_checkpoint(&f.genesis, &forged, wrong).is_err());
    }
    #[test]
    fn advertised_state_counts_and_corrupted_certified_tail_fail_closed() {
        let f = Fixture::new();
        let checkpoint = export_compact_checkpoint(&f.journal, &f.genesis).unwrap();
        let mut r = Reader::new(checkpoint.bytes(), MAX_COMPACT_BASE_BYTES).unwrap();
        r.take(DOMAIN.len() + 32 + 8 + 32 * 3).unwrap();
        r.bytes(MAX_RECORD_BYTES).unwrap();
        let state_bytes = r
            .bytes(yami_chain::snapshot::MAX_STATE_SNAPSHOT_BYTES)
            .unwrap();
        let start = checkpoint.bytes().len() - state_bytes.len();
        let mut oversized = checkpoint.bytes().to_vec();
        oversized[start + 82..start + 86].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(verify_compact_checkpoint(&f.genesis, &oversized, checkpoint.anchor()).is_err());
        let last = f.path.join("finalized-00000000000000000012.bin");
        let original = std::fs::read(&last).unwrap();
        let mut corrupt = original.clone();
        *corrupt.last_mut().unwrap() ^= 1;
        std::fs::write(&last, corrupt).unwrap();
        assert!(prune_certified_before(&f.journal, &f.genesis, 7).is_err());
        assert!(f.journal.record(1).unwrap().is_some());
        assert!(!f.path.join("compact-base.bin").exists());
        std::fs::write(last, original).unwrap();
        prune_certified_before(&f.journal, &f.genesis, 7).unwrap();
        std::fs::remove_file(f.path.join("finalized-00000000000000000011.bin")).unwrap();
        assert!(replay_certified(&f.journal, &f.genesis).is_err()); // receipt/future suffix forbids a partial replay
    }
    #[test]
    fn fresh_import_idempotence_and_used_identity_refusal() {
        let f = Fixture::new();
        let checkpoint = export_compact_checkpoint(&f.journal, &f.genesis).unwrap();
        let destination = f.path.join("fresh");
        let j = Journal::open(&destination).unwrap();
        let state =
            import_compact_checkpoint(&j, &f.genesis, checkpoint.bytes(), checkpoint.anchor())
                .unwrap();
        assert_eq!(replay_certified(&j, &f.genesis).unwrap(), state);
        assert!(j.record(1).unwrap().is_none());
        assert_eq!(
            import_compact_checkpoint(&j, &f.genesis, checkpoint.bytes(), checkpoint.anchor())
                .unwrap(),
            state
        );
        j.reserve_signature(13, 0, 1, b"reservation").unwrap();
        assert!(
            import_compact_checkpoint(&j, &f.genesis, checkpoint.bytes(), checkpoint.anchor())
                .is_err()
        );
        assert_eq!(
            j.signature_reservation(13, 0, 1).unwrap().unwrap(),
            b"reservation"
        );
    }
    #[test]
    fn durable_base_before_partial_deletion_and_missing_base_fail_closed() {
        let f = Fixture::new();
        let checkpoint = export_compact_checkpoint(&f.journal, &f.genesis).unwrap();
        f.journal.install_compact_base(checkpoint.bytes()).unwrap(); // crash before marker/deletion
        assert_eq!(
            replay_certified(&f.journal, &f.genesis).unwrap(),
            *checkpoint.state()
        );
        f.journal
            .install_compact_marker(&marker_bytes(checkpoint.state()))
            .unwrap();
        for h in 1..=5 {
            std::fs::remove_file(f.path.join(format!("finalized-{h:020}.bin"))).unwrap();
        } // interrupted unlink sequence
        assert_eq!(
            replay_certified(&f.journal, &f.genesis).unwrap(),
            *checkpoint.state()
        );
        assert_eq!(
            prune_certified_history(&f.journal, &f.genesis).unwrap(),
            *checkpoint.state()
        );
        std::fs::remove_file(f.path.join("compact-base.bin")).unwrap();
        assert!(replay_certified(&f.journal, &f.genesis).is_err());
        assert!(f.journal.replay(f.genesis.clone(), |_| Ok(())).is_err());
    }
}
