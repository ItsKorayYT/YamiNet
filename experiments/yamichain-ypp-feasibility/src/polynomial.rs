//! Standard KZG algebra and witness-update leakage experiments. These are NOT
//! transaction proofs or a proposed financial authorization scheme.
use crate::{merkle::bytes, Error, Result};
use ark_bls12_381::{Bls12_381, Fr};
use ark_ec::{AffineRepr, CurveGroup};
use ark_ff::{UniformRand, Zero};
use ark_poly::{
    univariate::DensePolynomial, EvaluationDomain, Evaluations, Radix2EvaluationDomain,
};
use ark_poly_commit::kzg10::{
    Commitment, Powers, Proof, Randomness, UniversalParams, VerifierKey, KZG10,
};
use ark_poly_commit::PCCommitmentState;
use rand::rngs::OsRng;
use std::{borrow::Cow, time::Instant};
type Poly = DensePolynomial<Fr>;
type Kzg = KZG10<Bls12_381, Poly>;
pub struct Experiment {
    params: UniversalParams<Bls12_381>,
    pub domain: Radix2EvaluationDomain<Fr>,
    pub vk: VerifierKey<Bls12_381>,
}
impl Experiment {
    pub fn new(n: usize) -> Result<Self> {
        if !n.is_power_of_two() || !(2..=4096).contains(&n) {
            return Err(Error::Capacity);
        }
        let params = Kzg::setup(n, false, &mut OsRng).map_err(|_| Error::Crypto)?;
        let vk = VerifierKey {
            g: params.powers_of_g[0],
            gamma_g: params.powers_of_gamma_g[&0],
            h: params.h,
            beta_h: params.beta_h,
            prepared_h: params.prepared_h.clone(),
            prepared_beta_h: params.prepared_beta_h.clone(),
        };
        let domain = Radix2EvaluationDomain::new(n).ok_or(Error::Capacity)?;
        Ok(Self { params, domain, vk })
    }
    fn powers(&self) -> Powers<'_, Bls12_381> {
        Powers {
            powers_of_g: Cow::Borrowed(&self.params.powers_of_g),
            powers_of_gamma_g: Cow::Owned(
                self.params.powers_of_gamma_g.values().copied().collect(),
            ),
        }
    }
    pub fn polynomial(&self, v: &[Fr]) -> Result<Poly> {
        if v.len() != self.domain.size() {
            return Err(Error::Capacity);
        }
        Ok(Evaluations::from_vec_and_domain(v.to_vec(), self.domain).interpolate())
    }
    pub fn commit(&self, p: &Poly) -> Result<Commitment<Bls12_381>> {
        Ok(Kzg::commit(&self.powers(), p, None, None)
            .map_err(|_| Error::Crypto)?
            .0)
    }
    pub fn open(&self, p: &Poly, i: usize) -> Result<Proof<Bls12_381>> {
        if i >= self.domain.size() {
            return Err(Error::Capacity);
        }
        Kzg::open(
            &self.powers(),
            p,
            self.domain.element(i),
            &Randomness::empty(),
        )
        .map_err(|_| Error::Crypto)
    }
    pub fn check(
        &self,
        c: &Commitment<Bls12_381>,
        i: usize,
        value: Fr,
        p: &Proof<Bls12_381>,
    ) -> bool {
        i < self.domain.size()
            && Kzg::check(&self.vk, c, self.domain.element(i), value, p).unwrap_or(false)
    }
    pub fn delta(&self, index: usize, value: Fr) -> Result<Poly> {
        if index >= self.domain.size() {
            return Err(Error::Capacity);
        }
        let mut evaluations = vec![Fr::zero(); self.domain.size()];
        evaluations[index] = value;
        self.polynomial(&evaluations)
    }
    pub fn hints(&self, delta: &Poly) -> Result<Vec<Proof<Bls12_381>>> {
        (0..self.domain.size())
            .map(|i| self.open(delta, i))
            .collect()
    }
    /// Attack uses only public delta commitment and proposed public refresh hints.
    /// It never receives the private write index, delta value, or setup trapdoor.
    pub fn infer_nonzero_updates(
        &self,
        delta: &Commitment<Bls12_381>,
        hints: &[Proof<Bls12_381>],
    ) -> Result<Vec<usize>> {
        if hints.len() != self.domain.size() {
            return Err(Error::Malformed);
        }
        Ok(hints
            .iter()
            .enumerate()
            .filter_map(|(i, p)| (!self.check(delta, i, Fr::zero(), p)).then_some(i))
            .collect())
    }
}
pub fn add(a: &Commitment<Bls12_381>, b: &Commitment<Bls12_381>) -> Commitment<Bls12_381> {
    Commitment((a.0.into_group() + b.0).into_affine())
}
pub fn refresh(a: &Proof<Bls12_381>, b: &Proof<Bls12_381>) -> Proof<Bls12_381> {
    Proof {
        w: (a.w.into_group() + b.w).into_affine(),
        random_v: None,
    }
}
#[derive(serde::Serialize)]
pub struct Measurement {
    pub entries: usize,
    pub setup_ms: f64,
    pub witness_hint_generation_ms: f64,
    pub attacker_scan_ms: f64,
    pub honest_witness_refresh_and_check_ms: f64,
    pub public_hint_bytes: usize,
    pub delta_commitment_bytes: usize,
    pub revealed_slots: Vec<usize>,
    pub independent_updates_commute: bool,
    pub stale_witness_rejected: bool,
    pub refreshed_witness_valid: bool,
    pub repeated_hidden_write_reidentified: bool,
    pub same_slot_blind_merge_is_not_a_version_update: bool,
}
pub fn run(n: usize) -> Result<Measurement> {
    let start = Instant::now();
    let exp = Experiment::new(n)?;
    let setup_ms = start.elapsed().as_secs_f64() * 1000.;
    let values = (0..n).map(|_| Fr::rand(&mut OsRng)).collect::<Vec<_>>();
    let p = exp.polynomial(&values)?;
    let c = exp.commit(&p)?;
    let a = exp.delta(0, Fr::from(7u64))?;
    let b = exp.delta(1, Fr::from(11u64))?;
    let da = exp.commit(&a)?;
    let db = exp.commit(&b)?;
    let combined = &(&p + &a) + &b;
    let correct = exp.commit(&combined)?;
    let independent_updates_commute =
        add(&add(&c, &da), &db) == correct && add(&add(&c, &db), &da) == correct;
    let old = exp.open(&p, 1)?;
    let after = add(&c, &da);
    let stale_witness_rejected = !exp.check(&after, 1, values[1], &old);
    let start = Instant::now();
    let hints = exp.hints(&a)?;
    let witness_hint_generation_ms = start.elapsed().as_secs_f64() * 1000.;
    let start = Instant::now();
    let refreshed = refresh(&old, &hints[1]);
    let refreshed_witness_valid = exp.check(&after, 1, values[1], &refreshed);
    let honest_witness_refresh_and_check_ms = start.elapsed().as_secs_f64() * 1000.;
    let start = Instant::now();
    let revealed_slots = exp.infer_nonzero_updates(&da, &hints)?;
    let attacker_scan_ms = start.elapsed().as_secs_f64() * 1000.;
    let again = exp.delta(0, Fr::from(13u64))?;
    let repeated_hidden_write_reidentified =
        exp.infer_nonzero_updates(&exp.commit(&again)?, &exp.hints(&again)?)? == revealed_slots;
    let conflict = exp.delta(0, Fr::from(17u64))?;
    let blind = add(&add(&c, &da), &exp.commit(&conflict)?);
    let same_slot_blind_merge_is_not_a_version_update =
        blind != after && blind != add(&c, &exp.commit(&conflict)?);
    Ok(Measurement {
        entries: n,
        setup_ms,
        witness_hint_generation_ms,
        attacker_scan_ms,
        honest_witness_refresh_and_check_ms,
        public_hint_bytes: hints.iter().map(|p| bytes(p).len()).sum(),
        delta_commitment_bytes: bytes(&da).len(),
        revealed_slots,
        independent_updates_commute,
        stale_witness_rejected,
        refreshed_witness_valid,
        repeated_hidden_write_reidentified,
        same_slot_blind_merge_is_not_a_version_update,
    })
}
