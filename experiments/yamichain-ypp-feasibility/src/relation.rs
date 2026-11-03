use crate::merkle::*;
use ark_bls12_381::Fr;
use ark_crypto_primitives::crh::{
    injective_map::{
        constraints::{PedersenCRHCompressorGadget, TECompressorGadget},
        TECompressor,
    },
    sha256::constraints::Sha256Gadget,
    CRHSchemeGadget,
};
use ark_ed_on_bls12_381::{constraints::EdwardsVar, EdwardsProjective};
use ark_r1cs_std::{boolean::Boolean, fields::fp::FpVar, prelude::*, uint8::UInt8};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
use std::sync::Arc;
type H = PedersenCRHCompressorGadget<
    EdwardsProjective,
    TECompressor,
    Window,
    EdwardsVar,
    TECompressorGadget,
>;
type R<T> = std::result::Result<T, SynthesisError>;
#[derive(Clone)]
pub(crate) struct Relation {
    pub params: Arc<Params>,
    pub write: Write,
    pub secret: Digest,
    pub old: Vault,
    pub new: Vault,
    pub path: Path,
}
fn sha(b: &[UInt8<Fr>]) -> R<Vec<UInt8<Fr>>> {
    Ok(Sha256Gadget::digest(b)?.0)
}
fn w(cs: ConstraintSystemRef<Fr>, b: &[u8]) -> R<Vec<UInt8<Fr>>> {
    UInt8::new_witness_vec(cs, b)
}
fn integer(cs: ConstraintSystemRef<Fr>, n: u64) -> R<(Vec<UInt8<Fr>>, FpVar<Fr>)> {
    let b = w(cs, &n.to_le_bytes())?;
    let bits = b
        .iter()
        .map(ToBitsGadget::to_bits_le)
        .collect::<R<Vec<_>>>()?
        .concat();
    let f = Boolean::le_bits_to_fp(&bits)?;
    Ok((b, f))
}
fn state(
    owner: &[UInt8<Fr>],
    balance: &[UInt8<Fr>],
    version: &[UInt8<Fr>],
    nonce: &[UInt8<Fr>],
) -> R<Vec<UInt8<Fr>>> {
    sha(&[
        UInt8::constant_vec(b"YPP3-vault"),
        owner.to_vec(),
        balance.to_vec(),
        version.to_vec(),
        nonce.to_vec(),
    ]
    .concat())
}
fn root(
    cs: ConstraintSystemRef<Fr>,
    params: &<H as CRHSchemeGadget<Hash, Fr>>::ParametersVar,
    mut c: Vec<UInt8<Fr>>,
    path: &Path,
) -> R<FpVar<Fr>> {
    let mut r = FpVar::Constant(Fr::from(0u64));
    for level in 0..DEPTH {
        let sibling = w(cs.clone(), &path.siblings[level])?;
        let bit = Boolean::new_witness(cs.clone(), || Ok(path.index & (1 << level) != 0))?;
        let left = sibling
            .iter()
            .zip(&c)
            .map(|(a, b)| UInt8::conditionally_select(&bit, a, b))
            .collect::<R<Vec<_>>>()?;
        let right = c
            .iter()
            .zip(&sibling)
            .map(|(a, b)| UInt8::conditionally_select(&bit, a, b))
            .collect::<R<Vec<_>>>()?;
        r = H::evaluate(params, &[vec![UInt8::constant(7)], left, right].concat())?;
        c = r.to_bytes_le()?;
    }
    Ok(r)
}
impl ConstraintSynthesizer<Fr> for Relation {
    fn generate_constraints(self, cs: ConstraintSystemRef<Fr>) -> R<()> {
        let before = FpVar::new_input(cs.clone(), || Ok(self.write.before))?;
        let after = FpVar::new_input(cs.clone(), || Ok(self.write.after))?;
        let marker = self
            .write
            .marker
            .into_iter()
            .map(|b| {
                let f = FpVar::new_input(cs.clone(), || Ok(Fr::from(u64::from(b))))?;
                let byte = UInt8::new_witness(cs.clone(), || Ok(b))?;
                Boolean::le_bits_to_fp(&byte.to_bits_le()?)?.enforce_equal(&f)?;
                Ok(byte)
            })
            .collect::<R<Vec<_>>>()?;
        let secret = w(cs.clone(), &self.secret)?;
        let owner = sha(&[UInt8::constant_vec(b"YPP3-owner"), secret.clone()].concat())?;
        let (b0, f0) = integer(cs.clone(), self.old.balance)?;
        let (b1, f1) = integer(cs.clone(), self.new.balance)?;
        f0.enforce_equal(&f1)?;
        let (v0, n0) = integer(cs.clone(), self.old.version)?;
        let (v1, n1) = integer(cs.clone(), self.new.version)?;
        (&n0 + Fr::from(1u64)).enforce_equal(&n1)?;
        let old_nonce = w(cs.clone(), &self.old.nonce)?;
        let new_nonce = w(cs.clone(), &self.new.nonce)?;
        let old = state(&owner, &b0, &v0, &old_nonce)?;
        let new = state(&owner, &b1, &v1, &new_nonce)?;
        let params = <H as CRHSchemeGadget<Hash, Fr>>::ParametersVar::new_constant(
            cs.clone(),
            self.params.as_ref(),
        )?;
        root(cs.clone(), &params, old.clone(), &self.path)?.enforce_equal(&before)?;
        root(cs, &params, new, &self.path)?.enforce_equal(&after)?;
        let mut padded = secret;
        padded.resize(64, UInt8::constant(0));
        let ip = padded
            .iter()
            .map(|b| b ^ UInt8::constant(0x36))
            .collect::<Vec<_>>();
        let op = padded
            .iter()
            .map(|b| b ^ UInt8::constant(0x5c))
            .collect::<Vec<_>>();
        let inner = sha(&[ip, UInt8::constant_vec(b"YPP3-version"), old].concat())?;
        sha(&[op, inner].concat())?.enforce_equal(&marker)?;
        Ok(())
    }
}
