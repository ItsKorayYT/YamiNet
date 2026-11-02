//! Local checkpoints authenticated by sequential execution from a pinned genesis.
//! These cannot bootstrap snapshot trust or authorize removal of certified history.
use crate::wire::{decode_certificate, verify_record};
use eyre::{eyre, Result};
use yami_chain::{codec::Writer, Digest, Genesis, State};
use yami_chain_storage::{read_bounded, Journal};
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
