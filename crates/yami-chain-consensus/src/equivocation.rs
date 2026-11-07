//! Objective evidence over the real consensus wire/signing domains. No economic side effects.
use crate::{signing::vote_preimage_for, types::*, wire};
use bytes::Bytes;
use malachite_core::SignedConsensusMsg;
use yami_chain::{Digest, Error, Genesis, Result};
const MAX_EVIDENCE_VOTE: usize = 2048;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedDoubleVote {
    offender: [u8; 32],
    height: u64,
    id: Digest,
}
impl VerifiedDoubleVote {
    pub fn offender(&self) -> [u8; 32] {
        self.offender
    }
    pub fn height(&self) -> u64 {
        self.height
    }
    pub fn id(&self) -> Digest {
        self.id
    }
}
/// Authenticate two conflicting votes at the same height/round/step under the
/// externally authenticated historical committee. A registration or peer hint
/// is never membership authority. This verifier does not authorize a slash.
pub fn verify_double_vote(
    genesis: &Genesis,
    left: &[u8],
    right: &[u8],
) -> Result<VerifiedDoubleVote> {
    if left.len() > MAX_EVIDENCE_VOTE || right.len() > MAX_EVIDENCE_VOTE {
        return Err(Error::Limit);
    }
    let a: SignedConsensusMsg<Context> = wire::decode(genesis.id(), Bytes::copy_from_slice(left))?;
    let b: SignedConsensusMsg<Context> = wire::decode(genesis.id(), Bytes::copy_from_slice(right))?;
    if wire::encode(genesis.id(), &a)?.as_ref() != left
        || wire::encode(genesis.id(), &b)?.as_ref() != right
    {
        return Err(Error::Encoding);
    }
    let (SignedConsensusMsg::Vote(a), SignedConsensusMsg::Vote(b)) = (a, b) else {
        return Err(Error::Unsupported);
    };
    let av = &a.message;
    let bv = &b.message;
    if av.height != bv.height
        || av.round != bv.round
        || av.step != bv.step
        || av.address != bv.address
        || av.value == bv.value
    {
        return Err(Error::Signature);
    }
    let offender = av.address.0 .0;
    if genesis
        .committee_at(av.height.0)?
        .member(&offender)
        .is_none()
    {
        return Err(Error::Signature);
    }
    yami_chain::verify(&offender, &vote_preimage_for(genesis, av)?, &a.signature)?;
    yami_chain::verify(&offender, &vote_preimage_for(genesis, bv)?, &b.signature)?;
    let round = av.round.as_u32().ok_or(Error::Signature)?;
    let step = match av.step {
        malachite::app::types::core::VoteType::Prevote => 1,
        malachite::app::types::core::VoteType::Precommit => 2,
    };
    // A third conflicting vote is the same offense, not a second financial penalty.
    let mut w = yami_chain::codec::Writer::default();
    w.raw(&genesis.id().0);
    w.u64(av.height.0);
    w.u32(round);
    w.u8(step);
    w.raw(&offender);

    Ok(VerifiedDoubleVote {
        offender,
        height: av.height.0,
        id: yami_chain::hash(b"YAMI-CONSENSUS-DOUBLE-VOTE-SLOT-1", &w.0),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use malachite::app::types::core as m;
    use yami_chain::{development_key, Signer as _};
    fn vote(g: &Genesis, value: u8, round: u32, step: m::VoteType, index: u8) -> Vec<u8> {
        let k = development_key("consensus", index);
        let v = Vote {
            height: Height(1),
            round: m::Round::new(round),
            value: m::NilOrVal::Val(Value(Digest([value; 32]))),
            step,
            address: Address(Digest(k.verifying_key().to_bytes())),
            extension: None,
        };
        let sig = k.sign(&vote_preimage_for(g, &v).unwrap()).to_bytes();
        wire::encode(
            g.id(),
            &SignedConsensusMsg::<Context>::Vote(m::SignedMessage::new(v, sig)),
        )
        .unwrap()
        .to_vec()
    }
    #[test]
    fn actual_signed_conflicts_verify_and_have_order_independent_identity() {
        let g = Genesis::local();
        let a = vote(&g, 1, 0, m::VoteType::Precommit, 0);
        let b = vote(&g, 2, 0, m::VoteType::Precommit, 0);
        let e = verify_double_vote(&g, &a, &b).unwrap();
        let c = vote(&g, 3, 0, m::VoteType::Precommit, 0);
        assert_eq!(e.id(), verify_double_vote(&g, &a, &c).unwrap().id());
        assert_eq!(e, verify_double_vote(&g, &b, &a).unwrap());
        assert_eq!(e.height(), 1);
        assert_eq!(
            e.offender(),
            development_key("consensus", 0).verifying_key().to_bytes()
        );
    }
    #[test]
    fn ordinary_votes_and_forged_or_cross_context_evidence_do_not_slash() {
        let g = Genesis::local();
        let a = vote(&g, 1, 0, m::VoteType::Precommit, 0);
        for b in [
            a.clone(),
            vote(&g, 2, 1, m::VoteType::Precommit, 0),
            vote(&g, 2, 0, m::VoteType::Prevote, 0),
            vote(&g, 2, 0, m::VoteType::Precommit, 1),
            vote(&g, 2, 0, m::VoteType::Precommit, 9),
        ] {
            assert!(verify_double_vote(&g, &a, &b).is_err());
        }
        let mut b = vote(&g, 2, 0, m::VoteType::Precommit, 0);
        let n = b.len();
        b[n - 1] ^= 1;
        assert!(verify_double_vote(&g, &a, &b).is_err());
        assert!(verify_double_vote(&g, &a, &vec![0; 2049]).is_err());
        let other = Genesis::development_schedule(&[(1, 5)]).unwrap();
        let b = vote(&other, 2, 0, m::VoteType::Precommit, 0);
        assert!(verify_double_vote(&g, &a, &b).is_err());
    }
}
