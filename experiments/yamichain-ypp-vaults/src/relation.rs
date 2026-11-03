//! A single relation for debit-to-inbox and absorb-from-inbox. SHA-256 is used
//! unchanged, including in the circuit. All integers have 64 constrained bits.
use crate::{hash, Credit, Digest, State};
use ark_bls12_381::Fr;
use ark_crypto_primitives::crh::sha256::constraints::Sha256Gadget;
use ark_r1cs_std::{
    alloc::AllocVar, boolean::Boolean, eq::EqGadget, fields::fp::FpVar, prelude::ToBitsGadget,
    uint8::UInt8,
};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};

#[derive(Clone)]
pub(crate) struct Relation {
    pub debit: bool,
    pub old: State,
    pub new: State,
    pub secret: Digest,
    pub credit: Credit,
    pub ciphertext_hash: Digest,
    pub context: Digest,
    pub binding: Digest,
    pub old_commitment: Digest,
    pub new_commitment: Digest,
    pub credit_commitment: Digest,
}

pub(crate) const PUBLIC_INPUTS: usize = 1 + 6 * 32;

// Exactly the same ordering as public allocation below. One field per byte:
// deliberately simple, canonical, and not relying on field reduction of digests.
pub(crate) fn inputs(
    debit: bool,
    old: Digest,
    new: Digest,
    credit: Digest,
    ct: Digest,
    context: Digest,
    binding: Digest,
) -> Vec<Fr> {
    std::iter::once(Fr::from(u64::from(debit)))
        .chain(
            [old, new, credit, ct, context, binding]
                .into_iter()
                .flatten()
                .map(|b| Fr::from(u64::from(b))),
        )
        .collect()
}

