//! Disk-backed payload retention. No global candidate quota can stop consensus.
//! Each authenticated proposer/round in Malachite's bounded lookahead may allocate one payload. All
//! acknowledged values remain available until finality, including across restart.
use crate::{signing::proposal_preimage_for, types::*, wire::WireCodec};
use eyre::{eyre, Result};
use m::Context as _;
use malachite::app::types::{codec::Codec, core as m};
use malachite_core::SignedConsensusMsg;
use std::{io, sync::Arc};
use yami_chain::codec::Writer;
use yami_chain::{Block, Digest, State, MAX_BLOCK_BYTES};
use yami_chain_storage::{read_bounded, Journal, MAX_RECORD_BYTES};

impl Part {
    pub(crate) fn proposal(&self) -> yami_chain::Result<Proposal> {
        Ok(Proposal {
            height: self.height,
            round: self.round,
            value: Value(self.block.id()?),
            pol_round: self.valid_round,
            address: self.proposer,
        })
    }
}
pub(crate) struct Candidates {
    journal: Arc<Journal>,
    validated: Option<Validated>,
}
struct Validated {
    parent: (Digest, u64, Digest, Digest),
    block: Block,
    next: State,
}
impl Candidates {
    pub fn new(journal: Arc<Journal>) -> Self {
        Self {
            journal,
            validated: None,
        }
    }
    fn name(height: u64, round: m::Round) -> Result<String> {
        Ok(format!(
            "proposal-{height:020}-{:010}.bin",
            round.as_u32().ok_or_else(|| eyre!("undefined round"))?
        ))
    }
    fn block_name(height: u64, id: Digest) -> String {
        format!("candidate-{height:020}-{id}.bin")
    }
    fn read(&self, name: &str, cap: usize) -> Result<Option<Vec<u8>>> {
        match read_bounded(&self.journal.path().join(name), cap) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    pub fn store_block(&self, block: &Block) -> Result<()> {
        self.journal.write_once(
            &Self::block_name(block.header.height, block.id()?),
            &block.canonical()?,
        )?;
        Ok(())
    }
    pub fn block(&self, height: u64, id: Digest) -> Result<Option<Block>> {
        self.read(&Self::block_name(height, id), MAX_BLOCK_BYTES)?
            .map(|bytes| {
                let block = Block::decode(&bytes)?;
                if block.header.height != height || block.id()? != id {
                    return Err(eyre!("candidate identity mismatch"));
                }
                Ok(block)
            })
            .transpose()
    }
    fn decode_proposal(&self, state: &State, bytes: Vec<u8>) -> Result<Part> {
        if let Some(metadata) = bytes.strip_prefix(b"YC-CACHED-2") {
            let message: SignedConsensusMsg<Context> =
                WireCodec::new(state.genesis()).decode(bytes::Bytes::copy_from_slice(metadata))?;
            let SignedConsensusMsg::Proposal(signed) = message else {
                return Err(eyre!("cached proposal message type"));
            };
            let p = signed.message;
            let block = self
                .block(p.height.0, p.value.0)?
                .ok_or_else(|| eyre!("cached payload missing"))?;
            Ok(Part {
                height: p.height,
                round: p.round,
                valid_round: p.pol_round,
                proposer: p.address,
                block,
                signature: signed.signature,
            })
        } else {
            // Preserve stores created by PR #15; no unsafe rewrite/migration.
            Ok(WireCodec::new(state.genesis()).decode(bytes.into())?)
        }
    }
    fn authenticated(state: &State, part: &Part) -> bool {
        part.height.0 == state.height() + 1
            && part.block.header.height == part.height.0
            && part.round.is_defined()
            && (part.valid_round.is_nil() || part.valid_round < part.round)
            && Context
                .select_proposer(
                    &Committee::at(state.genesis(), part.height.0).expect("validated genesis"),
                    part.height,
                    part.round,
                )
                .address
                == part.proposer
            && part
                .proposal()
                .and_then(|p| {
                    yami_chain::verify(
                        &part.proposer.0 .0,
                        &proposal_preimage_for(state.genesis(), &p)?,
                        &part.signature,
                    )
                })
                .is_ok()
    }
    // A single volatile entry is bounded and cannot authorize disk/network data.
    // Authentication still runs on every proposal. Exact parent and complete block
    // equality are required. Restart starts empty; disk-loaded blocks must execute
    // successfully before they can populate this cache.
    pub fn execute(&mut self, state: &State, block: &Block) -> yami_chain::Result<State> {
        let parent = (
            state.genesis().id(),
            state.height(),
            state.tip(),
            state.root(),
        );
        if let Some(v) = &self.validated {
            if v.parent == parent && v.block == *block {
                return Ok(v.next.clone());
            }
        }
        let next = state.execute(block)?;
        self.validated = Some(Validated {
            parent,
            block: block.clone(),
            next: next.clone(),
        });
        Ok(next)
    }
    fn valid(&mut self, state: &State, part: &Part) -> bool {
        Self::authenticated(state, part) && self.execute(state, &part.block).is_ok()
    }
    pub fn admit(&mut self, state: &State, active_round: m::Round, part: &Part) -> Result<bool> {
        let current = active_round.as_u32().unwrap_or(0);
        let Some(round) = part.round.as_u32() else {
            return Ok(false);
        };
        // A proposal/part may arrive before this node starts its round. Match
        // Malachite's existing future-round bound; exact-round filtering loses
        // honest builders after delays/partitions and can prevent recovery.
        if round < current
            || round > current.saturating_add(malachite_core::MAX_FUTURE_ROUND_LOOKAHEAD)
            || part.height.0 != state.height() + 1
        {
            return Ok(false);
        }
        let name = Self::name(part.height.0, part.round)?;
        if let Some(bytes) = self.read(&name, MAX_RECORD_BYTES)? {
            let original = self.decode_proposal(state, bytes)?;
            if original.height != part.height || original.round != part.round {
                return Err(eyre!("proposal filename mismatch"));
            }
            // Disk cache membership is not authentication. Recheck even an
            // identical retransmission: corrupted metadata/payloads must never
            // become a Valid proposal or an equivocation evidence source.
            if !self.valid(state, &original) {
                return Err(eyre!("invalid cached proposal"));
            }
            if original != *part {
                // Keep one independently verifiable equivocation pair per round;
                // additional equivocations consume no further storage.
                let evidence = name.replacen("proposal-", "equivocation-", 1);
                if self.read(&evidence, MAX_RECORD_BYTES)?.is_none()
                    && Self::authenticated(state, part)
                    && original.proposal()? != part.proposal()?
                {
                    let mut pair = Writer::default();
                    for p in [&original, part] {
                        pair.bytes(&WireCodec::new(state.genesis()).encode(
                            &SignedConsensusMsg::Proposal(m::SignedMessage::new(
                                p.proposal()?,
                                p.signature,
                            )),
                        )?)?;
                    }
                    self.journal.write_once(&evidence, &pair.0)?;
                }
                return Ok(false);
            }
            return Ok(true);
        }
        if !self.valid(state, part) {
            return Ok(false);
        }
        self.store_block(&part.block)?;
        let mut metadata = b"YC-CACHED-2".to_vec();
        metadata.extend_from_slice(&WireCodec::new(state.genesis()).encode(
            &SignedConsensusMsg::Proposal(m::SignedMessage::new(part.proposal()?, part.signature)),
        )?);
        self.journal.write_once(&name, &metadata)?;
        Ok(true)
    }
    pub fn recover(&mut self, state: &State, round: m::Round) -> Result<Option<Part>> {
        self.read(&Self::name(state.height() + 1, round)?, MAX_RECORD_BYTES)?
            .map(|bytes| {
                let part = self.decode_proposal(state, bytes)?;
                if part.round != round || !self.valid(state, &part) {
                    return Err(eyre!("invalid recovered proposal"));
                }
                Ok(part)
            })
            .transpose()
    }
    pub fn prune_finalized(&mut self, height: u64) -> Result<()> {
        self.validated = None;
        self.journal.remove_finalized_candidates(height)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yami_chain::{development_key, Genesis, Signer as _, Transaction};
    fn signed(state: &State, round: m::Round, label: &str) -> Part {
        let proposer = Context
            .select_proposer(&Committee::local(), Height(1), round)
            .address;
        let key = (0..4)
            .map(|i| development_key("consensus", i))
            .find(|k| k.verifying_key().to_bytes() == proposer.0 .0)
            .unwrap();
        let block = state
            .propose(vec![Transaction::sign(
                label,
                0,
                10,
                &development_key("registry", 1),
            )
            .unwrap()])
            .unwrap();
        let mut part = Part {
            height: Height(1),
            round,
            valid_round: m::Round::Nil,
            proposer,
            block,
            signature: [0; 64],
        };
        part.signature = key
            .sign(&proposal_preimage_for(state.genesis(), &part.proposal().unwrap()).unwrap())
            .to_bytes();
        part
    }
    #[test]
    fn authenticated_flood_is_bounded_and_next_round_progresses_after_restart() {
        let path = std::env::temp_dir().join(format!("yami-flood-{}", std::process::id()));
        let state = State::new(Genesis::local()).unwrap();
        let mut c = Candidates::new(Arc::new(Journal::open(&path).unwrap()));
        let round = m::Round::ZERO;
        let original = signed(&state, round, "test-first");
        assert!(c.admit(&state, round, &original).unwrap());
        for n in 0..256 {
            let conflicting = signed(&state, round, &format!("test-flood-{n}"));
            assert!(!c.admit(&state, round, &conflicting).unwrap());
            let future = signed(
                &state,
                m::Round::new(n + 1 + malachite_core::MAX_FUTURE_ROUND_LOOKAHEAD),
                "test-future",
            );
            assert!(!c.admit(&state, round, &future).unwrap());
        }
        // One accepted payload, its proposal and one complete evidence pair.
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 4);
        let mut forged = original.clone();
        forged.signature[0] ^= 1;
        assert!(!c.admit(&state, round, &forged).unwrap());
        let honest = signed(&state, m::Round::new(1), "test-honest");
        assert!(c.admit(&state, round, &honest).unwrap()); // arrives before StartedRound(1)
        drop(c);
        let mut c = Candidates::new(Arc::new(Journal::open(&path).unwrap()));
        assert_eq!(c.recover(&state, round).unwrap(), Some(original.clone()));
        assert_eq!(c.recover(&state, honest.round).unwrap(), Some(honest));
        assert_eq!(
            c.block(1, original.block.id().unwrap()).unwrap(),
            Some(original.block)
        );
        c.prune_finalized(0).unwrap();
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 6);
        c.prune_finalized(1).unwrap();
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 2); // lock and evidence
        drop(c);
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn volatile_execution_cache_is_bound_to_exact_parent_and_block_and_dies_on_restart() {
        let path = std::env::temp_dir().join(format!("yami-execute-cache-{}", std::process::id()));
        let journal = Arc::new(Journal::open(&path).unwrap());
        let state = State::new(Genesis::local()).unwrap();
        let mut c = Candidates::new(journal.clone());
        let block = state.development_workload_block(64).unwrap();
        let next = c.execute(&state, &block).unwrap();
        assert_eq!(next, state.execute(&block).unwrap());
        assert_eq!(c.execute(&state.clone(), &block).unwrap(), next);
        let child = next.propose(Vec::new()).unwrap();
        assert_eq!(
            c.execute(&next, &child).unwrap(),
            next.execute(&child).unwrap()
        );
        let alternate = state
            .execute(&state.development_workload_block(63).unwrap())
            .unwrap();
        assert!(c.execute(&alternate, &child).is_err());
        let mut corrupt = child.clone();
        corrupt.header.state.0[0] ^= 1;
        assert!(c.execute(&next, &corrupt).is_err());
        assert_eq!(
            c.execute(&next, &child).unwrap(),
            next.execute(&child).unwrap()
        );
        let genesis = Genesis::development_schedule(&[(1, 5)]).unwrap();
        let other = State::new(genesis).unwrap();
        assert!(c.execute(&other, &block).is_err());
        c.prune_finalized(next.height()).unwrap();
        assert!(c.validated.is_none());
        drop(c);
        let c = Candidates::new(journal.clone());
        assert!(c.validated.is_none());
        assert_eq!(state.names().len(), 0);
        drop(c);
        drop(journal);
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn corrupted_cache_cannot_validate_retransmissions_or_create_evidence() {
        for attack in 0..3 {
            let path = std::env::temp_dir().join(format!(
                "yami-cache-corrupt-{}-{attack}",
                std::process::id()
            ));
            let state = State::new(Genesis::local()).unwrap();
            let journal = Arc::new(Journal::open(&path).unwrap());
            let mut c = Candidates::new(journal.clone());
            let original = signed(&state, m::Round::ZERO, "test-authenticated");
            assert!(c.admit(&state, original.round, &original).unwrap());
            assert!(c.admit(&state, original.round, &original).unwrap());
            let mut corrupt = original.clone();
            match attack {
                0 => corrupt.signature[0] ^= 1,
                1 => corrupt.proposer = Address(Digest([99; 32])),
                _ => {
                    corrupt.block.header.state = Digest([99; 32]);
                    let key = (0..4)
                        .map(|i| development_key("consensus", i))
                        .find(|k| k.verifying_key().to_bytes() == corrupt.proposer.0 .0)
                        .unwrap();
                    corrupt.signature = key
                        .sign(
                            &proposal_preimage_for(state.genesis(), &corrupt.proposal().unwrap())
                                .unwrap(),
                        )
                        .to_bytes();
                    c.store_block(&corrupt.block).unwrap();
                }
            }
            // Model disk corruption using the supported legacy cache encoding.
            let name = Candidates::name(1, original.round).unwrap();
            std::fs::write(
                path.join(name),
                WireCodec::new(state.genesis()).encode(&corrupt).unwrap(),
            )
            .unwrap();
            assert!(c.admit(&state, original.round, &corrupt).is_err());
            assert!(c.admit(&state, original.round, &original).is_err());
            assert!(c.recover(&state, original.round).is_err());
            assert!(!std::fs::read_dir(&path).unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("equivocation-")));
            drop(c);
            drop(journal);
            std::fs::remove_dir_all(path).unwrap();
        }
    }
}

