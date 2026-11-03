use crate::{Error, Result};
use ark_crypto_primitives::{
    crh::{
        injective_map::{InjectiveMap, TECompressor},
        pedersen,
    },
    encryption::{elgamal, AsymmetricEncryptionScheme},
};
use ark_ec::{AffineRepr, CurveGroup, PrimeGroup};
use ark_ed_on_bls12_381::{EdwardsAffine as Point, EdwardsProjective as Group, Fr as Scalar};
use ark_ff::{UniformRand, Zero};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use hmac::{Hmac, Mac};
use rand::rngs::OsRng;
use sha2::{Digest as _, Sha256};
use std::collections::HashMap;

pub type Digest = [u8; 32];
pub type Cipher = (Point, Point);
pub type Enc = elgamal::ElGamal<Group>;
#[derive(Clone)]
pub struct Window;
impl pedersen::Window for Window {
    const WINDOW_SIZE: usize = 128;
    const NUM_WINDOWS: usize = 5;
}
pub type TreeParams = pedersen::Parameters<Group>;
pub type TreeHash =
    ark_crypto_primitives::crh::injective_map::PedersenCRHCompressor<Group, TECompressor, Window>;
pub fn hash(parts: &[&[u8]]) -> Digest {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}
pub fn scalar() -> Scalar {
    loop {
        let s = Scalar::rand(&mut OsRng);
        if !s.is_zero() {
            return s;
        }
    }
}
pub fn point() -> Point {
    (Group::generator() * scalar()).into_affine()
}
pub fn bytes<T: CanonicalSerialize>(v: &T) -> Vec<u8> {
    let mut b = Vec::new();
    v.serialize_compressed(&mut b)
        .expect("memory serialization");
    b
}
pub fn scalar_bytes(s: &Scalar) -> Digest {
    bytes(s).try_into().expect("32 byte scalar")
}
pub fn xbytes(p: &Point) -> Digest {
    let x = TECompressor::injective_map(p).expect("prime order point");
    bytes(&x).try_into().expect("32 byte field")
}
pub fn nf(key: &Scalar, c: Digest) -> Digest {
    let mut m = Hmac::<Sha256>::new_from_slice(&scalar_bytes(key)).expect("HMAC accepts fixed key");
    m.update(b"YPP2-nullifier-v1");
    m.update(&c);
    m.finalize().into_bytes().into()
}
pub fn params() -> elgamal::Parameters<Group> {
    elgamal::Parameters {
        generator: Group::generator().into_affine(),
    }
}
pub fn encrypt(pk: Point, m: Point, r: Scalar) -> Cipher {
    Enc::encrypt(&params(), &pk, &m, &elgamal::Randomness(r)).expect("group encryption")
}
pub fn decrypt(sk: Scalar, c: &Cipher) -> Point {
    Enc::decrypt(&params(), &elgamal::SecretKey(sk), c).expect("group decryption")
}
pub fn valid(p: &Point) -> bool {
    p.is_on_curve() && p.is_in_correct_subgroup_assuming_on_curve()
}

/// Bounded-u16 ElGamal plaintext decoding by standard baby-step/giant-step.
/// This limit is a prototype limit, not monetary policy or a 64-bit decoder.
pub struct Decoder {
    baby: HashMap<Vec<u8>, u16>,
    stride: Group,
}
impl Default for Decoder {
    fn default() -> Self {
        let g = Group::generator();
        let mut cur = Group::zero();
        let mut baby = HashMap::new();
        for i in 0..256 {
            baby.insert(bytes(&cur.into_affine()), i);
            cur += g;
        }
        Self { baby, stride: cur }
    }
}
impl Decoder {
    pub fn amount(&self, p: Point) -> Option<u16> {
        let mut q = p.into_group();
        for j in 0..256u16 {
            if let Some(i) = self.baby.get(&bytes(&q.into_affine())) {
                return Some(j * 256 + *i);
            }
            q -= self.stride;
        }
        None
    }
}

#[derive(Clone)]
pub struct Opening {
    pub(crate) owner: Point,
    pub(crate) view: Point,
    pub(crate) amount: u16,
    pub(crate) salt: Point,
    pub(crate) credit: bool,
}
impl Opening {
    pub fn amount(&self) -> u16 {
        self.amount
    }
    pub fn commitment(&self) -> Digest {
        hash(&[
            b"YPP2-record-v1",
            &[u8::from(self.credit)],
            &xbytes(&self.owner),
            &xbytes(&self.view),
            &self.amount.to_le_bytes(),
            &xbytes(&self.salt),
        ])
    }
}
#[derive(Clone, CanonicalSerialize, CanonicalDeserialize)]
pub struct Output {
    pub commitment: Digest,
    pub delivery: [Cipher; 3],
}
impl Output {
    pub(crate) fn create(o: &Opening) -> (Self, [Scalar; 3]) {
        let rs = [scalar(), scalar(), scalar()];
        let amount = (Group::generator() * Scalar::from(u64::from(o.amount))).into_affine();
        (
            Self {
                commitment: o.commitment(),
                delivery: [
                    encrypt(o.view, amount, rs[0]),
                    encrypt(o.view, o.salt, rs[1]),
                    encrypt(o.view, o.owner, rs[2]),
                ],
            },
            rs,
        )
    }
    pub(crate) fn recover(
        &self,
        view: Scalar,
        credit: bool,
        decoder: &Decoder,
    ) -> Result<Option<Opening>> {
        let Some(amount) = decoder.amount(decrypt(view, &self.delivery[0])) else {
            return Ok(None);
        };
        let o = Opening {
            owner: decrypt(view, &self.delivery[2]),
            view: (Group::generator() * view).into_affine(),
            amount,
            salt: decrypt(view, &self.delivery[1]),
            credit,
        };
        if o.commitment() != self.commitment {
            return Ok(None);
        }
        if !valid(&o.owner) || o.owner.is_zero() || o.salt.is_zero() {
            return Err(Error::Recovery);
        }
        Ok(Some(o))
    }
}
