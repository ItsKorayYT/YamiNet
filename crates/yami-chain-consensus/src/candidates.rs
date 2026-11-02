//! Disk-backed payload retention. No global candidate quota can stop consensus.
//! Only the active round's authenticated proposer may allocate one payload. All
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
}
impl Candidates {
    pub fn new(journal: Arc<Journal>) -> Self {
        Self { journal }
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
    fn valid(state: &State, part: &Part) -> bool {
        Self::authenticated(state, part) && state.execute(&part.block).is_ok()
    }
    pub fn admit(&mut self, state: &State, active_round: m::Round, part: &Part) -> Result<bool> {
        if part.round != active_round || part.round.is_nil() || part.height.0 != state.height() + 1
        {
            return Ok(false);
        }
        let name = Self::name(part.height.0, part.round)?;
        if let Some(bytes) = self.read(&name, MAX_RECORD_BYTES)? {
            let original: Part = WireCodec::new(state.genesis()).decode(bytes.into())?;
            if original.height != part.height || original.round != part.round {
                return Err(eyre!("proposal filename mismatch"));
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
        if !Self::valid(state, part) {
            return Ok(false);
        }
        self.store_block(&part.block)?;
        self.journal
            .write_once(&name, &WireCodec::new(state.genesis()).encode(part)?)?;
        Ok(true)
    }
    pub fn recover(&self, state: &State, round: m::Round) -> Result<Option<Part>> {
        self.read(&Self::name(state.height() + 1, round)?, MAX_RECORD_BYTES)?
            .map(|bytes| {
                let part: Part = WireCodec::new(state.genesis()).decode(bytes.into())?;
                if part.round != round || !Self::valid(state, &part) {
                    return Err(eyre!("invalid recovered proposal"));
                }
                Ok(part)
            })
            .transpose()
    }
    pub fn prune_finalized(&self, height: u64) -> Result<()> {
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
            let future = signed(&state, m::Round::new(n + 1), "test-future");
            assert!(!c.admit(&state, round, &future).unwrap());
        }
        // One accepted payload, its proposal and one complete evidence pair.
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 4);
        let mut forged = original.clone();
        forged.signature[0] ^= 1;
        assert!(!c.admit(&state, round, &forged).unwrap());
        let honest = signed(&state, m::Round::new(1), "test-honest");
        assert!(c.admit(&state, honest.round, &honest).unwrap());
        drop(c);
        let c = Candidates::new(Arc::new(Journal::open(&path).unwrap()));
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
        }
        drop(candidates);
        let candidates = Candidates::new(Arc::new(Journal::open(&path).unwrap()));
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
