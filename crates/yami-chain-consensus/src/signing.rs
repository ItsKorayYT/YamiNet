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

fn prefix(genesis: &Genesis, height: u64, step: u8) -> yami_chain::Result<Writer> {
    let mut w = Writer::default();
    w.raw(b"YC-CONSENSUS-2");
    w.raw(&genesis.id().0);
    w.raw(&genesis.committee_at(height)?.hash().0);
    w.u8(step);
    Ok(w)
}
pub(crate) fn vote_preimage_for(genesis: &Genesis, v: &Vote) -> yami_chain::Result<Vec<u8>> {
    let mut w = prefix(genesis, v.height.0, 1)?;
    put_vote(&mut w, v)?;
    Ok(w.0)
}
pub(crate) fn proposal_preimage_for(
    genesis: &Genesis,
    p: &Proposal,
) -> yami_chain::Result<Vec<u8>> {
    let mut w = prefix(genesis, p.height.0, 2)?;
    put_proposal(&mut w, p);
    Ok(w.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrashPoint {
    Proposal,
    Vote,
    BeforeCommit,
    BeforeEpoch,
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
#[derive(Clone)]
pub(crate) struct DurableSigner {
    pub key: SigningKey,
    pub genesis: Genesis,
    pub journal: Arc<Journal>,
    pub crash: Option<CrashPoint>,
}
pub(crate) struct SignatureVerifier(pub Genesis);
fn transport_peer(genesis: &Genesis, key: &[u8; 32]) -> Option<Vec<u8>> {
    #[cfg(feature = "insecure-devnet-input")]
    if let Some(transport) = genesis.live_transport(key) {
        let public = libp2p_identity::ed25519::PublicKey::try_from_bytes(&transport).ok()?;
        return Some(
            libp2p_identity::PublicKey::from(public)
                .to_peer_id()
                .to_bytes(),
        );
    }
    let member = genesis
        .epochs
        .iter()
        .find_map(|epoch| epoch.validators.member(key))?;
    let public = libp2p_identity::ed25519::PublicKey::try_from_bytes(&member.transport_key).ok()?;
    Some(
        libp2p_identity::PublicKey::from(public)
            .to_peer_id()
            .to_bytes(),
    )
}
#[async_trait]
impl Signer<Context> for DurableSigner {
    async fn sign_vote(&self, vote: Vote) -> Result<m::SignedMessage<Context, Vote>, Error> {
        if vote.address.0 .0 != self.key.verifying_key().to_bytes()
            || vote.height.0 == 0
            || !self
                .genesis
                .committee_at(vote.height.0)
                .is_ok_and(|s| s.member(&vote.address.0 .0).is_some())
        {
            return Err(Error::new());
        }
        let preimage = vote_preimage_for(&self.genesis, &vote).map_err(Error::from_source)?;
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
        use m::Context as _;
        if Context
            .select_proposer(
                &Committee::at(&self.genesis, proposal.height.0).map_err(Error::from_source)?,
                proposal.height,
                proposal.round,
            )
            .address
            != proposal.address
        {
            return Err(Error::new());
        }
        let preimage =
            proposal_preimage_for(&self.genesis, &proposal).map_err(Error::from_source)?;
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
        if public_key != self.key.verifying_key().to_bytes()
            || peer_id.len() > 128
            || !valid_peer_binding(
                &self.genesis,
                &self.key.verifying_key().to_bytes(),
                &peer_id,
            )
        {
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
        let Ok(bytes) = vote_preimage_for(&self.0, vote) else {
            return Ok(VerificationResult::Invalid);
        };
        Ok(VerificationResult::from_bool(
            self.0
                .committee_at(vote.height.0)
                .is_ok_and(|s| s.member(key).is_some())
                && vote.address.0 .0 == *key
                && yami_chain::verify(key, &bytes, sig).is_ok(),
        ))
    }
    async fn verify_signed_proposal(
        &self,
        p: &Proposal,
        sig: &[u8; 64],
        key: &[u8; 32],
    ) -> Result<VerificationResult, Error> {
        Ok(VerificationResult::from_bool(
            self.0
                .committee_at(p.height.0)
                .is_ok_and(|s| s.member(key).is_some())
                && p.address.0 .0 == *key
                && yami_chain::verify(
                    key,
                    &proposal_preimage_for(&self.0, p).map_err(Error::from_source)?,
                    sig,
                )
                .is_ok(),
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
            valid_peer_binding(&self.0, &key, &p.peer_id)
                && yami_chain::verify(&key, &p.preimage(), &p.signature).is_ok(),
        ))
    }
}

// A finalized identity ALWAYS uses its finalized transport key. Permissionless
// candidates may authenticate and synchronize before selection, but cannot borrow
// a reserved key/PeerId and acquire no voting power from this proof.
fn valid_peer_binding(genesis: &Genesis, key: &[u8; 32], peer_id: &[u8]) -> bool {
    if let Some(expected) = transport_peer(genesis, key) {
        return expected == peer_id;
    }
    #[cfg(feature = "insecure-devnet-input")]
    if genesis.live_scope().is_some() && !genesis.is_reserved_key(key) {
        let Ok(peer) = libp2p_identity::PeerId::from_bytes(peer_id) else {
            return false;
        };
        let multihash = peer.as_ref();
        if multihash.code() != 0 {
            return false;
        }
        let Ok(public) = libp2p_identity::PublicKey::try_decode_protobuf(multihash.digest()) else {
            return false;
        };
        let Ok(ed) = public.try_into_ed25519() else {
            return false;
        };
        let transport = ed.to_bytes();
        return transport != *key && !genesis.is_reserved_key(&transport);
    }
    false
}
