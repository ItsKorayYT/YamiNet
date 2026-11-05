//! Inactive root-authenticated completion. Never called by node recovery/pruning.
//! Independent minima must come from the operator's trusted chain/rollback store,
//! never from a peer, envelope, completion marker or copied generation directory.
use crate::verify_record;
use eyre::{eyre, Result};
use std::{
    io::Read,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
use yami_chain::{
    checkpoint_v2::{self as codec, InactiveState, Manifest, StateAnchor},
    codec::{Reader, Writer},
    Digest, Genesis, State,
};
use yami_chain_storage::{FinalizedRecord, MAX_RECORD_BYTES};

const DOMAIN: &[u8] = b"YC-INACTIVE-CHECKPOINT-ENVELOPE-2";
pub const MAX_ENVELOPE_BYTES: usize = MAX_RECORD_BYTES + codec::MAX_MANIFEST_BYTES + 256;
pub const MAX_ANCESTRY_RECORDS: u64 = 8192;
pub const MAX_ANCESTRY_BYTES: u64 = 128 * 1024 * 1024;
static IMPORTING: AtomicBool = AtomicBool::new(false);
struct Admission;
impl Admission {
    fn acquire() -> Result<Self> {
        IMPORTING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| eyre!("inactive V2 import already active"))?;
        Ok(Self)
    }
}
impl Drop for Admission {
    fn drop(&mut self) {
        IMPORTING.store(false, Ordering::Release);
    }
}
/// Inspection result only: cannot seed consensus, install a compact base, prune,
/// update rollback minima, or restore a signing identity.
pub struct VerifiedContext {
    genesis: Genesis,
    manifest: Manifest,
}
impl VerifiedContext {
    pub fn anchor(&self) -> StateAnchor {
        self.manifest.anchor().state
    }
    /// Bounded framing reader. Input must already have a deadline and a finite
    /// EOF; this synchronous API does not implement network/peer scheduling.
    pub fn complete<R: Read>(
        &self,
        open: impl FnMut(u32) -> std::io::Result<R>,
    ) -> Result<InactiveState> {
        let _admission = Admission::acquire()?;
        Ok(codec::import(
            self.genesis.clone(),
            self.manifest.anchor(),
            &self.manifest,
            open,
        )?)
    }
    /// Dedicated immutable generation only. A completion witness is published
    /// after destination readback; interruptions resume only identical prefixes.
    pub fn transfer(&self, source: &Path, destination: &Path) -> Result<InactiveState> {
        let _admission = Admission::acquire()?;
        Ok(codec::store::transfer(
            source,
            destination,
            self.genesis.clone(),
            self.manifest.anchor(),
        )?)
    }
    /// Rechecks every invariant and the canonical root. Local metadata supplies
    /// no finality or rollback authority: this context must be verified anew.
    pub fn recover(&self, path: &Path) -> Result<InactiveState> {
        let _admission = Admission::acquire()?;
        Ok(codec::store::recover(
            path,
            self.genesis.clone(),
            self.manifest.anchor(),
        )?)
    }
}
/// Inactive operator fixture envelope. No production writer/installation exists.
/// The manifest commitment is framing only, regardless of who built this envelope.
pub fn envelope(
    genesis: &Genesis,
    record: &FinalizedRecord,
    manifest: &Manifest,
) -> Result<Vec<u8>> {
    verify_record(genesis, record)?;
    let expected = StateAnchor {
        genesis: genesis.id(),
        height: record.block.header.height,
        tip: record.block.id()?,
        root: record.block.header.state,
    };
    Manifest::decode_transport(&manifest.canonical(), expected)?;
    let mut w = Writer::default();
    w.raw(DOMAIN);
    w.raw(&genesis.id().0);
    w.raw(&genesis.committee_at(expected.height + 1)?.hash().0);
    w.bytes(&record.canonical()?)?;
    w.bytes(&manifest.canonical())?;
    if w.0.len() > MAX_ENVELOPE_BYTES {
        return Err(eyre!("V2 envelope budget"));
    }
    Ok(w.0)
}
/// Verify finality BEFORE opening any state chunks. The caller pins genesis and
/// an independently retained exact minimum. All intervening certified records
/// must link consecutively to that minimum; a valid certificate alone is not
/// ancestry. The target may equal the minimum only with exact tip/root equality.
/// This authenticates the header root, never the peer's manifest commitment.
/// Byte slices are streamed by the caller; no ancestry vector is allocated here.
pub fn verify_context<B: AsRef<[u8]>>(
    genesis: &Genesis,
    minimum: StateAnchor,
    bytes: &[u8],
    ancestry: impl IntoIterator<Item = B>,
) -> Result<VerifiedContext> {
    let _admission = Admission::acquire()?;
    genesis.validate()?;
    if minimum.genesis != genesis.id() || minimum.height >= 1_000_000 {
        return Err(eyre!("V2 independent minimum genesis/height"));
    }
    if minimum.height == 0 {
        let initial = State::new(genesis.clone())?;
        if minimum.tip != initial.tip() || minimum.root != initial.root() {
            return Err(eyre!("V2 genesis minimum"));
        }
    }
    let mut r = Reader::new(bytes, MAX_ENVELOPE_BYTES)?;
    if r.take(DOMAIN.len())? != DOMAIN || r.array::<32>()? != genesis.id().0 {
        return Err(eyre!("V2 envelope version/genesis"));
    }
    let next_committee = Digest(r.array()?);
    let terminal = FinalizedRecord::decode(r.bytes(MAX_RECORD_BYTES)?)?;
    verify_record(genesis, &terminal)?;
    let target = StateAnchor {
        genesis: genesis.id(),
        height: terminal.block.header.height,
        tip: terminal.block.id()?,
        root: terminal.block.header.state,
    };
    if target.height == 0
        || target.height >= 1_000_000
        || target.height < minimum.height
        || target.height - minimum.height > MAX_ANCESTRY_RECORDS
        || genesis.committee_at(target.height + 1)?.hash() != next_committee
    {
        return Err(eyre!("V2 rollback/ancestry budget/next committee"));
    }
    let mut height = minimum.height;
    let mut tip = minimum.tip;
    let mut root = minimum.root;
    let mut remaining = MAX_ANCESTRY_BYTES;
    for encoded in ancestry {
        let encoded = encoded.as_ref();
        // Reject an extra iterator element before decoding or certificate work.
        if height >= target.height {
            return Err(eyre!("V2 excess ancestry"));
        }
        remaining = remaining
            .checked_sub(encoded.len() as u64)
            .ok_or_else(|| eyre!("V2 ancestry byte budget"))?;
        let record = FinalizedRecord::decode(encoded)?;
        verify_record(genesis, &record)?;
        if record.block.header.height != height + 1 || record.block.header.parent != tip {
            return Err(eyre!("V2 ancestry gap/conflict"));
        }
        height += 1;
        tip = record.block.id()?;
        root = record.block.header.state;
    }
    if (height, tip, root) != (target.height, target.tip, target.root) {
        return Err(eyre!("V2 minimum/terminal ancestry mismatch"));
    }
    let manifest = Manifest::decode_transport(r.bytes(codec::MAX_MANIFEST_BYTES)?, target)?;
    r.end()?;
    Ok(VerifiedContext {
        genesis: genesis.clone(),
        manifest,
    })
}

#[cfg(test)]
mod tests;
