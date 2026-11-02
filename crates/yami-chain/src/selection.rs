//! Inactive interfaces for a future reviewed PoS selector. No live consumer,
//! randomness implementation, backing verifier, money or election is provided.
use crate::{codec::Writer, hash, Digest, Error, Result, ValidatorSet};

/// Output of future authenticated stake/delegation accounting. `power` is an
/// eligibility weight, not a balance. Construction does not authenticate backing.
#[derive(Clone, Debug)]
pub struct EligibilitySnapshot {
    pub epoch: u64,
    pub activation_height: u64,
    pub parent_checkpoint: Digest,
    pub accounting_root: Digest,
    pub eligibility_root: Digest,
    pub eligible_count: u64,
    pub total_weight: u64,
    pub committee_size: u16,
}
impl EligibilitySnapshot {
    pub fn commitment(&self) -> Result<Digest> {
        if self.epoch == 0
            || self.activation_height == 0
            || self.total_weight == 0
            || self.committee_size == 0
            || self.committee_size > 100
            || self.eligible_count < u64::from(self.committee_size)
        {
            return Err(Error::Genesis);
        }
        let mut w = Writer::default();
        w.u64(self.epoch);
        w.u64(self.activation_height);
        w.raw(&self.parent_checkpoint.0);
        w.raw(&self.accounting_root.0);
        w.raw(&self.eligibility_root.0);
        w.u64(self.eligible_count);
        w.u64(self.total_weight);
        w.u16(self.committee_size);
        Ok(hash(b"YC-INACTIVE-ELIGIBILITY-1", &w.0))
    }
}
/// A provider must verify registration/key possession, unique shielded backing,
/// delegation non-duplication, unbonding and slashing before producing weights.
/// No implementation is connected to the node or payment boundary.
pub trait EligibilityProvider {
    type MembershipWitness;
    /// Candidate paging must authenticate membership under eligibility_root.
    /// No proof format or accepting verifier is selected by this interface.
    fn candidate_at(
        &self,
        snapshot: &EligibilitySnapshot,
        index: u64,
    ) -> Result<(crate::Member, Self::MembershipWitness)>;
    fn eligibility_at(
        &self,
        epoch: u64,
        activation_height: u64,
        parent: Digest,
    ) -> Result<EligibilitySnapshot>;
}
/// A future implementation needs separately approved unbiased randomness,
/// grinding/adaptive-corruption analysis, sampling rules and activation proofs.
pub trait CommitteeSelector {
    fn select(
        &self,
        snapshot: &EligibilitySnapshot,
        randomness_commitment: Digest,
    ) -> Result<ValidatorSet>;
}
pub struct ElectionsDisabled;
impl CommitteeSelector for ElectionsDisabled {
    fn select(&self, _: &EligibilitySnapshot, _: Digest) -> Result<ValidatorSet> {
        Err(Error::Unsupported)
    }
}
/// An unauthenticated proposal for review/transport only; cannot change Genesis.
/// A future activation verifier must authenticate its parent and selection proof.
pub struct EpochActivationProposal {
    pub eligibility_commitment: Digest,
    pub selected: ValidatorSet,
    pub randomness_commitment: Digest,
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::Genesis;
    #[test]
    fn domain_binds_epoch_height_backing_parent_and_weights_but_never_elects() {
        let genesis = Genesis::local();
        let original = EligibilitySnapshot {
            epoch: 1,
            activation_height: 2,
            parent_checkpoint: Digest([1; 32]),
            accounting_root: Digest([2; 32]),
            eligibility_root: genesis.committee_at(1).unwrap().hash(),
            eligible_count: 1000,
            total_weight: 1000,
            committee_size: 25,
        };
        let digest = original.commitment().unwrap();
        for field in 0..8 {
            let mut changed = original.clone();
            match field {
                0 => changed.epoch += 1,
                1 => changed.activation_height += 1,
                2 => changed.parent_checkpoint.0[0] ^= 1,
                3 => changed.accounting_root.0[0] ^= 1,
                4 => changed.total_weight += 1,
                5 => changed.eligibility_root.0[0] ^= 1,
                6 => changed.eligible_count += 1,
                _ => changed.committee_size += 1,
            }
            assert_ne!(digest, changed.commitment().unwrap());
            assert_eq!(
                ElectionsDisabled.select(&changed, Digest([9; 32])),
                Err(Error::Unsupported)
            );
        }
        assert_eq!(genesis.id(), Genesis::local().id());
    }
}
