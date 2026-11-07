//! Hermetic test-only accounting. No wire discriminant, executable entry point,
//! YAMI issuance, spendable asset, proof substitute or election is compiled.
use crate::{signing::vote_preimage_for, types::*};
use malachite::app::types::core as m;
use std::collections::{BTreeMap, BTreeSet};
use yami_chain::{codec::Writer, verify, Error, Genesis, Result};
type Key = [u8; 32];
#[derive(Clone, Debug, PartialEq, Eq)]
struct Registration {
    owner: Key,
    transport: Key,
    commission: u16,
    since: u64,
    jailed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Position {
    owner: Key,
    validator: Key,
    units: u128,
    since: u64,
    end: Option<u64>,
    unlock: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Op {
    Register {
        consensus: Key,
        transport: Key,
        commission: u16,
        consensus_pop: [u8; 64],
        transport_pop: [u8; 64],
    },
    Delegate {
        validator: Key,
        id: u64,
        units: u128,
    },
    Unbond {
        id: u64,
    },
    Withdraw {
        id: u64,
    },
}
fn registration_bytes(
    genesis: &Genesis,
    owner: Key,
    consensus: Key,
    transport: Key,
    commission: u16,
) -> Vec<u8> {
    let mut w = Writer::default();
    w.raw(b"YC-SIM-REGISTRATION-2");
    w.raw(&genesis.id().0);
    for key in [owner, consensus, transport] {
        w.raw(&key);
    }
    w.u16(commission);
    w.0
}
fn command_bytes(genesis: &Genesis, owner: Key, nonce: u64, op: &Op) -> Vec<u8> {
    let mut w = Writer::default();
    w.raw(b"YC-SIM-COMMAND-2");
    w.raw(&genesis.id().0);
    w.raw(&owner);
    w.u64(nonce);
    match op {
        Op::Register {
            consensus,
            transport,
            commission,
            consensus_pop,
            transport_pop,
        } => {
            w.u8(0);
            w.raw(consensus);
            w.raw(transport);
            w.u16(*commission);
            w.raw(consensus_pop);
            w.raw(transport_pop);
        }
        Op::Delegate {
            validator,
            id,
            units,
        } => {
            w.u8(1);
            w.raw(validator);
            w.u64(*id);
            w.raw(&units.to_be_bytes());
        }
        Op::Unbond { id } => {
            w.u8(2);
            w.u64(*id);
        }
        Op::Withdraw { id } => {
            w.u8(3);
            w.u64(*id);
        }
    }
    w.0
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Simulation {
    genesis: Genesis,
    height: u64,
    delay: u64,
    evidence_age: u64,
    liquid: BTreeMap<Key, u128>,
    nonces: BTreeMap<Key, u64>,
    validators: BTreeMap<Key, Registration>,
    positions: BTreeMap<u64, Position>,
    used_positions: BTreeSet<u64>,
    punished: BTreeSet<(Key, u64, u32, u8)>,
    rewarded: BTreeSet<(Key, u64)>,
    reward_pool: u128,
    burned: u128,
    initial: u128,
}
impl Simulation {
    fn new(
        genesis: Genesis,
        fixtures: &[(Key, u128)],
        reward_pool: u128,
        delay: u64,
        evidence_age: u64,
    ) -> Result<Self> {
        if delay <= evidence_age || evidence_age == 0 {
            return Err(Error::Unsupported);
        }
        genesis.validate()?;
        let mut liquid = BTreeMap::new();
        let mut initial = reward_pool;
        for (key, amount) in fixtures {
            if genesis.is_reserved_key(key) || liquid.insert(*key, *amount).is_some() {
                return Err(Error::Duplicate);
            }
            initial = initial.checked_add(*amount).ok_or(Error::Overflow)?;
        }
        Ok(Self {
            genesis,
            height: 1,
            delay,
            evidence_age,
            liquid,
            initial,
            reward_pool,
            burned: 0,
            nonces: BTreeMap::new(),
            validators: BTreeMap::new(),
            positions: BTreeMap::new(),
            used_positions: BTreeSet::new(),
            punished: BTreeSet::new(),
            rewarded: BTreeSet::new(),
        })
    }
    fn invariant(&self) -> Result<()> {
        let sum = self
            .liquid
            .values()
            .chain(self.positions.values().map(|p| &p.units))
            .try_fold(
                self.reward_pool
                    .checked_add(self.burned)
                    .ok_or(Error::Overflow)?,
                |a, b| a.checked_add(*b).ok_or(Error::Overflow),
            )?;
        if sum != self.initial {
            return Err(Error::Overflow);
        }
        Ok(())
    }
    fn credit(&mut self, owner: Key, units: u128) -> Result<()> {
        let old = self.liquid.get(&owner).copied().unwrap_or(0);
        self.liquid
            .insert(owner, old.checked_add(units).ok_or(Error::Overflow)?);
        Ok(())
    }
    fn apply(&mut self, owner: Key, nonce: u64, op: Op, signature: [u8; 64]) -> Result<()> {
        let mut next = self.clone();
        if self.genesis.is_reserved_key(&owner) {
            return Err(Error::Signature);
        }
        verify(
            &owner,
            &command_bytes(&self.genesis, owner, nonce, &op),
            &signature,
        )?;
        if nonce != *self.nonces.get(&owner).unwrap_or(&0) {
            return Err(Error::Nonce);
        }
        match op {
            Op::Register {
                consensus,
                transport,
                commission,
                consensus_pop,
                transport_pop,
            } => {
                if commission > 10_000
                    || owner == consensus
                    || owner == transport
                    || consensus == transport
                    || self.validators.iter().any(|(key, v)| {
                        *key == consensus
                            || *key == transport
                            || v.transport == transport
                            || v.transport == consensus
                            || v.owner == consensus
                            || v.owner == transport
                    })
                    || self.liquid.contains_key(&consensus)
                    || self.liquid.contains_key(&transport)
                    || self.validators.contains_key(&owner)
                    || self.validators.values().any(|v| v.transport == owner)
                {
                    return Err(Error::Duplicate);
                }
                let payload =
                    registration_bytes(&self.genesis, owner, consensus, transport, commission);
                verify(&consensus, &payload, &consensus_pop)?;
                verify(&transport, &payload, &transport_pop)?;
                next.validators.insert(
                    consensus,
                    Registration {
                        owner,
                        transport,
                        commission,
                        since: self.height,
                        jailed: false,
                    },
                );
            }
            Op::Delegate {
                validator,
                id,
                units,
            } => {
                if units == 0
                    || self.used_positions.contains(&id)
                    || self.validators.get(&validator).is_none_or(|v| v.jailed)
                {
                    return Err(Error::Unsupported);
                }
                let liquid = self
                    .liquid
                    .get(&owner)
                    .copied()
                    .unwrap_or(0)
                    .checked_sub(units)
                    .ok_or(Error::Overflow)?;
                next.liquid.insert(owner, liquid);
                next.used_positions.insert(id);
                next.positions.insert(
                    id,
                    Position {
                        owner,
                        validator,
                        units,
                        since: self.height,
                        end: None,
                        unlock: None,
                    },
                );
            }
            Op::Unbond { id } => {
                let p = next.positions.get_mut(&id).ok_or(Error::Unsupported)?;
                if p.owner != owner || p.end.is_some() {
                    return Err(Error::Unsupported);
                }
                p.end = Some(self.height);
                p.unlock = Some(self.height.checked_add(self.delay).ok_or(Error::Overflow)?);
            }
            Op::Withdraw { id } => {
                let p = next.positions.get(&id).ok_or(Error::Unsupported)?;
                if p.owner != owner || p.unlock.is_none_or(|h| h > self.height) {
                    return Err(Error::Unsupported);
                }
                let units = p.units;
                next.positions.remove(&id);
                next.credit(owner, units)?;
            }
        }
        next.nonces
            .insert(owner, nonce.checked_add(1).ok_or(Error::Overflow)?);
        next.invariant()?;
        *self = next;
        Ok(())
    }
    fn eligible(&self, key: Key, min_self: u128, min_total: u128) -> bool {
        let Some(v) = self.validators.get(&key) else {
            return false;
        };
        if v.jailed {
            return false;
        }
        let mut own = 0u128;
        let mut total = 0u128;
        for p in self
            .positions
            .values()
            .filter(|p| p.validator == key && p.end.is_none())
        {
            total += p.units;
            if p.owner == v.owner {
                own += p.units;
            }
        }
        own >= min_self && total >= min_total
    }
    fn reward(&mut self, key: Key, epoch: u64, units: u128) -> Result<()> {
        let mut next = self.clone();
        let v = self.validators.get(&key).ok_or(Error::Unsupported)?;
        if v.jailed || !next.rewarded.insert((key, epoch)) {
            return Err(Error::Duplicate);
        }
        let stake = self
            .positions
            .values()
            .filter(|p| p.validator == key && p.end.is_none())
            .try_fold(0u128, |a, p| a.checked_add(p.units).ok_or(Error::Overflow))?;
        if stake == 0 || units > self.reward_pool {
            return Err(Error::Overflow);
        }
        let commission = units
            .checked_mul(v.commission.into())
            .ok_or(Error::Overflow)?
            / 10_000;
        let delegator_units = units - commission;
        let mut paid = commission;
        next.credit(v.owner, commission)?;
        for p in self
            .positions
            .values()
            .filter(|p| p.validator == key && p.end.is_none())
        {
            let share = delegator_units
                .checked_mul(p.units)
                .ok_or(Error::Overflow)?
                / stake;
            next.credit(p.owner, share)?;
            paid = paid.checked_add(share).ok_or(Error::Overflow)?;
        }
        // Integer rounding remainder stays in the fixture reward reserve.
        next.reward_pool -= paid;
        next.invariant()?;
        *self = next;
        Ok(())
    }
    fn slash(
        &mut self,
        a: &m::SignedMessage<Context, Vote>,
        b: &m::SignedMessage<Context, Vote>,
        basis_points: u16,
    ) -> Result<()> {
        let left = crate::wire::encode(
            self.genesis.id(),
            &malachite_core::SignedConsensusMsg::<Context>::Vote(a.clone()),
        )?;
        let right = crate::wire::encode(
            self.genesis.id(),
            &malachite_core::SignedConsensusMsg::<Context>::Vote(b.clone()),
        )?;
        crate::equivocation::verify_double_vote(&self.genesis, &left, &right)?;
        let va = &a.message;
        if basis_points == 0
            || basis_points > 10_000
            || va.height.0 > self.height
            || self.height - va.height.0 > self.evidence_age
        {
            return Err(Error::Signature);
        }
        let evidence = (
            va.address.0 .0,
            va.height.0,
            va.round.as_u32().ok_or(Error::Encoding)?,
            match va.step {
                m::VoteType::Prevote => 1,
                m::VoteType::Precommit => 2,
            },
        );
        if va.height.0
            < self
                .validators
                .get(&va.address.0 .0)
                .ok_or(Error::Unsupported)?
                .since
        {
            return Err(Error::Signature);
        }
        let mut next = self.clone();
        if !next.punished.insert(evidence) {
            return Err(Error::Duplicate);
        }
        next.validators
            .get_mut(&va.address.0 .0)
            .ok_or(Error::Unsupported)?
            .jailed = true;
        for p in next.positions.values_mut().filter(|p| {
            p.validator == va.address.0 .0
                && p.since <= va.height.0
                && p.end.is_none_or(|h| va.height.0 <= h)
        }) {
            let penalty = p
                .units
                .checked_mul(basis_points.into())
                .ok_or(Error::Overflow)?
                / 10_000;
            p.units -= penalty;
            next.burned = next.burned.checked_add(penalty).ok_or(Error::Overflow)?;
        }
        next.invariant()?;
        *self = next;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use yami_chain::{development_key, Digest, Signer as _, SigningKey};
    fn submit(s: &mut Simulation, key: &SigningKey, op: Op) -> Result<()> {
        let owner = key.verifying_key().to_bytes();
        let nonce = *s.nonces.get(&owner).unwrap_or(&0);
        let signature = key
            .sign(&command_bytes(&s.genesis, owner, nonce, &op))
            .to_bytes();
        s.apply(owner, nonce, op, signature)
    }
    fn registration(g: &Genesis, owner: &SigningKey) -> Op {
        let c = development_key("consensus", 0);
        let t = development_key("transport", 0);
        let consensus = c.verifying_key().to_bytes();
        let transport = t.verifying_key().to_bytes();
        let p = registration_bytes(
            g,
            owner.verifying_key().to_bytes(),
            consensus,
            transport,
            1000,
        );
        Op::Register {
            consensus,
            transport,
            commission: 1000,
            consensus_pop: c.sign(&p).to_bytes(),
            transport_pop: t.sign(&p).to_bytes(),
        }
    }
    fn evidence(
        g: &Genesis,
    ) -> (
        m::SignedMessage<Context, Vote>,
        m::SignedMessage<Context, Vote>,
    ) {
        let k = development_key("consensus", 0);
        let address = Address(Digest(k.verifying_key().to_bytes()));
        let sign = |value| {
            let v = Vote {
                height: Height(1),
                round: m::Round::ZERO,
                step: m::VoteType::Precommit,
                value,
                address,
                extension: None,
            };
            let signature = k.sign(&vote_preimage_for(g, &v).unwrap()).to_bytes();
            m::SignedMessage::new(v, signature)
        };
        (
            sign(m::NilOrVal::Nil),
            sign(m::NilOrVal::Val(Value(Digest([5; 32])))),
        )
    }
    #[test]
    fn registration_delegation_rewards_unbonding_and_real_signed_evidence_conserve() {
        let g = Genesis::local();
        let owner = development_key("stake-owner", 0);
        let delegator = development_key("delegator", 0);
        let o = owner.verifying_key().to_bytes();
        let d = delegator.verifying_key().to_bytes();
        let validator = development_key("consensus", 0).verifying_key().to_bytes();
        let mut s = Simulation::new(g.clone(), &[(o, 1000), (d, 2000)], 1000, 10, 5).unwrap();
        submit(&mut s, &owner, registration(&g, &owner)).unwrap();
        submit(
            &mut s,
            &owner,
            Op::Delegate {
                validator,
                id: 1,
                units: 100,
            },
        )
        .unwrap();
        submit(
            &mut s,
            &delegator,
            Op::Delegate {
                validator,
                id: 2,
                units: 900,
            },
        )
        .unwrap();
        assert!(s.eligible(validator, 100, 1000));
        assert!(!s.eligible(validator, 101, 1000));
        s.reward(validator, 1, 100).unwrap();
        assert_eq!(s.liquid[&o], 919);
        assert_eq!(s.liquid[&d], 1181);
        let before = s.clone();
        assert!(s.reward(validator, 1, 100).is_err());
        assert_eq!(s, before);
        s.height = 2;
        submit(&mut s, &delegator, Op::Unbond { id: 2 }).unwrap();
        let before = s.clone();
        assert!(submit(&mut s, &delegator, Op::Withdraw { id: 2 }).is_err());
        assert_eq!(s, before);
        let (a, b) = evidence(&g);
        s.height = 3;
        s.slash(&a, &b, 1000).unwrap();
        assert_eq!(s.burned, 100);
        assert!(!s.eligible(validator, 1, 1));
        let before = s.clone();
        assert!(s.slash(&b, &a, 1000).is_err());
        assert_eq!(s, before);
        s.height = 12;
        submit(&mut s, &delegator, Op::Withdraw { id: 2 }).unwrap();
        assert_eq!(s.liquid[&d], 1991);
        s.invariant().unwrap();
        assert!(s.slash(&a, &b, 1000).is_err());
    }
    #[test]
    fn invalid_proofs_replays_duplicates_balances_and_randomized_failures_are_atomic() {
        let g = Genesis::local();
        let key = development_key("stake-owner", 1);
        let owner = key.verifying_key().to_bytes();
        assert!(Simulation::new(g.clone(), &[(owner, 1), (owner, 1)], 0, 10, 5).is_err());
        assert!(Simulation::new(g.clone(), &[(owner, u128::MAX)], 1, 10, 5).is_err());
        assert!(Simulation::new(g.clone(), &[], 0, 5, 5).is_err());
        let mut s = Simulation::new(g.clone(), &[(owner, 1000)], 500, 10, 5).unwrap();
        let mut bad = registration(&g, &key);
        if let Op::Register {
            ref mut consensus_pop,
            ..
        } = bad
        {
            consensus_pop[0] ^= 1;
        }
        let before = s.clone();
        assert!(submit(&mut s, &key, bad).is_err());
        assert_eq!(s, before);
        submit(&mut s, &key, registration(&g, &key)).unwrap();
        let validator = development_key("consensus", 0).verifying_key().to_bytes();
        let op = Op::Delegate {
            validator,
            id: 7,
            units: 100,
        };
        let sig = key.sign(&command_bytes(&g, owner, 1, &op)).to_bytes();
        s.apply(owner, 1, op.clone(), sig).unwrap();
        let before = s.clone();
        assert!(s.apply(owner, 1, op, sig).is_err());
        assert_eq!(s, before);
        let (a, mut b) = evidence(&g);
        b.signature[0] ^= 1;
        assert!(s.slash(&a, &b, 1000).is_err());
        assert_eq!(s, before);
        let mut seed = 17u64;
        for id in 0..200 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let before = s.clone();
            let op = match seed % 3 {
                0 => Op::Delegate {
                    validator,
                    id,
                    units: (seed % 2000).into(),
                },
                1 => Op::Unbond { id },
                _ => Op::Withdraw { id },
            };
            if submit(&mut s, &key, op).is_err() {
                assert_eq!(s, before);
            }
            s.invariant().unwrap();
        }
    }
}
