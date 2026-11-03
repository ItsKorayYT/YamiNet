//! Local checkpoints authenticated by sequential execution from a pinned genesis.
//! These cannot bootstrap snapshot trust or authorize removal of certified history.
use crate::wire::{decode_certificate, verify_record};
use eyre::{eyre, Result};
use yami_chain::{
    codec::{Reader, Writer},
    Digest, Genesis, State,
};
use yami_chain_storage::{read_bounded, FinalizedRecord, Journal, MAX_RECORD_BYTES};
#[derive(Debug)]
pub struct AuthenticatedCheckpoint {
    genesis: Digest,
    height: u64,
    block: Digest,
    state: Digest,
    next_committee: Digest,
    certificate: Vec<u8>,
}
impl AuthenticatedCheckpoint {
    pub fn height(&self) -> u64 {
        self.height
    }
    pub fn canonical(&self) -> yami_chain::Result<Vec<u8>> {
        let mut w = Writer::default();
        w.raw(b"YC-CHECKPOINT-2");
        w.raw(&self.genesis.0);
        w.u64(self.height);
        for d in [self.block, self.state, self.next_committee] {
            w.raw(&d.0);
        }
        w.bytes(&self.certificate)?;
        Ok(w.0)
    }
    pub fn persist(&self, journal: &Journal) -> Result<()> {
        journal.write_once(
            &format!("checkpoint-{:020}.bin", self.height),
            &self.canonical()?,
        )?;
        Ok(())
    }
    /// Remove only reproducible candidate payloads; retain all certificates,
    /// original equivocation evidence, signer reservations, WAL and genesis.
    pub fn prune_candidates(&self, journal: &Journal) -> Result<()> {
        let record = journal
            .record(self.height)?
            .ok_or_else(|| eyre!("checkpoint history missing"))?;
        if record.block.id()? != self.block
            || record.block.header.state != self.state
            || record.certificate != self.certificate
        {
            return Err(eyre!("checkpoint store mismatch"));
        }
        journal.remove_finalized_candidates(self.height)?;
        Ok(())
    }
}
/// The record count is explicitly bounded by the caller. Exceeding the budget
/// fails before replay; it never skips certificate or state verification.
pub fn authenticate_checkpoint(
    journal: &Journal,
    genesis: &Genesis,
    height: u64,
    max_records: u64,
) -> Result<AuthenticatedCheckpoint> {
    if height == 0 || height > max_records || height > 1_000_000 {
        return Err(eyre!("checkpoint replay budget"));
    }
    if read_bounded(&journal.path().join("genesis.bin"), 1_048_576)? != genesis.canonical() {
        return Err(eyre!("checkpoint genesis mismatch"));
    }
    let mut state = State::new(genesis.clone())?;
    let mut certificate = Vec::new();
    for h in 1..=height {
        let record = journal
            .record(h)?
            .ok_or_else(|| eyre!("checkpoint history gap"))?;
        verify_record(genesis, &record)?;
        state = state.execute(&record.block)?;
        certificate = record.certificate;
    }
    let cert = decode_certificate(genesis, &certificate)?;
    if cert.value_id.0 != state.tip() {
        return Err(eyre!("checkpoint tip mismatch"));
    }
    Ok(AuthenticatedCheckpoint {
        genesis: genesis.id(),
        height,
        block: state.tip(),
        state: state.root(),
        next_committee: genesis.committee_at(height + 1)?.hash(),
        certificate,
    })
}
/// Validate a stored checkpoint by recomputing it. No unverified deserialization
/// yields an AuthenticatedCheckpoint or an accepting snapshot-import capability.
pub fn verify_checkpoint(
    journal: &Journal,
    genesis: &Genesis,
    height: u64,
    max_records: u64,
    bytes: &[u8],
) -> Result<AuthenticatedCheckpoint> {
    let authenticated = authenticate_checkpoint(journal, genesis, height, max_records)?;
    if authenticated.canonical()? != bytes {
        return Err(eyre!("checkpoint commitment mismatch"));
    }
    Ok(authenticated)
}

/// Transport inventory for verified historical records, not a compact state
/// snapshot or an import capability. All history/evidence remains in place.
/// Private construction requires authenticated sequential replay. Restoring from
/// an inventory still requires every record and normal certificate execution.
#[derive(Debug)]
pub struct HistorySnapshotInventory {
    checkpoint: AuthenticatedCheckpoint,
    records: Vec<yami_chain::Digest>,
    bytes: u64,
}
impl HistorySnapshotInventory {
    pub fn canonical(&self) -> yami_chain::Result<Vec<u8>> {
        let mut w = Writer::default();
        w.raw(b"YC-HISTORY-INVENTORY-1");
        w.bytes(&self.checkpoint.canonical()?)?;
        w.u64(self.bytes);
        w.u64(self.records.len() as u64);
        for digest in &self.records {
            w.raw(&digest.0);
        }
        Ok(w.0)
    }
    pub fn commitment(&self) -> yami_chain::Result<Digest> {
        Ok(yami_chain::hash(
            b"YC-HISTORY-INVENTORY-HASH-1",
            &self.canonical()?,
        ))
    }
}
/// Refuses excess work before replay; authenticates each included record and
/// checks byte budget incrementally. Never deletes evidence, WAL or history.
pub fn history_snapshot_inventory(
    journal: &Journal,
    genesis: &Genesis,
    height: u64,
    max_bytes: u64,
) -> Result<HistorySnapshotInventory> {
    if height == 0 || height > 1024 {
        return Err(eyre!("inventory record budget"));
    }
    let mut remaining = max_bytes;
    let mut records = Vec::new();
    for h in 1..=height {
        let record = journal
            .record(h)?
            .ok_or_else(|| eyre!("inventory history gap"))?;
        let bytes = record.canonical()?;
        remaining = remaining
            .checked_sub(bytes.len() as u64)
            .ok_or_else(|| eyre!("inventory byte budget"))?;
        verify_record(genesis, &record)?;
        records.push(yami_chain::hash(b"YC-HISTORY-RECORD-1", &bytes));
    }
    let checkpoint = authenticate_checkpoint(journal, genesis, height, 1024)?;
    Ok(HistorySnapshotInventory {
        checkpoint,
        records,
        bytes: max_bytes - remaining,
    })
}

