//! Isolated account-based ElGamal transfer experiment. Never a consensus backend.
use ark_bls12_381::{Bls12_381, Fr};
use ark_ec::{AffineRepr, CurveGroup, PrimeGroup};
use ark_ed_on_bls12_381::{
    constraints::EdwardsVar, EdwardsAffine as Point, EdwardsProjective as Group, Fr as Scalar,
};
use ark_ff::{One, PrimeField, UniformRand, Zero};
use ark_groth16::{prepare_verifying_key, Groth16, Proof, ProvingKey, VerifyingKey};
use ark_r1cs_std::{boolean::Boolean, fields::fp::FpVar, prelude::*};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
pub type Cipher = (Point, Point); // (bG + rY, rG)
pub fn bytes<T: CanonicalSerialize>(v: &T) -> Vec<u8> {
    let mut b = vec![];
    v.serialize_compressed(&mut b).unwrap();
    b
}
pub fn random() -> Scalar {
    loop {
        let x = Scalar::rand(&mut OsRng);
        if !x.is_zero() {
            return x;
        }
    }
}
pub fn g() -> Group {
    Group::generator()
}
pub fn encrypt(y: Point, b: Scalar, r: Scalar) -> Cipher {
    ((g() * b + y * r).into_affine(), (g() * r).into_affine())
}
fn add(a: Cipher, b: Cipher) -> Cipher {
    ((a.0 + b.0).into_affine(), (a.1 + b.1).into_affine())
}
/// Try-and-increment hash-to-curve, cofactor cleared; no known discrete log to G.
/// Research domain only. Not an RFC 9380 interoperability claim.
pub fn epoch_base(epoch: u64) -> Point {
    for counter in 0u64.. {
        let mut h = Sha256::new();
        h.update(b"YPP-accounts/research/epoch/v1");
        h.update(epoch.to_le_bytes());
        h.update(counter.to_le_bytes());
        let d = h.finalize();
        let y = Fr::from_le_bytes_mod_order(&d);
        if let Some(p) = Point::get_point_from_y_unchecked(y, false) {
            let p = p.mul_by_cofactor_to_group().into_affine();
            if !p.is_zero() {
                return p;
            }
        }
    }
    unreachable!()
}
#[derive(Clone, CanonicalSerialize, CanonicalDeserialize)]
pub struct Statement {
    pub epoch: u64,
    pub keys: Vec<Point>,
    pub old: Vec<Cipher>,
    pub changes: Vec<Point>,
    pub d: Point,
    pub tag: Point,
}
impl Statement {
    pub fn inputs(&self) -> Vec<Fr> {
        let mut out = vec![];
        for p in self
            .keys
            .iter()
            .chain(self.old.iter().flat_map(|c| [&c.0, &c.1]))
            .chain(self.changes.iter())
            .chain([&self.d, &self.tag, &epoch_base(self.epoch)])
        {
            out.extend([p.x, p.y]);
        }
        out
    }
}
#[derive(Clone)]
pub struct Witness {
    pub sender: usize,
    pub recipient: usize,
    pub secret: Scalar,
    pub randomness: Scalar,
    pub amount: u32,
    pub remaining: u32,
}
#[derive(Clone)]
pub struct Relation {
    pub statement: Statement,
    pub witness: Witness,
}
fn scalar_bits(cs: ConstraintSystemRef<Fr>, x: Scalar) -> Result<Vec<Boolean<Fr>>, SynthesisError> {
    let b = bytes(&x);
    let vars = UInt8::new_witness_vec(cs, &b)?;
    let bits = vars.to_bits_le()?;
    Boolean::enforce_smaller_or_equal_than_le(&bits, (-Scalar::one()).into_bigint())?;
    Ok(bits)
}
fn range(
    cs: ConstraintSystemRef<Fr>,
    v: u32,
) -> Result<(FpVar<Fr>, Vec<Boolean<Fr>>), SynthesisError> {
    let b = (0..32)
        .map(|i| Boolean::new_witness(cs.clone(), || Ok(v >> i & 1 == 1)))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((Boolean::le_bits_to_fp(&b)?, b))
}
impl ConstraintSynthesizer<Fr> for Relation {
    fn generate_constraints(self, cs: ConstraintSystemRef<Fr>) -> Result<(), SynthesisError> {
        let s = &self.statement;
        let w = &self.witness;
        let keys = s
            .keys
            .iter()
            .map(|p| EdwardsVar::new_input(cs.clone(), || Ok(*p)))
            .collect::<Result<Vec<_>, _>>()?;
        let old = s
            .old
            .iter()
            .map(|c| {
                Ok((
                    EdwardsVar::new_input(cs.clone(), || Ok(c.0))?,
                    EdwardsVar::new_input(cs.clone(), || Ok(c.1))?,
                ))
            })
            .collect::<Result<Vec<_>, SynthesisError>>()?;
        let changes = s
            .changes
            .iter()
            .map(|p| EdwardsVar::new_input(cs.clone(), || Ok(*p)))
            .collect::<Result<Vec<_>, _>>()?;
        let d = EdwardsVar::new_input(cs.clone(), || Ok(s.d))?;
        let tag = EdwardsVar::new_input(cs.clone(), || Ok(s.tag))?;
        let base = EdwardsVar::new_input(cs.clone(), || Ok(epoch_base(s.epoch)))?;
        let generator = EdwardsVar::new_constant(cs.clone(), g())?;
        let x = scalar_bits(cs.clone(), w.secret)?;
        Boolean::kary_or(&x)?.enforce_equal(&Boolean::TRUE)?;
        let r = scalar_bits(cs.clone(), w.randomness)?;
        Boolean::kary_or(&r)?.enforce_equal(&Boolean::TRUE)?;
        let (amount, ab) = range(cs.clone(), w.amount)?;
        amount
            .is_eq(&FpVar::zero())?
            .enforce_equal(&Boolean::FALSE)?;
        let (_, bb) = range(cs.clone(), w.remaining)?;
        let ag = generator.scalar_mul_le(ab.iter())?;
        let bg = generator.scalar_mul_le(bb.iter())?;
        d.enforce_equal(&generator.scalar_mul_le(r.iter())?)?;
        tag.enforce_equal(&base.scalar_mul_le(x.iter())?)?;
        let owner = generator.scalar_mul_le(x.iter())?;
        let mut ns = FpVar::zero();
        let mut nr = FpVar::zero();
        let mut selected_key = EdwardsVar::zero();
        let mut selected_l = EdwardsVar::zero();
        let mut selected_r = EdwardsVar::zero();
        for i in 0..keys.len() {
            let a = Boolean::new_witness(cs.clone(), || Ok(i == w.sender))?;
            let b = Boolean::new_witness(cs.clone(), || Ok(i == w.recipient))?;
            (&a & &b).enforce_equal(&Boolean::FALSE)?;
            ns += FpVar::from(a.clone());
            nr += FpVar::from(b.clone());
            selected_key += a.select(&keys[i], &EdwardsVar::zero())?;
            selected_l += a.select(&old[i].0, &EdwardsVar::zero())?;
            selected_r += a.select(&old[i].1, &EdwardsVar::zero())?;
            let delta = b.select(&ag, &EdwardsVar::zero())? - a.select(&ag, &EdwardsVar::zero())?;
            changes[i].enforce_equal(&(keys[i].scalar_mul_le(r.iter())? + delta))?;
        }
        ns.enforce_equal(&FpVar::one())?;
        nr.enforce_equal(&FpVar::one())?;
        selected_key.enforce_equal(&owner)?;
        selected_l.enforce_equal(&(selected_r.scalar_mul_le(x.iter())? + ag + bg))?;
        Ok(())
    }
}
pub fn setup(r: Relation) -> ProvingKey<Bls12_381> {
    Groth16::<Bls12_381>::generate_random_parameters_with_reduction(r, &mut OsRng).unwrap()
}
pub fn prove(pk: &ProvingKey<Bls12_381>, r: Relation) -> Proof<Bls12_381> {
    Groth16::<Bls12_381>::create_random_proof_with_reduction(r, pk, &mut OsRng).unwrap()
}
pub fn verify(vk: &VerifyingKey<Bls12_381>, s: &Statement, p: &Proof<Bls12_381>) -> bool {
    if s.keys.len() < 2 || s.old.len() != s.keys.len() || s.changes.len() != s.keys.len() {
        return false;
    }
    Groth16::<Bls12_381>::verify_proof(&prepare_verifying_key(vk), p, &s.inputs()).unwrap_or(false)
}
#[derive(Clone, CanonicalSerialize, CanonicalDeserialize)]
pub struct Ledger {
    pub epoch: u64,
    pub keys: Vec<Point>,
    pub settled: Vec<Cipher>,
    pub pending: Vec<Cipher>,
    pub tags: Vec<Point>,
}
impl Ledger {
    fn new(keys: Vec<Point>, balances: Vec<Cipher>) -> Result<Self, String> {
        let unique: HashSet<_> = keys.iter().map(bytes).collect();
        if keys.len() != balances.len()
            || unique.len() != keys.len()
            || keys.iter().any(|p| {
                p.is_zero() || !p.is_on_curve() || !p.is_in_correct_subgroup_assuming_on_curve()
            })
        {
            return Err("invalid or duplicate registration".into());
        }
        Ok(Self {
            epoch: 1,
            pending: vec![(Point::zero(), Point::zero()); keys.len()],
            keys,
            settled: balances,
            tags: vec![],
        })
    }
    pub fn relation(
        &self,
        sender: usize,
        recipient: usize,
        secret: Scalar,
        balance: u32,
        amount: u32,
    ) -> Relation {
        let r = random();
        let changes = self
            .keys
            .iter()
            .enumerate()
            .map(|(i, y)| {
                encrypt(
                    *y,
                    if i == sender {
                        -Scalar::from(amount)
                    } else if i == recipient {
                        Scalar::from(amount)
                    } else {
                        Scalar::zero()
                    },
                    r,
                )
                .0
            })
            .collect();
        Relation {
            statement: Statement {
                epoch: self.epoch,
                keys: self.keys.clone(),
                old: self.settled.clone(),
                changes,
                d: (g() * r).into_affine(),
                tag: (epoch_base(self.epoch) * secret).into_affine(),
            },
            witness: Witness {
                sender,
                recipient,
                secret,
                randomness: r,
                amount,
                remaining: balance
                    .checked_sub(amount)
                    .expect("insufficient witness balance"),
            },
        }
    }
    pub fn apply(
        &mut self,
        vk: &VerifyingKey<Bls12_381>,
        s: &Statement,
        p: &Proof<Bls12_381>,
    ) -> Result<(), String> {
        if s.epoch != self.epoch
            || s.keys != self.keys
            || s.old != self.settled
            || s.changes.len() != self.keys.len()
            || self.tags.contains(&s.tag)
        {
            return Err("stale state, wrong set or conflicting debit".into());
        }
        if !verify(vk, s, p) {
            return Err("invalid proof".into());
        }
        for (i, c) in s.changes.iter().enumerate() {
            self.pending[i] = add(self.pending[i], (*c, s.d));
        }
        self.tags.push(s.tag);
        Ok(())
    }
    pub fn rollover(&mut self) {
        for i in 0..self.keys.len() {
            self.settled[i] = add(self.settled[i], self.pending[i]);
            self.pending[i] = (Point::zero(), Point::zero());
        }
        self.epoch += 1;
        self.tags.clear();
    }
    pub fn root(&self) -> [u8; 32] {
        Sha256::digest(bytes(self)).into()
    }
    /// Requires a separately authenticated finalized root, not a root supplied by RPC.
    pub fn recover(
        &self,
        root: [u8; 32],
        index: usize,
        secret: Scalar,
        limit: u32,
    ) -> Result<u32, String> {
        if index >= self.keys.len()
            || self.root() != root
            || (g() * secret).into_affine() != self.keys[index]
        {
            return Err("unauthenticated state or wrong key".into());
        }
        let c = add(self.settled[index], self.pending[index]);
        decode((c.0 - c.1 * secret).into_affine(), limit)
            .ok_or_else(|| "balance outside recovery bound".into())
    }
}
/// Baby-step/giant-step, bounded message-in-exponent recovery.
pub fn decode(p: Point, limit: u32) -> Option<u32> {
    let m = ((limit as f64 + 1.).sqrt().ceil() as u64).max(1);
    let mut table = std::collections::HashMap::new();
    let mut q = Group::zero();
    for j in 0..m {
        table.insert(bytes(&q.into_affine()), j);
        q += g();
    }
    let step = g() * Scalar::from(m);
    let mut q = p.into_group();
    for i in 0..=m {
        if let Some(j) = table.get(&bytes(&q.into_affine())) {
            let v = i * m + j;
            if v <= limit as u64 {
                return Some(v as u32);
            }
        }
        q -= step;
    }
    None
}
#[derive(Clone)]
pub struct Registration {
    pub key: Point,
    pub commitment: Point,
    pub response: Scalar,
}
fn challenge(y: Point, k: Point) -> Scalar {
    let mut h = Sha256::new();
    h.update(b"YPP-accounts/research/register/v1");
    h.update(bytes(&y));
    h.update(bytes(&k));
    Scalar::from_le_bytes_mod_order(&h.finalize())
}
impl Registration {
    pub fn create(x: Scalar) -> Self {
        let key = (g() * x).into_affine();
        let k = random();
        let commitment = (g() * k).into_affine();
        Self {
            key,
            commitment,
            response: k + challenge(key, commitment) * x,
        }
    }
    pub fn verify(&self) -> bool {
        [self.key, self.commitment].iter().all(|p| {
            !p.is_zero() && p.is_on_curve() && p.is_in_correct_subgroup_assuming_on_curve()
        }) && g() * self.response
            == self.commitment.into_group() + self.key * challenge(self.key, self.commitment)
    }
}
pub fn fixture(n: usize) -> (Ledger, Vec<Scalar>) {
    let secrets = (0..n).map(|_| random()).collect::<Vec<_>>();
    let registrations = secrets
        .iter()
        .map(|x| Registration::create(*x))
        .collect::<Vec<_>>();
    let balances = registrations
        .iter()
        .map(|r| encrypt(r.key, Scalar::from(1000u32), random()))
        .collect();
    (
        Ledger::registered_genesis(registrations, balances).unwrap(),
        secrets,
    )
}
impl Ledger {
    /// Trusted experimental genesis; registration proves keys, not initial funding/ranges.
    pub fn registered_genesis(
        registrations: Vec<Registration>,
        balances: Vec<Cipher>,
    ) -> Result<Self, String> {
        if registrations.iter().any(|r| !r.verify()) {
            return Err("invalid proof of key possession".into());
        }
        Self::new(registrations.iter().map(|r| r.key).collect(), balances)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ark_relations::r1cs::ConstraintSystem;
    fn satisfied(r: Relation) -> bool {
        let cs = ConstraintSystem::new_ref();
        r.generate_constraints(cs.clone()).unwrap();
        cs.is_satisfied().unwrap()
    }
    #[test]
    fn statement_adversaries() {
        let (l, x) = fixture(4);
        let r = l.relation(0, 1, x[0], 1000, 7);
        assert!(satisfied(r.clone()));
        let mut bad = r.clone();
        bad.witness.secret = x[2];
        assert!(!satisfied(bad));
        let mut bad = r.clone();
        bad.witness.remaining = 1000;
        assert!(!satisfied(bad));
        let mut bad = r.clone();
        bad.witness.recipient = 0;
        assert!(!satisfied(bad));
        let mut bad = r.clone();
        bad.statement.changes[2] = (bad.statement.changes[2] + g()).into_affine();
        assert!(!satisfied(bad));
        let mut bad = r.clone();
        bad.statement.tag = (bad.statement.tag + g()).into_affine();
        assert!(!satisfied(bad));
        let mut bad = r.clone();
        bad.witness.amount = 0;
        assert!(!satisfied(bad));
        let mut bad = r.clone();
        bad.statement.epoch += 1;
        assert!(!satisfied(bad));
        let mut bad = r;
        bad.witness.remaining = u32::MAX;
        assert!(!satisfied(bad));
    }
    #[test]
    fn registration_and_recovery() {
        let (mut l, x) = fixture(4);
        let mut r = Registration::create(x[0]);
        r.key = (g() * x[1]).into_affine();
        assert!(!r.verify());
        let r = Registration::create(Scalar::zero());
        assert!(!r.verify());
        assert!(Ledger::registered_genesis(
            vec![r.clone(), r],
            vec![(Point::zero(), Point::zero()); 2]
        )
        .is_err());
        assert!(Ledger::new(vec![l.keys[0]; 2], vec![l.settled[0]; 2]).is_err());
        let mut low_order = Registration::create(x[0]);
        low_order.key = Point::new_unchecked(Fr::zero(), -Fr::one());
        assert!(!low_order.verify());
        let root = l.root();
        assert!(l.recover(root, 1, x[0], 2000).is_err());
        assert_eq!(l.recover(root, 1, x[1], 2000).unwrap(), 1000);
        l.pending[1].0 = (l.pending[1].0 + g()).into_affine();
        assert!(l.recover(root, 1, x[1], 2000).is_err());
        assert_eq!(
            decode((g() * Scalar::from(u32::MAX)).into_affine(), u32::MAX),
            Some(u32::MAX)
        );
    }
    #[test]
    fn real_proofs_concurrent_credits_conflicts_persistence() {
        let (mut l, x) = fixture(4);
        let a = l.relation(0, 1, x[0], 1000, 7);
        let pk = setup(a.clone());
        let pa = prove(&pk, a.clone());
        let b = l.relation(2, 1, x[2], 1000, 9);
        let pb = prove(&pk, b.clone());
        let conflict = l.relation(0, 3, x[0], 1000, 11);
        let pc = prove(&pk, conflict.clone());
        let vk =
            VerifyingKey::<Bls12_381>::deserialize_compressed(bytes(&pk.vk).as_slice()).unwrap();
        let mut tampered = a.statement.clone();
        tampered.changes[1] = (tampered.changes[1] + g()).into_affine();
        let before = bytes(&l);
        assert!(l.apply(&vk, &tampered, &pa).is_err());
        assert_eq!(before, bytes(&l));
        let mut reverse = l.clone();
        reverse.apply(&vk, &b.statement, &pb).unwrap();
        reverse.apply(&vk, &a.statement, &pa).unwrap();
        l.apply(&pk.vk, &a.statement, &pa).unwrap();
        l.apply(&pk.vk, &b.statement, &pb).unwrap();
        assert_eq!(l.pending, reverse.pending);
        assert!(l.apply(&pk.vk, &a.statement, &pa).is_err());
        assert!(l.apply(&pk.vk, &conflict.statement, &pc).is_err());
        let state = bytes(&l);
        let mut l = Ledger::deserialize_compressed(state.as_slice()).unwrap();
        let root = l.root();
        assert_eq!(l.recover(root, 1, x[1], 2000).unwrap(), 1016);
        assert_eq!(l.recover(root, 0, x[0], 2000).unwrap(), 993);
        l.rollover();
        assert_eq!(l.recover(l.root(), 1, x[1], 2000).unwrap(), 1016);
        assert!(l.apply(&pk.vk, &a.statement, &pa).is_err());
        let c = l.relation(1, 3, x[1], 1016, 1001);
        let proof = prove(&pk, c.clone());
        assert!(verify(&pk.vk, &c.statement, &proof));
        l.apply(&pk.vk, &c.statement, &proof).unwrap();
        assert_eq!(l.recover(l.root(), 1, x[1], 2000).unwrap(), 15);
        let mut tampered = c.statement;
        tampered.d = (tampered.d + g()).into_affine();
        assert!(!verify(&pk.vk, &tampered, &proof));
    }
}

#[cfg(test)]
mod observer_tests {
    use super::*;
    #[test]
    fn observer_owned_decoys_are_cryptographically_excludable() {
        for n in [16, 64, 128] {
            let (l, x) = fixture(n);
            let r = l.relation(0, 1, x[0], 1000, 7);
            for (i, secret) in x.iter().enumerate().skip(2) {
                assert!((r.statement.changes[i] - r.statement.d * secret).is_zero());
            }
            assert_eq!(
                (r.statement.changes[1] - r.statement.d * x[1]).into_affine(),
                (g() * Scalar::from(7u32)).into_affine()
            );
            assert_eq!(
                (r.statement.changes[0] - r.statement.d * x[0]).into_affine(),
                (-g() * Scalar::from(7u32)).into_affine()
            );
        }
    }
}
