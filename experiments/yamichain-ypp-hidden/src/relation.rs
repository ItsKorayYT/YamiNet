use crate::{
    crypto::*,
    tree::{Witness, DEPTH},
    Transaction,
};
use ark_bls12_381::Fr;
use ark_crypto_primitives::{
    crh::{
        injective_map::{
            constraints::{PedersenCRHCompressorGadget, TECompressorGadget},
            TECompressor,
        },
        sha256::constraints::Sha256Gadget,
        CRHSchemeGadget,
    },
    encryption::{
        elgamal::{self, constraints::ElGamalEncGadget},
        AsymmetricEncryptionGadget,
    },
};
use ark_ec::{CurveGroup, PrimeGroup};
use ark_ed_on_bls12_381::{constraints::EdwardsVar, EdwardsProjective as Group, Fr as Scalar};
use ark_ff::PrimeField;
use ark_r1cs_std::{boolean::Boolean, fields::fp::FpVar, prelude::*, uint8::UInt8};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
use std::sync::Arc;
type HashGadget =
    PedersenCRHCompressorGadget<Group, TECompressor, Window, EdwardsVar, TECompressorGadget>;
type EncGadget = ElGamalEncGadget<Group, EdwardsVar>;
type GResult<T> = std::result::Result<T, SynthesisError>;
type ScalarWitness = (Vec<UInt8<Fr>>, Vec<Boolean<Fr>>);
#[derive(Clone)]
pub(crate) struct Relation {
    pub params: Arc<TreeParams>,
    pub tx: Transaction,
    pub spend: [u8; 32],
    pub view: [u8; 32],
    pub old: Opening,
    pub credit: Opening,
    pub live: bool,
    pub witnesses: [Witness; 2],
    pub new: Opening,
    pub payment: Opening,
    pub randomness: [[Scalar; 3]; 2],
}
fn digest(b: &[UInt8<Fr>]) -> GResult<Vec<UInt8<Fr>>> {
    Ok(Sha256Gadget::digest(b)?.0)
}
fn witness_bytes(cs: ConstraintSystemRef<Fr>, v: &[u8]) -> GResult<Vec<UInt8<Fr>>> {
    UInt8::new_witness_vec(cs, v)
}
fn public_bytes(cs: ConstraintSystemRef<Fr>, v: Digest) -> GResult<Vec<UInt8<Fr>>> {
    v.into_iter()
        .map(|b| {
            let f = FpVar::new_input(cs.clone(), || Ok(Fr::from(u64::from(b))))?;
            let byte = UInt8::new_witness(cs.clone(), || Ok(b))?;
            Boolean::le_bits_to_fp(&byte.to_bits_le()?)?.enforce_equal(&f)?;
            Ok(byte)
        })
        .collect()
}
fn scalar_bits(cs: ConstraintSystemRef<Fr>, v: [u8; 32]) -> GResult<ScalarWitness> {
    let b = witness_bytes(cs, &v)?;
    let bits = b
        .iter()
        .map(ToBitsGadget::to_bits_le)
        .collect::<GResult<Vec<_>>>()?
        .concat();
    Boolean::enforce_smaller_or_equal_than_le(&bits, (-Scalar::from(1u64)).into_bigint())?;
    Boolean::kary_or(&bits)?.enforce_equal(&Boolean::TRUE)?;
    Ok((b, bits))
}
fn hmac(key: &[UInt8<Fr>], c: &[UInt8<Fr>]) -> GResult<Vec<UInt8<Fr>>> {
    let mut padded = key.to_vec();
    padded.resize(64, UInt8::constant(0));
    let ipad = padded
        .iter()
        .map(|b| b ^ UInt8::constant(0x36))
        .collect::<Vec<_>>();
    let opad = padded
        .iter()
        .map(|b| b ^ UInt8::constant(0x5c))
        .collect::<Vec<_>>();
    let inner = digest(&[ipad, UInt8::constant_vec(b"YPP2-nullifier-v1"), c.to_vec()].concat())?;
    digest(&[opad, inner].concat())
}
struct RecordVar {
    owner: EdwardsVar,
    view: EdwardsVar,
    salt: EdwardsVar,
    amount: FpVar<Fr>,
    commitment: Vec<UInt8<Fr>>,
}
fn record(cs: ConstraintSystemRef<Fr>, o: &Opening, credit: bool) -> GResult<RecordVar> {
    let owner = EdwardsVar::new_witness(cs.clone(), || Ok(o.owner))?;
    let view = EdwardsVar::new_witness(cs.clone(), || Ok(o.view))?;
    let salt = EdwardsVar::new_witness(cs.clone(), || Ok(o.salt))?;
    for p in [&owner, &view, &salt] {
        p.is_zero()?.enforce_equal(&Boolean::FALSE)?;
    }
    let amount_bytes = witness_bytes(cs, &o.amount.to_le_bytes())?;
    let bits = amount_bytes
        .iter()
        .map(ToBitsGadget::to_bits_le)
        .collect::<GResult<Vec<_>>>()?
        .concat();
    let amount = Boolean::le_bits_to_fp(&bits)?;
    let commitment = digest(
        &[
            UInt8::constant_vec(b"YPP2-record-v1"),
            vec![UInt8::constant(u8::from(credit))],
            owner.x.to_bytes_le()?,
            view.x.to_bytes_le()?,
            amount_bytes,
            salt.x.to_bytes_le()?,
        ]
        .concat(),
    )?;
    Ok(RecordVar {
        owner,
        view,
        salt,
        amount,
        commitment,
    })
}
fn membership(
    cs: ConstraintSystemRef<Fr>,
    p: &<HashGadget as CRHSchemeGadget<TreeHash, Fr>>::ParametersVar,
    c: Vec<UInt8<Fr>>,
    w: &Witness,
) -> GResult<FpVar<Fr>> {
    let mut node = c;
    let mut root = FpVar::Constant(Fr::from(0u64));
    for level in 0..DEPTH {
        let sibling = witness_bytes(cs.clone(), &w.siblings[level])?;
        let bit = Boolean::new_witness(cs.clone(), || Ok(w.index & (1 << level) != 0))?;
        let left = sibling
            .iter()
            .zip(&node)
            .map(|(a, b)| UInt8::conditionally_select(&bit, a, b))
            .collect::<GResult<Vec<_>>>()?;
        let right = node
            .iter()
            .zip(&sibling)
            .map(|(a, b)| UInt8::conditionally_select(&bit, a, b))
            .collect::<GResult<Vec<_>>>()?;
        root = HashGadget::evaluate(p, &[vec![UInt8::constant(9)], left, right].concat())?;
        node = root.to_bytes_le()?;
    }
    Ok(root)
}
impl ConstraintSynthesizer<Fr> for Relation {
    fn generate_constraints(self, cs: ConstraintSystemRef<Fr>) -> GResult<()> {
        let root = FpVar::new_input(cs.clone(), || Ok(self.tx.anchor))?;
        let nfs = [
            public_bytes(cs.clone(), self.tx.nullifiers[0])?,
            public_bytes(cs.clone(), self.tx.nullifiers[1])?,
        ];
        let mut publics = Vec::new();
        for out in &self.tx.outputs {
            let c = public_bytes(cs.clone(), out.commitment)?;
            let enc = out
                .delivery
                .iter()
                .map(|e| {
                    <EncGadget as AsymmetricEncryptionGadget<Enc, Fr>>::OutputVar::new_input(
                        cs.clone(),
                        || Ok(e),
                    )
                })
                .collect::<GResult<Vec<_>>>()?;
            publics.push((c, enc));
        }
        let (key, view_bits) = scalar_bits(cs.clone(), self.view)?;
        let (_, spend_bits) = scalar_bits(cs.clone(), self.spend)?;
        let generator = EdwardsVar::constant(Group::generator());
        let owner = generator.scalar_mul_le(spend_bits.iter())?;
        let view = generator.scalar_mul_le(view_bits.iter())?;
        let old = record(cs.clone(), &self.old, false)?;
        let credit = record(cs.clone(), &self.credit, true)?;
        let new = record(cs.clone(), &self.new, false)?;
        let payment = record(cs.clone(), &self.payment, true)?;
        for r in [&old, &credit, &new] {
            r.owner.enforce_equal(&owner)?;
            r.view.enforce_equal(&view)?;
        }
        (&old.amount + &credit.amount).enforce_equal(&(&new.amount + &payment.amount))?;
        let live = Boolean::new_witness(cs.clone(), || Ok(self.live))?;
        credit
            .amount
            .conditional_enforce_equal(&FpVar::Constant(Fr::from(0u64)), &!live.clone())?;
        let params = <HashGadget as CRHSchemeGadget<TreeHash, Fr>>::ParametersVar::new_constant(
            cs.clone(),
            self.params.as_ref(),
        )?;
        membership(
            cs.clone(),
            &params,
            old.commitment.clone(),
            &self.witnesses[0],
        )?
        .enforce_equal(&root)?;
        membership(
            cs.clone(),
            &params,
            credit.commitment.clone(),
            &self.witnesses[1],
        )?
        .conditional_enforce_equal(&root, &live)?;
        hmac(&key, &old.commitment)?.enforce_equal(&nfs[0])?;
        hmac(&key, &credit.commitment)?.enforce_equal(&nfs[1])?;
        let ep = <EncGadget as AsymmetricEncryptionGadget<Enc, Fr>>::ParametersVar::new_constant(
            cs.clone(),
            crate::crypto::params(),
        )?;
        let mut ephemerals = Vec::new();
        for (i, o) in [&new, &payment].into_iter().enumerate() {
            o.commitment.enforce_equal(&publics[i].0)?;
            let pk = <EncGadget as AsymmetricEncryptionGadget<Enc, Fr>>::PublicKeyVar::new_witness(
                cs.clone(),
                || {
                    Ok(if i == 0 {
                        self.new.view
                    } else {
                        self.payment.view
                    })
                },
            )?;
            pk.pk.enforce_equal(&o.view)?;
            let amount_bits = o.amount.to_bits_le()?;
            let amount = generator.scalar_mul_le(amount_bits.iter())?;
            for (j, msg) in [&amount, &o.salt, &o.owner].into_iter().enumerate() {
                let actual_msg = if j == 0 {
                    (Group::generator()
                        * Scalar::from(u64::from(if i == 0 {
                            self.new.amount
                        } else {
                            self.payment.amount
                        })))
                    .into_affine()
                } else if j == 1 {
                    if i == 0 {
                        self.new.salt
                    } else {
                        self.payment.salt
                    }
                } else if i == 0 {
                    self.new.owner
                } else {
                    self.payment.owner
                };
                let m =
                    <EncGadget as AsymmetricEncryptionGadget<Enc, Fr>>::PlaintextVar::new_witness(
                        cs.clone(),
                        || Ok(actual_msg),
                    )?;
                m.plaintext.enforce_equal(msg)?;
                let randomness =
                    <EncGadget as AsymmetricEncryptionGadget<Enc, Fr>>::RandomnessVar::new_witness(
                        cs.clone(),
                        || Ok(elgamal::Randomness::<Group>(self.randomness[i][j])),
                    )?;
                let encrypted = EncGadget::encrypt(&ep, &m, &randomness, &pk)?;
                encrypted.enforce_equal(&publics[i].1[j])?;
                encrypted.c1.is_zero()?.enforce_equal(&Boolean::FALSE)?;
                for prior in &ephemerals {
                    encrypted.c1.is_eq(prior)?.enforce_equal(&Boolean::FALSE)?;
                }
                ephemerals.push(encrypted.c1);
            }
        }
        Ok(())
    }
}