/// Verify transported inventory bytes by recomputation against local authenticated
/// history. This deliberately grants no state import or history deletion ability.
pub fn verify_history_inventory(
    journal: &Journal,
    genesis: &Genesis,
    height: u64,
    max_bytes: u64,
    bytes: &[u8],
) -> Result<HistorySnapshotInventory> {
    if bytes.len() > 65_536 {
        return Err(eyre!("inventory encoding cap"));
    }
    let inventory = history_snapshot_inventory(journal, genesis, height, max_bytes)?;
    if inventory.canonical()? != bytes {
        return Err(eyre!("inventory commitment mismatch"));
    }
    Ok(inventory)
}

/// Bounded full-history transport, not a compact state snapshot or a pruning
/// capability. Certificates and execution are always checked from pinned genesis.
pub const MAX_HISTORY_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;
const SNAPSHOT_DOMAIN: &[u8] = b"YC-HISTORY-SNAPSHOT-1";

pub fn history_snapshot_digest(bytes: &[u8]) -> Digest {
    yami_chain::hash(b"YC-HISTORY-SNAPSHOT-HASH-1", bytes)
}

pub fn export_history_snapshot(
    journal: &Journal,
    genesis: &Genesis,
    height: u64,
) -> Result<Vec<u8>> {
    let checkpoint = authenticate_checkpoint(journal, genesis, height, 1024)?;
    let mut w = Writer::default();
    w.raw(SNAPSHOT_DOMAIN);
    w.raw(&checkpoint.genesis.0);
    w.u64(height);
    for digest in [
        checkpoint.block,
        checkpoint.state,
        checkpoint.next_committee,
    ] {
        w.raw(&digest.0);
    }
    for h in 1..=height {
        let record = journal
            .record(h)?
            .ok_or_else(|| eyre!("snapshot history gap"))?;
        let bytes = record.canonical()?;
        if w.0
            .len()
            .checked_add(4 + bytes.len())
            .is_none_or(|n| n > MAX_HISTORY_SNAPSHOT_BYTES)
        {
            return Err(eyre!("snapshot byte budget"));
        }
        w.bytes(&bytes)?;
    }
    Ok(w.0)
}

/// The caller pins the transport digest independently. Never treat a digest
/// supplied alongside an untrusted download as evidence of the latest head.
/// Validate the complete archive before writing finalized records. Installation
/// is a resumable series of atomic certified appends: an interrupted operation
/// can leave a valid prefix, never an imported unauthenticated state or success.
/// Does not copy signer/WAL/identity material; not a validator identity backup.
/// The exclusive journal borrow prevents concurrent node appends during restore.
pub fn restore_history_snapshot(
    journal: &mut Journal,
    genesis: &Genesis,
    bytes: &[u8],
    trusted_digest: Digest,
) -> Result<State> {
    if bytes.len() > MAX_HISTORY_SNAPSHOT_BYTES || history_snapshot_digest(bytes) != trusted_digest
    {
        return Err(eyre!("snapshot digest or byte budget"));
    }
    let mut r = Reader::new(bytes, MAX_HISTORY_SNAPSHOT_BYTES)?;
    if r.take(SNAPSHOT_DOMAIN.len())? != SNAPSHOT_DOMAIN || r.array::<32>()? != genesis.id().0 {
        return Err(eyre!("snapshot domain or genesis"));
    }
    let height = r.u64()?;
    if height == 0 || height > 1024 {
        return Err(eyre!("snapshot record budget"));
    }
    let tip = Digest(r.array()?);
    let root = Digest(r.array()?);
    let next_committee = Digest(r.array()?);
    let mut state = State::new(genesis.clone())?;
    // Retain borrowed record slices, not a second copy of the archive. No record
    // is installed until all certificates, transitions and framing have passed.
    let mut records = Vec::with_capacity(height as usize);
    for _ in 0..height {
        let bytes = r.bytes(MAX_RECORD_BYTES)?;
        let record = FinalizedRecord::decode(bytes)?;
        verify_record(genesis, &record)?;
        state = state.execute(&record.block)?;
        records.push(bytes);
    }
    r.end()?;
    if state.height() != height
        || state.tip() != tip
        || state.root() != root
        || genesis.committee_at(height + 1)?.hash() != next_committee
    {
        return Err(eyre!("snapshot terminal state mismatch"));
    }
    let existing = journal.replay(genesis.clone(), |record| verify_record(genesis, record))?;
    if existing.height() > height {
        return Err(eyre!("snapshot would roll back local history"));
    }
    for h in 1..=existing.height() {
        let local = journal
            .record(h)?
            .ok_or_else(|| eyre!("snapshot local history gap"))?;
        let incoming = FinalizedRecord::decode(records[(h - 1) as usize])?;
        // Different quorum subsets can certify the same block. Both certificates
        // were verified above; keep the local bytes/receipt rather than requiring
        // identical certificate aggregation or overwriting an immutable record.
        if local.block != incoming.block {
            return Err(eyre!("snapshot conflicts with local history"));
        }
    }
    for bytes in records.into_iter().skip(existing.height() as usize) {
        journal.append(&FinalizedRecord::decode(bytes)?)?;
    }
    Ok(state)
}
