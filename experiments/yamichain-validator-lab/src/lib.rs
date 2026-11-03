//! Isolated simulated elections. No production selector implementation or live
//! eligibility, randomness beacon, economic Sybil resistance or stake accounting.
use eyre::{eyre, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use yami_chain::{
    codec::Writer,
    development_key, hash,
    validators::{AddressPolicy, Announcement},
    verify, Digest, Epoch, Genesis, Member, Signer as _, SigningKey, ValidatorSet,
};

pub const MAX_PLAN_BYTES: usize = 1_048_576;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Allocation {
    pub id: u64,
    pub index: u8,
    pub weight: u64,
    pub from: u64,
    pub until: u64,
    pub signature: Vec<u8>,
}
impl Allocation {
    fn preimage(&self, scope: Digest) -> Vec<u8> {
        let mut w = Writer::default();
        w.raw(b"YC-SIM-ALLOCATION-1");
        w.raw(&scope.0);
        for value in [self.id, self.weight, self.from, self.until] {
            w.u64(value);
        }
        w.u8(self.index);
        for role in ["consensus", "transport"] {
            w.raw(&development_key(role, self.index).verifying_key().to_bytes());
        }
        w.0
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Round {
    pub epoch: u64,
    pub height: u64,
    pub committee_size: u16,
    pub parent: [u8; 32],
    pub announcements: Vec<Vec<u8>>,
    pub frozen_root: [u8; 32],
    pub entropy_commitment: [u8; 32],
    pub entropy: [u8; 32],
    pub signature: Vec<u8>,
}
impl Round {
    fn preimage(&self, scope: Digest) -> Vec<u8> {
        let mut w = Writer::default();
        w.raw(b"YC-SIM-FROZEN-ELECTION-1");
        w.raw(&scope.0);
        w.u64(self.epoch);
        w.u64(self.height);
        w.u16(self.committee_size);
        for d in [self.parent, self.frozen_root, self.entropy_commitment] {
            w.raw(&d);
        }
        w.0
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub version: u16,
    pub scope: [u8; 32],
    pub eligibility_authority: [u8; 32],
    pub entropy_authority: [u8; 32],
    pub allocations: Vec<Allocation>,
    pub rounds: Vec<Round>,
}
pub fn plan_digest(bytes: &[u8]) -> Digest {
    hash(b"YC-SIM-PLAN-PIN-1", bytes)
}
fn signature(bytes: &[u8]) -> Result<[u8; 64]> {
    bytes.try_into().map_err(|_| eyre!("signature length"))
}
fn fresh() -> Result<[u8; 32]> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|e| eyre!("OS entropy: {e}"))?;
    Ok(bytes)
}

impl Plan {
    pub fn decode(bytes: &[u8], pin: Digest) -> Result<Self> {
        if bytes.len() > MAX_PLAN_BYTES || plan_digest(bytes) != pin {
            return Err(eyre!("plan size/pin mismatch"));
        }
        Ok(serde_json::from_slice(bytes)?)
    }
    fn validate_allocations(&self) -> Result<()> {
        if self.version != 1
            || self.allocations.is_empty()
            || self.allocations.len() > 100
            || self.rounds.is_empty()
            || self.rounds.len() > 64
            || self.eligibility_authority == self.entropy_authority
        {
            return Err(eyre!("plan bounds"));
        }
        let mut ids = BTreeSet::new();
        let mut indices = BTreeSet::new();
        for a in &self.allocations {
            if a.index >= 100
                || a.id == 0
                || a.weight == 0
                || a.from == 0
                || a.until < a.from
                || !ids.insert(a.id)
                || !indices.insert(a.index)
            {
                return Err(eyre!("duplicate/invalid simulated backing"));
            }
            verify(
                &self.eligibility_authority,
                &a.preimage(Digest(self.scope)),
                &signature(&a.signature)?,
            )?;
        }
        Ok(())
    }
    fn roster(&self, round: &Round) -> Result<(Vec<(u64, Member)>, Digest)> {
        if round.announcements.len() > 128 {
            return Err(eyre!("announcement cap"));
        }
        let mut registrations = BTreeMap::new();
        for bytes in &round.announcements {
            let a = Announcement::decode(bytes)?;
            a.verify(
                Digest(self.scope),
                round.epoch,
                AddressPolicy::LoopbackSimulation,
            )?;
            if registrations.insert(a.consensus, a).is_some() {
                return Err(eyre!("conflicting frozen registrations"));
            }
        }
        let mut allocations = self.allocations.clone();
        allocations.sort_by_key(|a| a.id);
        let mut w = Writer::default();
        w.raw(b"YC-SIM-FROZEN-ROSTER-1");
        w.raw(&self.scope);
        w.u64(round.epoch);
        let mut roster = Vec::new();
        for a in allocations {
            if a.from > round.epoch || a.until < round.epoch {
                continue;
            }
            let key = development_key("consensus", a.index)
                .verifying_key()
                .to_bytes();
            let Some(registration) = registrations.get(&key) else {
                return Err(eyre!("eligible registration omitted from frozen input"));
            };
            if registration.transport
                != development_key("transport", a.index)
                    .verifying_key()
                    .to_bytes()
            {
                return Err(eyre!("allocation key binding"));
            }
            w.bytes(&a.preimage(Digest(self.scope)))?;
            w.bytes(&registration.canonical())?;
            if registration.registered {
                roster.push((
                    a.id,
                    Member {
                        consensus_key: key,
                        transport_key: registration.transport,
                        power: a.weight,
                    },
                ));
            }
        }
        Ok((roster, hash(b"YC-SIM-ROSTER-HASH-1", &w.0)))
    }
    /// Compiles an externally pinned, closed simulation plan into an ordinary
    /// pinned genesis. It does not activate the production ElectionsDisabled API.
    pub fn compile(&self) -> Result<Genesis> {
        self.validate_allocations()?;
        let mut parent = Digest(self.scope);
        let mut last_height = 0;
        let mut epochs = Vec::new();
        for (i, round) in self.rounds.iter().enumerate() {
            if round.epoch != i as u64 + 1
                || round.height <= last_height
                || round.height > 1_000_000
                || (i == 0 && round.height != 1)
                || round.parent != parent.0
            {
                return Err(eyre!("election activation/parent mismatch"));
            }
            let (roster, root) = self.roster(round)?;
            if root.0 != round.frozen_root
                || hash(b"YC-SIM-ENTROPY-COMMIT-1", &round.entropy).0 != round.entropy_commitment
            {
                return Err(eyre!("frozen inputs/randomness changed"));
            }
            verify(
                &self.entropy_authority,
                &round.preimage(Digest(self.scope)),
                &signature(&round.signature)?,
            )?;
            // The seed is authenticated *simulation input*, not an unbiased
            // public beacon. The authority could grind/withhold before pinning.
            let mut seed_bytes = round.preimage(Digest(self.scope));
            seed_bytes.extend_from_slice(&round.entropy);
            let seed = hash(b"YC-SIM-ELECTION-SEED-1", &seed_bytes);
            let selected = sample(roster, round.committee_size as usize, seed)?;
            let mut transcript = seed_bytes;
            transcript.extend_from_slice(&selected.hash().0);
            parent = hash(b"YC-SIM-ELECTION-TRANSCRIPT-1", &transcript);
            epochs.push(Epoch {
                first_height: round.height,
                validators: selected,
            });
            last_height = round.height;
        }
        let g = Genesis {
            epochs,
            ..Genesis::local()
        };
        g.validate()?;
        Ok(g)
    }
    /// Fresh ephemeral fixture signing authorities and entropy. No issuer secret
    /// or randomness key is persisted; only local development validator keys.
    pub fn fixture(base_port: u16) -> Result<Self> {
        if !(1024..=65400).contains(&base_port) {
            return Err(eyre!("port range"));
        }
        let issuer = SigningKey::from_bytes(&fresh()?);
        let beacon = SigningKey::from_bytes(&fresh()?);
        let mut plan = Self {
            version: 1,
            scope: fresh()?,
            eligibility_authority: issuer.verifying_key().to_bytes(),
            entropy_authority: beacon.verifying_key().to_bytes(),
            allocations: vec![],
            rounds: vec![],
        };
        for index in 0..8 {
            let mut a = Allocation {
                id: u64::from(index) + 1,
                index,
                weight: 10 + u64::from(index) * 3,
                from: if index >= 6 { 2 } else { 1 },
                until: 3,
                signature: vec![],
            };
            a.signature = issuer
                .sign(&a.preimage(Digest(plan.scope)))
                .to_bytes()
                .to_vec();
            plan.allocations.push(a);
        }
        let mut parent = Digest(plan.scope);
        let mut retiring = None;
        for (i, height) in [1, 5, 9].into_iter().enumerate() {
            let epoch = i as u64 + 1;
            let mut round = Round {
                epoch,
                height,
                committee_size: 4,
                parent: parent.0,
                announcements: vec![],
                frozen_root: [0; 32],
                entropy_commitment: [0; 32],
                entropy: [0; 32],
                signature: vec![],
            };
            for index in 0..8 {
                round.announcements.push(
                    Announcement::sign(
                        Digest(plan.scope),
                        epoch,
                        epoch,
                        std::net::SocketAddrV4::new(
                            std::net::Ipv4Addr::LOCALHOST,
                            base_port + u16::from(index),
                        ),
                        !(retiring == Some(index) && epoch >= 2),
                        &development_key("consensus", index),
                        &development_key("transport", index),
                    )
                    .canonical(),
                );
            }
            round.frozen_root = plan.roster(&round)?.1 .0;
            round.entropy = fresh()?;
            round.entropy_commitment = hash(b"YC-SIM-ENTROPY-COMMIT-1", &round.entropy).0;
            round.signature = beacon
                .sign(&round.preimage(Digest(plan.scope)))
                .to_bytes()
                .to_vec();
            let (roster, _) = plan.roster(&round)?;
            let mut seed_bytes = round.preimage(Digest(plan.scope));
            seed_bytes.extend_from_slice(&round.entropy);
            let selected = sample(roster, 4, hash(b"YC-SIM-ELECTION-SEED-1", &seed_bytes))?;
            if i == 0 {
                retiring = (0..8).find(|index| {
                    selected
                        .member(
                            &development_key("consensus", *index)
                                .verifying_key()
                                .to_bytes(),
                        )
                        .is_some()
                });
            }
            seed_bytes.extend_from_slice(&selected.hash().0);
            parent = hash(b"YC-SIM-ELECTION-TRANSCRIPT-1", &seed_bytes);
            plan.rounds.push(round);
        }
        plan.compile()?;
        Ok(plan)
    }
}

/// Exact integer weighted sampling without replacement. Registry identity is
/// bound to a unique simulated backing ID, canonically ordered by that ID. Selected
/// seats have unit BFT power; input weight affects sampling, not a second vote.
fn sample(mut pool: Vec<(u64, Member)>, size: usize, seed: Digest) -> Result<ValidatorSet> {
    if size == 0 || size > 100 || size > pool.len() {
        return Err(eyre!(
            "insufficient eligible candidates; halt, never lower quorum"
        ));
    }
    pool.sort_by_key(|(id, _)| *id);
    let mut selected = Vec::new();
    let mut counter = 0u64;
    for _ in 0..size {
        let total = pool
            .iter()
            .try_fold(0u64, |sum, (_, m)| sum.checked_add(m.power))
            .ok_or_else(|| eyre!("weight overflow"))?;
        if total == 0 || total > u64::MAX / 3 {
            return Err(eyre!("weight bounds"));
        }
        // Reject the low incomplete residue interval of a 2^64 sample space.
        let threshold = total.wrapping_neg() % total;
        let mut ticket = None;
        for _ in 0..256 {
            let mut input = seed.0.to_vec();
            input.extend_from_slice(&counter.to_be_bytes());
            counter = counter
                .checked_add(1)
                .ok_or_else(|| eyre!("sampler counter"))?;
            let bytes = hash(b"YC-SIM-SAMPLE-1", &input).0;
            let value = u64::from_be_bytes(bytes[..8].try_into()?);
            if value >= threshold {
                ticket = Some(value % total);
                break;
            }
        }
        let mut ticket = ticket.ok_or_else(|| eyre!("sampler rejection cap"))?;
        let mut chosen = None;
        for (i, (_, member)) in pool.iter().enumerate() {
            if ticket < member.power {
                chosen = Some(i);
                break;
            }
            ticket -= member.power;
        }
        let mut member = pool
            .remove(chosen.ok_or_else(|| eyre!("sampler invariant"))?)
            .1;
        member.power = 1;
        selected.push(member);
    }
    Ok(ValidatorSet::new(selected)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn authorize_first(plan: &mut Plan) {
        let key = SigningKey::from_bytes(&[201; 32]);
        plan.entropy_authority = key.verifying_key().to_bytes();
        plan.rounds[0].frozen_root = plan.roster(&plan.rounds[0]).unwrap().1 .0;
        plan.rounds[0].signature = key
            .sign(&plan.rounds[0].preimage(Digest(plan.scope)))
            .to_bytes()
            .to_vec();
    }
    #[test]
    fn fixed_inputs_are_order_independent_and_discovery_is_not_eligibility() {
        let plan = Plan::fixture(28100).unwrap();
        let expected = plan.compile().unwrap();
        let mut reordered = plan.clone();
        reordered.allocations.reverse();
        for round in &mut reordered.rounds {
            round.announcements.reverse();
        }
        assert_eq!(reordered.compile().unwrap(), expected);
        let mut sybil = plan;
        for round in &mut sybil.rounds {
            for index in 20..40 {
                round.announcements.push(
                    Announcement::sign(
                        Digest(sybil.scope),
                        round.epoch,
                        1,
                        "127.0.0.1:29000".parse().unwrap(),
                        true,
                        &development_key("consensus", index),
                        &development_key("transport", index),
                    )
                    .canonical(),
                );
            }
        }
        assert_eq!(sybil.compile().unwrap(), expected);
        assert!(expected
            .epochs
            .iter()
            .all(|e| e.validators.members().len() == 4 && e.validators.total_power() == 4));
        let retired = Announcement::decode(
            sybil.rounds[1]
                .announcements
                .iter()
                .find(|bytes| !Announcement::decode(bytes).unwrap().registered)
                .unwrap(),
        )
        .unwrap()
        .consensus;
        assert!(expected.epochs[1..]
            .iter()
            .all(|e| e.validators.member(&retired).is_none()));
    }
    #[test]
    fn forged_backing_randomness_and_conflicting_elections_fail() {
        let plan = Plan::fixture(28100).unwrap();
        for attack in 0..10 {
            let mut p = plan.clone();
            match attack {
                0 => p.allocations[0].weight += 1,
                1 => p.allocations[1].id = p.allocations[0].id,
                2 => p.allocations[1].index = p.allocations[0].index,
                3 => p.rounds[0].entropy[0] ^= 1,
                4 => p.rounds[1].parent[0] ^= 1,
                5 => p.rounds[1].epoch = 1,
                6 => p.rounds[1].height = 1,
                7 => p.rounds[0].frozen_root[0] ^= 1,
                8 => p.rounds[0].announcements[0][40] ^= 1,
                _ => p.rounds[0].signature[0] ^= 1,
            }
            assert!(p.compile().is_err(), "attack {attack}");
        }
        let mut fresh_authority = plan.clone();
        let attacker = SigningKey::from_bytes(&[88; 32]);
        fresh_authority.allocations[0].weight += 1000;
        fresh_authority.allocations[0].signature = attacker
            .sign(&fresh_authority.allocations[0].preimage(Digest(plan.scope)))
            .to_bytes()
            .to_vec();
        assert!(fresh_authority.compile().is_err());
        let bytes = serde_json::to_vec(&plan).unwrap();
        assert!(Plan::decode(
            &serde_json::to_vec(&Plan::fixture(28100).unwrap()).unwrap(),
            plan_digest(&bytes)
        )
        .is_err());
        assert!(Plan::decode(&bytes[..bytes.len() - 1], plan_digest(&bytes)).is_err());
    }
    #[test]
    fn authenticated_shortage_halts_without_shrinking_committee_or_quorum() {
        let mut plan = Plan::fixture(28100).unwrap();
        plan.rounds[0].committee_size = 7; // Only six eligible backing slots.
        authorize_first(&mut plan);
        assert!(plan
            .compile()
            .unwrap_err()
            .to_string()
            .contains("insufficient eligible"));
        let mut plan = Plan::fixture(28100).unwrap();
        for index in 0..3 {
            plan.rounds[0].announcements[index] = Announcement::sign(
                Digest(plan.scope),
                1,
                2,
                "127.0.0.1:28100".parse().unwrap(),
                false,
                &development_key("consensus", index as u8),
                &development_key("transport", index as u8),
            )
            .canonical();
        }
        authorize_first(&mut plan);
        assert!(plan
            .compile()
            .unwrap_err()
            .to_string()
            .contains("insufficient eligible"));
    }
    #[test]
    fn integer_sampler_bounds_uniqueness_and_weight_safety() {
        let pool: Vec<_> = (0..8)
            .map(|i| {
                (
                    u64::from(i) + 1,
                    Member {
                        consensus_key: development_key("consensus", i).verifying_key().to_bytes(),
                        transport_key: development_key("transport", i).verifying_key().to_bytes(),
                        power: u64::from(i) + 1,
                    },
                )
            })
            .collect();
        assert!(sample(pool.clone(), 0, Digest([1; 32])).is_err());
        assert!(sample(pool.clone(), 9, Digest([1; 32])).is_err());
        assert_eq!(
            sample(pool.clone(), 8, Digest([1; 32]))
                .unwrap()
                .members()
                .len(),
            8
        );
        let a = sample(pool.clone(), 4, Digest([2; 32])).unwrap();
        assert_eq!(sample(pool.clone(), 4, Digest([2; 32])).unwrap(), a);
        let mut huge = pool;
        huge[0].1.power = u64::MAX;
        assert!(sample(huge, 4, Digest([1; 32])).is_err());
    }
}
