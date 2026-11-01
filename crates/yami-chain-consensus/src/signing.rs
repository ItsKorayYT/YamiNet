use crate::{
    types::*,
    wire::{put_proposal, put_vote},
};
use async_trait::async_trait;
use bytes::Bytes;
use malachite::app::types::core as m;
use malachite_signing::{Error, Signer, VerificationResult, Verifier};
use std::sync::Arc;
use yami_chain::{codec::Writer, Genesis, Signer as _, SigningKey};
use yami_chain_storage::Journal;

fn prefix(step: u8) -> Writer {
    let mut w = Writer::default();
    w.raw(b"YC-CONSENSUS-1");
    w.raw(&Genesis::local().id().0);
    w.raw(&Genesis::local().committee_hash().0);
    w.u8(step);
    w
}
pub(crate) fn vote_preimage(v: &Vote) -> yami_chain::Result<Vec<u8>> {
    let mut w = prefix(1);
    put_vote(&mut w, v)?;
    Ok(w.0)
}
fn proposal_preimage(p: &Proposal) -> Vec<u8> {
    let mut w = prefix(2);
    put_proposal(&mut w, p);
    w.0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrashPoint {
    Proposal,
    Vote,
    BeforeCommit,
}
pub(crate) fn maybe_crash(
    journal: &Journal,
    point: Option<CrashPoint>,
    at: CrashPoint,
) -> std::io::Result<()> {
    if point != Some(at) {
        return Ok(());
    }
    let name = format!("crash-fired-{at:?}.bin");
    if !journal.path().join(&name).exists() {
        journal.write_once(&name, b"intentional local crash injection")?;
        std::process::exit(86);
    }
    Ok(())
}
// Deliberately no Debug/Serialize: this object contains a consensus signing key.
pub(crate) struct DurableSigner {
    pub key: SigningKey,
    pub journal: Arc<Journal>,
    pub crash: Option<CrashPoint>,
}
pub(crate) struct SignatureVerifier;
#[async_trait]
impl Signer<Context> for DurableSigner {
    async fn sign_vote(&self, vote: Vote) -> Result<m::SignedMessage<Context, Vote>, Error> {
        if vote.address.0 .0 != self.key.verifying_key().to_bytes() || vote.height.0 == 0 {
            return Err(Error::new());
        }
        let preimage = vote_preimage(&vote).map_err(Error::from_source)?;
        let round = vote.round.as_u32().ok_or_else(Error::new)?;
        let step = match vote.step {
            m::VoteType::Prevote => 1,
            m::VoteType::Precommit => 2,
        };
        self.journal
            .reserve_signature(vote.height.0, round, step, &preimage)
            .map_err(Error::from_source)?;
        let signature = self.key.sign(&preimage).to_bytes();
        if step == 2 {
            maybe_crash(&self.journal, self.crash, CrashPoint::Vote).map_err(Error::from_source)?;
        }
        Ok(m::SignedMessage::new(vote, signature))
    }
    async fn sign_proposal(
        &self,
        proposal: Proposal,
    ) -> Result<m::SignedMessage<Context, Proposal>, Error> {
        if proposal.address.0 .0 != self.key.verifying_key().to_bytes() || proposal.height.0 == 0 {
            return Err(Error::new());
        }
        let preimage = proposal_preimage(&proposal);
        let round = proposal.round.as_u32().ok_or_else(Error::new)?;
        self.journal
            .reserve_signature(proposal.height.0, round, 0, &preimage)
            .map_err(Error::from_source)?;
        let signature = self.key.sign(&preimage).to_bytes();
        maybe_crash(&self.journal, self.crash, CrashPoint::Proposal).map_err(Error::from_source)?;
        Ok(m::SignedMessage::new(proposal, signature))
    }
    async fn sign_vote_extension(
        &self,
        _: m::VoteExtensionScope<Context>,
        _: Bytes,
    ) -> Result<m::SignedMessage<Context, Bytes>, Error> {
        Err(Error::new())
    }
    async fn sign_validator_proof(
        &self,
        public_key: Vec<u8>,
        peer_id: Vec<u8>,
    ) -> Result<m::ValidatorProof<Context>, Error> {
        if public_key != self.key.verifying_key().to_bytes() || peer_id.len() > 128 {
            return Err(Error::new());
        }
        let bytes = m::ValidatorProof::<Context>::signing_bytes(&public_key, &peer_id);
        Ok(m::ValidatorProof::new(
            public_key,
            peer_id,
            self.key.sign(&bytes).to_bytes(),
        ))
    }
}
#[async_trait]
impl Verifier<Context> for SignatureVerifier {
    async fn verify_signed_vote(
        &self,
        vote: &Vote,
        sig: &[u8; 64],
        key: &[u8; 32],
    ) -> Result<VerificationResult, Error> {
        let bytes = vote_preimage(vote).map_err(Error::from_source)?;
        Ok(VerificationResult::from_bool(
            vote.address.0 .0 == *key && yami_chain::verify(key, &bytes, sig).is_ok(),
        ))
    }
    async fn verify_signed_proposal(
        &self,
        p: &Proposal,
        sig: &[u8; 64],
        key: &[u8; 32],
    ) -> Result<VerificationResult, Error> {
        Ok(VerificationResult::from_bool(
            p.address.0 .0 == *key && yami_chain::verify(key, &proposal_preimage(p), sig).is_ok(),
        ))
    }
    async fn verify_signed_vote_extension(
        &self,
        _: &m::VoteExtensionScope<Context>,
        _: &Bytes,
        _: &[u8; 64],
        _: &[u8; 32],
    ) -> Result<VerificationResult, Error> {
        Ok(VerificationResult::Invalid)
    }
    async fn verify_validator_proof(
        &self,
        p: &m::ValidatorProof<Context>,
    ) -> Result<VerificationResult, Error> {
        let Ok(key) = <[u8; 32]>::try_from(p.public_key.as_slice()) else {
            return Ok(VerificationResult::Invalid);
        };
        Ok(VerificationResult::from_bool(
            Genesis::local().validators.contains(&key)
                && yami_chain::verify(&key, &p.preimage(), &p.signature).is_ok(),
        ))
    }
}
