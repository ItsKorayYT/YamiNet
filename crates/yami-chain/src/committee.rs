//! Deterministic committees fixed by an externally pinned genesis. No election,
//! monetary stake, governance override or network-supplied membership update.
use crate::{codec::Writer, development_key, hash, Digest, Error, Result, VerifyingKey};
use std::collections::{BTreeMap, BTreeSet};
pub const MAX_VALIDATORS: usize = 100;
pub const MAX_EPOCHS: usize = 64;
pub const MAX_GENESIS_BYTES: usize = 1_048_576;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub consensus_key: [u8; 32],
    pub transport_key: [u8; 32],
    pub power: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatorSet {
    members: Vec<Member>,
    total_power: u64,
}
impl ValidatorSet {
    pub fn new(mut members: Vec<Member>) -> Result<Self> {
        if members.is_empty() || members.len() > MAX_VALIDATORS {
            return Err(Error::Limit);
        }
        members.sort_by_key(|m| m.consensus_key);
        let mut keys = BTreeSet::new();
        let mut total = 0u64;
        for m in &members {
            if m.power == 0 {
                return Err(Error::Genesis);
            }
            for key in [m.consensus_key, m.transport_key] {
                let k = VerifyingKey::from_bytes(&key).map_err(|_| Error::Genesis)?;
                if k.is_weak() || !keys.insert(key) {
                    return Err(Error::Genesis);
                }
            }
            total = total.checked_add(m.power).ok_or(Error::Overflow)?;
        }
        // Upstream threshold arithmetic must not overflow at 2/3 or 1/3.
        if total > u64::MAX / 3 {
            return Err(Error::Overflow);
        }
        Ok(Self {
            members,
            total_power: total,
        })
    }
    pub fn members(&self) -> &[Member] {
        &self.members
    }
    pub fn total_power(&self) -> u64 {
        self.total_power
    }
    pub fn member(&self, key: &[u8; 32]) -> Option<&Member> {
        self.members.iter().find(|m| &m.consensus_key == key)
    }
    pub fn hash(&self) -> Digest {
        let mut w = Writer::default();
        self.put(&mut w);
        hash(b"yamichain-committee-v2", &w.0)
    }
    pub(crate) fn put(&self, w: &mut Writer) {
        w.u32(self.members.len() as u32);
        for m in &self.members {
            w.raw(&m.consensus_key);
            w.raw(&m.transport_key);
            w.u64(m.power);
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Epoch {
    pub first_height: u64,
    pub validators: ValidatorSet,
}
pub(crate) fn validate_epochs(epochs: &[Epoch]) -> Result<()> {
    if epochs.is_empty() || epochs.len() > MAX_EPOCHS || epochs[0].first_height != 1 {
        return Err(Error::Genesis);
    }
    let mut previous = 0;
    let mut consensus = BTreeMap::new();
    let mut transports = BTreeMap::new();
    for e in epochs {
        if e.first_height <= previous || e.first_height > 1_000_000 {
            return Err(Error::Genesis);
        }
        previous = e.first_height;
        for m in e.validators.members() {
            if consensus
                .insert(m.consensus_key, m.transport_key)
                .is_some_and(|k| k != m.transport_key)
                || transports
                    .insert(m.transport_key, m.consensus_key)
                    .is_some_and(|k| k != m.consensus_key)
            {
                return Err(Error::Genesis);
            }
        }
    }
    if consensus.keys().any(|key| transports.contains_key(key)) {
        return Err(Error::Genesis);
    }
    Ok(())
}
impl crate::Genesis {
    /// Public, non-financial simulation identities; never suitable for real keys.
    pub fn development_schedule(plan: &[(u64, usize)]) -> Result<Self> {
        let epochs = plan
            .iter()
            .map(|(first, count)| {
                if *count == 0 || *count > MAX_VALIDATORS {
                    return Err(Error::Limit);
                }
                Ok(Epoch {
                    first_height: *first,
                    validators: ValidatorSet::new(
                        (0..*count)
                            .map(|i| Member {
                                consensus_key: development_key("consensus", i as u8)
                                    .verifying_key()
                                    .to_bytes(),
                                transport_key: development_key("transport", i as u8)
                                    .verifying_key()
                                    .to_bytes(),
                                power: 1,
                            })
                            .collect(),
                    )?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let g = Self {
            chain_id: crate::CHAIN_ID.into(),
            protocol: crate::PROTOCOL,
            features: 0,
            epochs,
        };
        g.validate()?;
        Ok(g)
    }
    pub fn committee_at(&self, height: u64) -> Result<&ValidatorSet> {
        if height == 0 {
            return Err(Error::Header);
        }
        self.epochs
            .iter()
            .rev()
            .find(|e| e.first_height <= height)
            .map(|e| &e.validators)
            .ok_or(Error::Genesis)
    }
    pub fn is_reserved_key(&self, key: &[u8; 32]) -> bool {
        self.epochs
            .iter()
            .flat_map(|e| e.validators.members())
            .any(|m| &m.consensus_key == key || &m.transport_key == key)
    }
    pub fn has_consensus_key(&self, key: &[u8; 32]) -> bool {
        self.epochs
            .iter()
            .any(|e| e.validators.member(key).is_some())
    }
}
