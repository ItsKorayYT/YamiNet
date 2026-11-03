//! Rejected root-replacement candidate. Proofs are real; unsafe acceptance modes
//! exist only to reproduce concrete failures, never as a financial API.
use crate::{Error, Result};
use ark_bls12_381::{Bls12_381, Fr};
use ark_crypto_primitives::crh::{
    injective_map::{PedersenCRHCompressor, TECompressor},
    pedersen, CRHScheme,
};
use ark_ed_on_bls12_381::EdwardsProjective;
use ark_ff::PrimeField;
use ark_groth16::{prepare_verifying_key, Groth16, PreparedVerifyingKey, Proof, ProvingKey};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use hmac::{Hmac, Mac};
use rand::{rngs::OsRng, RngCore};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeSet,
    io::{Read, Write as _},
    sync::Arc,
};
use zeroize::{Zeroize, Zeroizing};

pub type Digest = [u8; 32];
pub const DEPTH: usize = 2;
pub const SLOTS: usize = 1 << DEPTH;
pub const PACKET_BYTES: usize = 288; // two fields, marker, 192-byte proof
const INPUTS: usize = 34;
#[derive(Clone)]
pub struct Window;
impl pedersen::Window for Window {
    const WINDOW_SIZE: usize = 128;
    const NUM_WINDOWS: usize = 5;
}
pub type Params = pedersen::Parameters<EdwardsProjective>;
pub type Hash = PedersenCRHCompressor<EdwardsProjective, TECompressor, Window>;
pub fn bytes<T: CanonicalSerialize>(v: &T) -> Vec<u8> {
    let mut b = Vec::new();
    v.serialize_compressed(&mut b)
        .expect("memory serialization");
    b
}
pub fn hash(p: &[&[u8]]) -> Digest {
    let mut h = Sha256::new();
    for b in p {
        h.update(b);
    }
    h.finalize().into()
}
fn random() -> Digest {
    let mut b = [0; 32];
    OsRng.fill_bytes(&mut b);
    b
}
pub(crate) fn owner(s: Digest) -> Digest {
    hash(&[b"YPP3-owner", &s])
}
pub(crate) fn marker(s: Digest, c: Digest) -> Digest {
    let mut h = Hmac::<Sha256>::new_from_slice(&s).expect("fixed HMAC key");
    h.update(b"YPP3-version");
    h.update(&c);
    h.finalize().into_bytes().into()
}
fn parent(params: &Params, a: Digest, b: Digest) -> Fr {
    Hash::evaluate(params, [&[7][..], &a, &b].concat().as_slice()).expect("fixed input")
}
#[derive(Clone)]
pub struct Vault {
    pub(crate) owner: Digest,
    pub(crate) balance: u64,
    pub(crate) version: u64,
    pub(crate) nonce: Digest,
}
impl Vault {
    pub fn balance(&self) -> u64 {
        self.balance
    }
    pub fn version(&self) -> u64 {
        self.version
    }
    pub fn commitment(&self) -> Digest {
        hash(&[
            b"YPP3-vault",
            &self.owner,
            &self.balance.to_le_bytes(),
            &self.version.to_le_bytes(),
            &self.nonce,
        ])
    }
}
pub struct Wallet {
    pub(crate) secret: Digest,
}
impl Drop for Wallet {
    fn drop(&mut self) {
        self.secret.zeroize();
    }
}
impl Wallet {
    pub fn new(balance: u64) -> (Self, Vault) {
        let secret = random();
        (
            Self { secret },
            Vault {
                owner: owner(secret),
                balance,
                version: 0,
                nonce: random(),
            },
        )
    }
    pub fn backup(&self, v: &Vault, to: &age::x25519::Recipient) -> Result<Vec<u8>> {
        if owner(self.secret) != v.owner {
            return Err(Error::Authorization);
        }
        let mut b = Zeroizing::new(b"YPP3-backup".to_vec());
        b.extend(self.secret);
        b.extend(v.balance.to_le_bytes());
        b.extend(v.version.to_le_bytes());
        b.extend(v.nonce);
        let enc = age::Encryptor::with_recipients(std::iter::once(to as &dyn age::Recipient))
            .map_err(|_| Error::Crypto)?;
        let mut out = Vec::new();
        let mut w = enc.wrap_output(&mut out).map_err(|_| Error::Crypto)?;
        w.write_all(&b).map_err(|_| Error::Crypto)?;
        w.finish().map_err(|_| Error::Crypto)?;
        Ok(out)
    }
    pub fn restore(id: &age::x25519::Identity, ct: &[u8]) -> Result<(Self, Vault)> {
        if ct.len() > 2048 {
            return Err(Error::Malformed);
        }
        let dec = age::Decryptor::new(ct).map_err(|_| Error::Recovery)?;
        let reader = dec
            .decrypt(std::iter::once(id as &dyn age::Identity))
            .map_err(|_| Error::Recovery)?;
        let mut b = Zeroizing::new(Vec::new());
        reader
            .take(2049)
            .read_to_end(&mut b)
            .map_err(|_| Error::Recovery)?;
        let prefix = b"YPP3-backup";
        if b.len() != prefix.len() + 80 || !b.starts_with(prefix) {
            return Err(Error::Recovery);
        }
        let b = &b[prefix.len()..];
        let secret = b[..32].try_into().map_err(|_| Error::Recovery)?;
        let balance = u64::from_le_bytes(b[32..40].try_into().map_err(|_| Error::Recovery)?);
        let version = u64::from_le_bytes(b[40..48].try_into().map_err(|_| Error::Recovery)?);
        let nonce = b[48..].try_into().map_err(|_| Error::Recovery)?;
        Ok((
            Self { secret },
            Vault {
                owner: owner(secret),
                balance,
                version,
                nonce,
            },
        ))
    }
}
#[derive(Clone)]
pub struct Snapshot {
    pub(crate) params: Arc<Params>,
    pub(crate) leaves: [Digest; SLOTS],
}
#[derive(Clone)]
pub(crate) struct Path {
    pub index: usize,
    pub siblings: [Digest; DEPTH],
}
impl Snapshot {
    pub fn new(params: Arc<Params>, vaults: &[Vault]) -> Result<Self> {
        if vaults.len() > SLOTS {
            return Err(Error::Capacity);
        }
        let mut leaves = [[0; 32]; SLOTS];
        for (i, v) in vaults.iter().enumerate() {
            leaves[i] = v.commitment();
        }
        Ok(Self { params, leaves })
    }
    fn layers(&self) -> Vec<Vec<Digest>> {
        let mut levels = vec![self.leaves.to_vec()];
        for i in 0..DEPTH {
            let next = levels[i]
                .chunks_exact(2)
                .map(|v| {
                    bytes(&parent(&self.params, v[0], v[1]))
                        .try_into()
                        .expect("field")
                })
                .collect();
            levels.push(next);
        }
        levels
    }
    pub fn root(&self) -> Fr {
        Fr::from_le_bytes_mod_order(&self.layers()[DEPTH][0])
    }
    pub(crate) fn path(&self, c: Digest) -> Result<Path> {
        let index = self
            .leaves
            .iter()
            .position(|a| *a == c)
            .ok_or(Error::Membership)?;
        let levels = self.layers();
        let mut i = index;
        let siblings = std::array::from_fn(|d| {
            let s = levels[d][i ^ 1];
            i /= 2;
            s
        });
        Ok(Path { index, siblings })
    }
    pub fn replace(&self, index: usize, c: Digest) -> Result<Self> {
        if index >= SLOTS {
            return Err(Error::Membership);
        }
        let mut next = self.clone();
        next.leaves[index] = c;
        Ok(next)
    }
    /// Public enumeration attack on the proposed "publish only the new leaf"
    /// repair. No owner secret, decryption key, or original opening is used.
    pub fn locate_published_write(&self, new: Digest, after: Fr) -> Vec<usize> {
        (0..SLOTS)
            .filter(|i| self.replace(*i, new).expect("bounded index").root() == after)
            .collect()
    }
    pub fn contains(&self, v: &Vault) -> bool {
        self.leaves.contains(&v.commitment())
    }
}
#[derive(Clone, CanonicalSerialize, CanonicalDeserialize)]
pub struct Write {
    pub before: Fr,
    pub after: Fr,
    pub marker: Digest,
    pub proof: Proof<Bls12_381>,
}
impl Write {
    pub fn encode(&self) -> Vec<u8> {
        bytes(self)
    }
    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() != PACKET_BYTES {
            return Err(Error::Malformed);
        }
        let mut data = b;
        let t = Self::deserialize_compressed(&mut data).map_err(|_| Error::Malformed)?;
        if !data.is_empty() {
            return Err(Error::Malformed);
        }
        Ok(t)
    }
    fn inputs(&self) -> Vec<Fr> {
        let mut v = vec![self.before, self.after];
        v.extend(self.marker.iter().map(|b| Fr::from(u64::from(*b))));
        v
    }
}
pub struct Prover {
    key: ProvingKey<Bls12_381>,
    params: Arc<Params>,
}
#[derive(Clone)]
pub struct Verifier {
    key: PreparedVerifyingKey<Bls12_381>,
}
impl Verifier {
    pub fn encode(&self) -> Vec<u8> {
        bytes(&self.key.vk)
    }
    pub fn decode(b: &[u8]) -> Result<Self> {
        let n = INPUTS + 1;
        if b.len() != 48 + 3 * 96 + 8 + n * 48
            || b.get(336..344) != Some(&(n as u64).to_le_bytes()[..])
        {
            return Err(Error::Malformed);
        }
        let mut data = b;
        let k = ark_groth16::VerifyingKey::<Bls12_381>::deserialize_compressed(&mut data)
            .map_err(|_| Error::Malformed)?;
        Ok(Self {
            key: prepare_verifying_key(&k),
        })
    }
    pub fn verify(&self, t: &Write) -> Result<()> {
        if Groth16::<Bls12_381>::verify_proof(&self.key, &t.proof, &t.inputs())
            .map_err(|_| Error::Crypto)?
        {
            Ok(())
        } else {
            Err(Error::Invalid)
        }
    }
}
pub(crate) fn statement(
    w: &Wallet,
    s: &Snapshot,
    old: &Vault,
) -> Result<(crate::relation::Relation, Vault, Snapshot)> {
    if owner(w.secret) != old.owner {
        return Err(Error::Authorization);
    }
    let path = s.path(old.commitment())?;
    let next = Vault {
        version: old.version.checked_add(1).ok_or(Error::Overflow)?,
        nonce: random(),
        ..old.clone()
    };
    let state = s.replace(path.index, next.commitment())?;
    let write = Write {
        before: s.root(),
        after: state.root(),
        marker: marker(w.secret, old.commitment()),
        proof: Proof::default(),
    };
    Ok((
        crate::relation::Relation {
            params: s.params.clone(),
            write,
            secret: w.secret,
            old: old.clone(),
            new: next.clone(),
            path,
        },
        next,
        state,
    ))
}
impl Prover {
    pub fn rotate(
        &self,
        w: &Wallet,
        s: &Snapshot,
        old: &Vault,
    ) -> Result<(Write, Vault, Snapshot)> {
        if !Arc::ptr_eq(&self.params, &s.params) {
            return Err(Error::Invalid);
        }
        let (r, next, state) = statement(w, s, old)?;
        let mut t = r.write.clone();
        t.proof =
            Groth16::<Bls12_381>::create_random_proof_with_reduction(r, &self.key, &mut OsRng)
                .map_err(|_| Error::Crypto)?;
        Ok((t, next, state))
    }
}
pub fn setup() -> Result<(Prover, Verifier, Arc<Params>, usize)> {
    use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystem};
    let params = Arc::new(Hash::setup(&mut OsRng).map_err(|_| Error::Crypto)?);
    let (w, v) = Wallet::new(u64::MAX);
    let snap = Snapshot::new(params.clone(), std::slice::from_ref(&v))?;
    let (r, _, _) = statement(&w, &snap, &v)?;
    let cs = ConstraintSystem::<Fr>::new_ref();
    r.clone()
        .generate_constraints(cs.clone())
        .map_err(|_| Error::Crypto)?;
    if !cs.is_satisfied().map_err(|_| Error::Crypto)? {
        return Err(Error::Invalid);
    }
    let count = cs.num_constraints();
    drop(cs);
    let key = Groth16::<Bls12_381>::generate_random_parameters_with_reduction(r, &mut OsRng)
        .map_err(|_| Error::Crypto)?;
    let verifier = Verifier {
        key: prepare_verifying_key(&key.vk),
    };
    Ok((
        Prover {
            key,
            params: params.clone(),
        },
        verifier,
        params,
        count,
    ))
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub enum Policy {
    Strict,
    UnsafeHistoricalReplacement,
    UnsafeFieldDelta,
}
#[derive(Clone)]
pub struct Record {
    pub write: Write,
    pub root: Fr,
    pub head: Digest,
}
#[derive(Clone)]
pub struct Node {
    pub root: Fr,
    pub(crate) policy: Policy,
    pub(crate) anchors: BTreeSet<Vec<u8>>,
    pub(crate) spent: BTreeSet<Digest>,
    pub records: Vec<Record>,
    pub head: Digest,
}
impl Node {
    pub fn new(root: Fr, policy: Policy) -> Self {
        Self {
            root,
            policy,
            anchors: BTreeSet::from([bytes(&root)]),
            spent: BTreeSet::new(),
            records: Vec::new(),
            head: hash(&[b"YPP3-genesis", &bytes(&root)]),
        }
    }
    pub fn apply(&mut self, v: &Verifier, t: Write) -> Result<()> {
        match self.policy {
            Policy::Strict => {
                if self.root != t.before {
                    return Err(Error::Stale);
                }
            }
            _ => {
                if !self.anchors.contains(&bytes(&t.before)) {
                    return Err(Error::Stale);
                }
            }
        }
        if self.spent.contains(&t.marker) {
            return Err(Error::Conflict);
        }
        v.verify(&t)?;
        self.root = match self.policy {
            Policy::UnsafeFieldDelta => self.root + t.after - t.before,
            _ => t.after,
        };
        self.anchors.insert(bytes(&self.root));
        self.spent.insert(t.marker);
        self.head = hash(&[b"YPP3-history", &self.head, &t.encode(), &bytes(&self.root)]);
        self.records.push(Record {
            write: t,
            root: self.root,
            head: self.head,
        });
        Ok(())
    }
    pub fn replay(&mut self, v: &Verifier, r: &Record) -> Result<()> {
        let mut n = self.clone();
        n.apply(v, r.write.clone())?;
        if n.root != r.root || n.head != r.head {
            return Err(Error::Recovery);
        }
        *self = n;
        Ok(())
    }
    pub fn require_head(&self, head: Digest) -> Result<()> {
        if self.head == head {
            Ok(())
        } else {
            Err(Error::Recovery)
        }
    }
}