#[cfg(test)]
mod retention_tests {
    use super::*;
    use yami_chain::{development_key, Genesis, Signer as _, Transaction};
    #[test]
    fn more_than_sixty_four_rounds_retain_acknowledged_payloads_without_a_global_quota() {
        let path = std::env::temp_dir().join(format!("yami-many-rounds-{}", std::process::id()));
        let state = State::new(Genesis::local()).unwrap();
        let mut candidates = Candidates::new(Arc::new(Journal::open(&path).unwrap()));
        let mut ids = Vec::new();
        for r in 0..80 {
            let round = m::Round::new(r);
            let proposer = Context
                .select_proposer(&Committee::local(), Height(1), round)
                .address;
            let key = (0..4)
                .map(|i| development_key("consensus", i))
                .find(|k| k.verifying_key().to_bytes() == proposer.0 .0)
                .unwrap();
            let block = state
                .propose(vec![Transaction::sign(
                    &format!("test-round-{r}"),
                    0,
                    1,
                    &development_key("registry", 3),
                )
                .unwrap()])
                .unwrap();
            let mut part = Part {
                height: Height(1),
                round,
                valid_round: m::Round::Nil,
                proposer,
                block,
                signature: [0; 64],
            };
            part.signature = key
                .sign(&proposal_preimage_for(state.genesis(), &part.proposal().unwrap()).unwrap())
                .to_bytes();
            assert!(candidates.admit(&state, round, &part).unwrap());
            ids.push(part.block.id().unwrap());
            if [15, 63, 79].contains(&r) {
                let footprint = candidates.journal.storage_footprint(1000).unwrap();
                println!("stall-candidates rounds={} state_height={} entries={} logical_bytes={} retained_candidate_bytes={}",r+1,state.height(),footprint.entries,footprint.bytes,footprint.retained_candidate_bytes);
            }
        }
        drop(candidates);
        let mut candidates = Candidates::new(Arc::new(Journal::open(&path).unwrap()));
        for (r, id) in ids.into_iter().enumerate() {
            assert!(candidates.block(1, id).unwrap().is_some());
            assert!(candidates
                .recover(&state, m::Round::new(r as u32))
                .unwrap()
                .is_some());
        }
        drop(candidates);
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[cfg(test)]
mod compact_storage_tests {
    use super::*;
    use yami_chain::{development_key, Genesis, Signer as _};
    #[test]
    fn metadata_is_bounded_and_legacy_payloads_recover_without_rewriting() {
        let path =
            std::env::temp_dir().join(format!("yami-compact-proposals-{}", std::process::id()));
        let state = State::new(Genesis::local()).unwrap();
        let journal = Arc::new(Journal::open(&path).unwrap());
        let mut candidates = Candidates::new(journal.clone());
        let make = |round| {
            let proposer = Context
                .select_proposer(&Committee::local(), Height(1), round)
                .address;
            let key = (0..4)
                .map(|i| development_key("consensus", i))
                .find(|k| k.verifying_key().to_bytes() == proposer.0 .0)
                .unwrap();
            let mut part = Part {
                height: Height(1),
                round,
                valid_round: m::Round::Nil,
                proposer,
                block: state.development_workload_block(64).unwrap(),
                signature: [0; 64],
            };
            part.signature = key
                .sign(&proposal_preimage_for(state.genesis(), &part.proposal().unwrap()).unwrap())
                .to_bytes();
            part
        };
        let legacy = make(m::Round::ZERO);
        candidates.store_block(&legacy.block).unwrap();
        let name = Candidates::name(1, legacy.round).unwrap();
        let bytes = WireCodec::new(state.genesis()).encode(&legacy).unwrap();
        journal.write_once(&name, &bytes).unwrap();
        assert_eq!(
            candidates.recover(&state, legacy.round).unwrap(),
            Some(legacy.clone())
        );
        assert!(candidates.admit(&state, legacy.round, &legacy).unwrap());
        assert_eq!(
            read_bounded(&path.join(name), MAX_RECORD_BYTES).unwrap(),
            bytes
        );
        let current = make(m::Round::new(1));
        assert!(candidates.admit(&state, current.round, &current).unwrap());
        let metadata = read_bounded(
            &path.join(Candidates::name(1, current.round).unwrap()),
            MAX_RECORD_BYTES,
        )
        .unwrap();
        assert!(metadata.starts_with(b"YC-CACHED-2"));
        assert!(metadata.len() < 256);
        assert!(current.block.canonical().unwrap().len() > 8000);
        assert_eq!(
            candidates.recover(&state, current.round).unwrap(),
            Some(current)
        );
        let edge = make(m::Round::new(malachite_core::MAX_FUTURE_ROUND_LOOKAHEAD));
        assert!(candidates.admit(&state, m::Round::Nil, &edge).unwrap());
        assert_eq!(candidates.recover(&state, edge.round).unwrap(), Some(edge));
        let beyond = make(m::Round::new(
            malachite_core::MAX_FUTURE_ROUND_LOOKAHEAD + 1,
        ));
        assert!(!candidates.admit(&state, m::Round::ZERO, &beyond).unwrap());
        drop(candidates);
        drop(journal);
        std::fs::remove_dir_all(path).unwrap();
    }
}

/// Paired laboratory profile on a genuinely replayed development-chain state.
/// This creates a NEW fixture directory and never opens a validator safety store.
/// Its public development identity must never be reused as a live validator.
#[cfg(feature = "large-state-lab")]
pub fn profile_candidate_retransmission(
    state: &State,
    directory: &std::path::Path,
) -> Result<(f64, f64)> {
    use yami_chain::{development_key, Genesis, Signer as _};
    eyre::ensure!(
        !directory.exists()
            && state.genesis()
                == &Genesis::development_schedule(
                    &state
                        .genesis()
                        .epochs
                        .iter()
                        .map(|e| (e.first_height, e.validators.members().len()))
                        .collect::<Vec<_>>()
                )?,
        "fresh development fixture required"
    );
    let journal = Arc::new(Journal::open(directory)?);
    journal.ensure_genesis(state.genesis())?;
    let mut candidates = Candidates::new(journal.clone());
    let height = Height(state.height() + 1);
    let proposer = Context
        .select_proposer(
            &Committee::at(state.genesis(), height.0)?,
            height,
            m::Round::ZERO,
        )
        .address;
    let (index, key) = (0..4)
        .map(|i| (i, development_key("consensus", i)))
        .find(|(_, k)| k.verifying_key().to_bytes() == proposer.0 .0)
        .ok_or_else(|| eyre!("fixture proposer"))?;
    journal.bind_validator_index(index)?;
    journal.migrate_signatures()?;
    let mut part = Part {
        height,
        round: m::Round::ZERO,
        valid_round: m::Round::Nil,
        proposer,
        block: state.development_distinct_workload_block(64)?,
        signature: [0; 64],
    };
    let preimage = proposal_preimage_for(state.genesis(), &part.proposal()?)?;
    journal.reserve_signature(height.0, 0, 0, &preimage)?;
    part.signature = key.sign(&preimage).to_bytes();
    eyre::ensure!(
        candidates.admit(state, part.round, &part)?,
        "initial fixture admission"
    );
    let at = std::time::Instant::now();
    for _ in 0..8 {
        candidates.validated = None;
        eyre::ensure!(
            candidates.admit(state, part.round, &part)?,
            "uncached retransmission"
        );
    }
    let uncached = at.elapsed().as_secs_f64() / 8.;
    let at = std::time::Instant::now();
    for _ in 0..8 {
        eyre::ensure!(
            candidates.admit(state, part.round, &part)?,
            "cached retransmission"
        );
    }
    let cached = at.elapsed().as_secs_f64() / 8.;
    eyre::ensure!(
        candidates.execute(state, &part.block)? == state.execute(&part.block)?,
        "cached execution oracle"
    );
    Ok((uncached, cached))
}