fn bytes(cs: ConstraintSystemRef<Fr>, value: &[u8]) -> Result<Vec<UInt8<Fr>>, SynthesisError> {
    UInt8::new_witness_vec(cs, value)
}
fn public_digest(
    cs: ConstraintSystemRef<Fr>,
    value: Digest,
) -> Result<Vec<UInt8<Fr>>, SynthesisError> {
    // UInt8::new_input packs bytes. Allocate fields individually to match inputs().
    value
        .into_iter()
        .map(|b| {
            let f = FpVar::new_input(cs.clone(), || Ok(Fr::from(u64::from(b))))?;
            let byte = UInt8::new_witness(cs.clone(), || Ok(b))?;
            Boolean::le_bits_to_fp(&byte.to_bits_le()?)?.enforce_equal(&f)?;
            Ok(byte)
        })
        .collect()
}
fn digest(data: &[UInt8<Fr>]) -> Result<Vec<UInt8<Fr>>, SynthesisError> {
    Ok(Sha256Gadget::digest(data)?.0)
}
fn integer(
    cs: ConstraintSystemRef<Fr>,
    value: u64,
) -> Result<(Vec<UInt8<Fr>>, FpVar<Fr>), SynthesisError> {
    let b = bytes(cs, &value.to_le_bytes())?;
    let bits = b
        .iter()
        .map(ToBitsGadget::to_bits_le)
        .collect::<Result<Vec<_>, _>>()?
        .concat();
    Ok((b, Boolean::le_bits_to_fp(&bits)?))
}
fn state_digest(
    owner: &[UInt8<Fr>],
    view: &[UInt8<Fr>],
    balance: &[UInt8<Fr>],
    version: &[UInt8<Fr>],
    blind: &[UInt8<Fr>],
) -> Result<Vec<UInt8<Fr>>, SynthesisError> {
    digest(
        &[
            vec![UInt8::constant(1)],
            owner.to_vec(),
            view.to_vec(),
            balance.to_vec(),
            version.to_vec(),
            blind.to_vec(),
        ]
        .concat(),
    )
}
impl ConstraintSynthesizer<Fr> for Relation {
    fn generate_constraints(self, cs: ConstraintSystemRef<Fr>) -> Result<(), SynthesisError> {
        let mode = Boolean::new_input(cs.clone(), || Ok(self.debit))?;
        let old_public = public_digest(cs.clone(), self.old_commitment)?;
        let new_public = public_digest(cs.clone(), self.new_commitment)?;
        let credit_public = public_digest(cs.clone(), self.credit_commitment)?;
        let ct = public_digest(cs.clone(), self.ciphertext_hash)?;
        let context = public_digest(cs.clone(), self.context)?;
        let binding = public_digest(cs.clone(), self.binding)?;
        let secret = bytes(cs.clone(), &self.secret)?;
        digest(&[vec![UInt8::constant(5)], secret.clone(), context].concat())?
            .enforce_equal(&binding)?;
        let owner = digest(&[vec![UInt8::constant(0)], secret].concat())?;
        let view = bytes(cs.clone(), &self.old.view)?;
        // New owner and viewing authority are necessarily unchanged.
        let (old_balance, ob) = integer(cs.clone(), self.old.balance)?;
        let (new_balance, nb) = integer(cs.clone(), self.new.balance)?;
        let (old_version, ov) = integer(cs.clone(), self.old.version)?;
        let (new_version, nv) = integer(cs.clone(), self.new.version)?;
        (&ov + Fr::from(1u64)).enforce_equal(&nv)?;
        let old_blind = bytes(cs.clone(), &self.old.blind)?;
        let new_blind = bytes(cs.clone(), &self.new.blind)?;
        state_digest(&owner, &view, &old_balance, &old_version, &old_blind)?
            .enforce_equal(&old_public)?;
        state_digest(&owner, &view, &new_balance, &new_version, &new_blind)?
            .enforce_equal(&new_public)?;
        let recipient = bytes(cs.clone(), &self.credit.owner)?;
        let recipient_view = bytes(cs.clone(), &self.credit.view)?;
        let (amount, a) = integer(cs.clone(), self.credit.amount)?;
        a.is_eq(&FpVar::Constant(Fr::from(0u64)))?
            .enforce_equal(&Boolean::FALSE)?;
        let salt = bytes(cs, &self.credit.salt)?;
        digest(
            &[
                vec![UInt8::constant(2)],
                recipient.clone(),
                recipient_view.clone(),
                amount,
                salt,
                ct,
            ]
            .concat(),
        )?
        .enforce_equal(&credit_public)?;
        // Field is > 2^254 and each operand is 64 bits; these sums cannot wrap.
        let debit_balance = &nb + &a;
        let absorb_balance = &ob + &a;
        ob.conditional_enforce_equal(&debit_balance, &mode)?;
        nb.conditional_enforce_equal(&absorb_balance, &!mode.clone())?;
        recipient.conditional_enforce_equal(&owner, &!mode.clone())?;
        recipient_view.conditional_enforce_equal(&view, &!mode)?;
        Ok(())
    }
}

pub(crate) fn owner(secret: Digest) -> Digest {
    hash(&[&[0], &secret])
}

// Keep the setup shape independent of witness values.
pub(crate) fn blank() -> Relation {
    let secret = [7; 32];
    let old = State {
        owner: owner(secret),
        view: [8; 32],
        balance: 2,
        version: 0,
        blind: [9; 32],
    };
    let new = State {
        balance: 1,
        version: 1,
        blind: [10; 32],
        ..old.clone()
    };
    let credit = Credit {
        owner: [11; 32],
        view: [12; 32],
        amount: 1,
        salt: [13; 32],
    };
    let ct = [14; 32];
    Relation {
        debit: true,
        old_commitment: old.commitment(),
        new_commitment: new.commitment(),
        credit_commitment: credit.commitment(ct),
        old,
        new,
        secret,
        credit,
        ciphertext_hash: ct,
        context: [15; 32],
        binding: hash(&[&[5], &secret, &[15; 32]]),
    }
}
