//! Milestone 2: hidden membership, persistent logical vault versions, and explicit
//! temporary note-like inbox records. Isolated synthetic experiment, not consensus.
#![forbid(unsafe_code)]
mod crypto;
mod relation;
mod tree;
use ark_bls12_381::{Bls12_381, Fr};
use ark_crypto_primitives::crh::CRHScheme;
use ark_ec::{CurveGroup, PrimeGroup};
use ark_ed_on_bls12_381::{EdwardsAffine as Point, EdwardsProjective as Group, Fr as Scalar};
use ark_ff::Zero;
use ark_groth16::{prepare_verifying_key, Groth16, PreparedVerifyingKey, Proof, ProvingKey};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use crypto::*;
pub use crypto::{Decoder, Digest, Opening, Output};
use rand::rngs::OsRng;
use std::{
    collections::{BTreeSet, VecDeque},
    io::{Read, Write},
    sync::Arc,
};
use tree::{Tree, Witness, DEPTH};
pub use tree::{CAPACITY, ROOT_WINDOW};
use zeroize::Zeroize;

pub type Result<T, E = Error> = std::result::Result<T, E>;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Malformed,
    Membership,
    Authorization,
    Overflow,
    Spent,
    Expired,
    Capacity,
    Recovery,
    Crypto,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
#[derive(Clone)]
pub struct Address {
    owner: Point,
    view: Point,
}
pub struct Wallet {
    spend: Scalar,
    view: Scalar,
}
impl Drop for Wallet {
    fn drop(&mut self) {
        self.spend.zeroize();
        self.view.zeroize();
    }
}
#[derive(Clone)]
pub struct Viewer {
    owner: Point,
    view: Scalar,
}
impl Drop for Viewer {
    fn drop(&mut self) {
        self.view.zeroize();
    }
}
impl Default for Wallet {
    fn default() -> Self {
        Self::new()
    }
}
impl Wallet {
    pub fn new() -> Self {
        Self {
            spend: scalar(),
            view: scalar(),
        }
    }
    pub fn address(&self) -> Address {
        Address {
            owner: (Group::generator() * self.spend).into_affine(),
            view: (Group::generator() * self.view).into_affine(),
        }
    }
    pub fn viewer(&self) -> Viewer {
        Viewer {
            owner: self.address().owner,
            view: self.view,
        }
    }
    fn opening(&self, amount: u16, credit: bool) -> Opening {
        let a = self.address();
        Opening {
            owner: a.owner,
            view: a.view,
            amount,
            salt: point(),
            credit,
        }
    }
    pub fn backup(&self, recipient: &age::x25519::Recipient) -> Result<Vec<u8>> {
        let mut data = zeroize::Zeroizing::new(b"YPP2-backup-v1".to_vec());
        data.extend_from_slice(&scalar_bytes(&self.spend));
        data.extend_from_slice(&scalar_bytes(&self.view));
        let encryptor =
            age::Encryptor::with_recipients(std::iter::once(recipient as &dyn age::Recipient))
                .map_err(|_| Error::Crypto)?;
        let mut out = Vec::new();
        let mut w = encryptor.wrap_output(&mut out).map_err(|_| Error::Crypto)?;
        w.write_all(&data).map_err(|_| Error::Crypto)?;
        w.finish().map_err(|_| Error::Crypto)?;
        Ok(out)
    }
    pub fn restore(identity: &age::x25519::Identity, ct: &[u8]) -> Result<Self> {
        if ct.len() > 2048 {
            return Err(Error::Malformed);
        }
        let d = age::Decryptor::new(ct).map_err(|_| Error::Recovery)?;
        let r = d
            .decrypt(std::iter::once(identity as &dyn age::Identity))
            .map_err(|_| Error::Recovery)?;
        let mut data = zeroize::Zeroizing::new(Vec::new());
        r.take(2049)
            .read_to_end(&mut data)
            .map_err(|_| Error::Recovery)?;
        let prefix = b"YPP2-backup-v1";
        if data.len() != prefix.len() + 64 || !data.starts_with(prefix) {
            return Err(Error::Recovery);
        }
        let mut b = &data[prefix.len()..];
        let spend = Scalar::deserialize_compressed(&mut b).map_err(|_| Error::Recovery)?;
        let view = Scalar::deserialize_compressed(&mut b).map_err(|_| Error::Recovery)?;
        if spend.is_zero() || view.is_zero() {
            return Err(Error::Recovery);
        }
        Ok(Self { spend, view })
    }
}

/// The complete observable public transcript. No old commitment, address, input
/// index, input kind/live flag, amount, or spent-credit identifier is published.
#[derive(Clone, CanonicalSerialize, CanonicalDeserialize)]
pub struct Transaction {
    pub anchor: Fr,
    pub nullifiers: [Digest; 2],
    pub outputs: [Output; 2],
    pub proof: Proof<Bls12_381>,
}
pub const PACKET_BYTES: usize = 740;
const INPUTS: usize = 153;
impl Transaction {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = b"YPP2".to_vec();
        self.serialize_compressed(&mut b)
            .expect("memory serialization");
        b
    }
    pub fn decode(data: &[u8]) -> Result<Self> {
        if data.len() != PACKET_BYTES || &data[..4] != b"YPP2" {
            return Err(Error::Malformed);
        }
        let mut b = &data[4..];
        let t = Self::deserialize_compressed(&mut b).map_err(|_| Error::Malformed)?;
        if !b.is_empty() {
            return Err(Error::Malformed);
        }
        Ok(t)
    }
    fn inputs(&self) -> Vec<Fr> {
        let mut v = vec![self.anchor];
        for nf in self.nullifiers {
            v.extend(nf.into_iter().map(|b| Fr::from(u64::from(b))));
        }
        for o in &self.outputs {
            v.extend(o.commitment.into_iter().map(|b| Fr::from(u64::from(b))));
            for (a, b) in o.delivery {
                v.extend([a.x, a.y, b.x, b.y]);
            }
        }
        v
    }
}
pub struct Prover {
    key: ProvingKey<Bls12_381>,
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
        let points = INPUTS + 1;
        let size = 48 + 96 * 3 + 8 + 48 * points;
        if b.len() != size || b.get(336..344) != Some(&(points as u64).to_le_bytes()[..]) {
            return Err(Error::Malformed);
        }
        let mut data = b;
        let key = ark_groth16::VerifyingKey::<Bls12_381>::deserialize_compressed(&mut data)
            .map_err(|_| Error::Malformed)?;
        if !data.is_empty() {
            return Err(Error::Malformed);
        }
        Ok(Self {
            key: prepare_verifying_key(&key),
        })
    }
    pub fn verify(&self, t: &Transaction) -> Result<()> {
        if t.nullifiers[0] == t.nullifiers[1] || t.outputs[0].commitment == t.outputs[1].commitment
        {
            return Err(Error::Invalid);
        }
        let mut ephemerals = BTreeSet::new();
        for o in &t.outputs {
            for c in &o.delivery {
                if !valid(&c.0) || !valid(&c.1) || c.0.is_zero() || !ephemerals.insert(bytes(&c.0))
                {
                    return Err(Error::Invalid);
                }
            }
        }
        if !Groth16::<Bls12_381>::verify_proof(&self.key, &t.proof, &t.inputs())
            .map_err(|_| Error::Invalid)?
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct Genesis {
    pub outputs: Vec<Output>,
    params: Arc<TreeParams>,
}
impl Genesis {
    pub fn checkpoint(&self) -> Digest {
        let mut b = bytes(self.params.as_ref());
        b.extend_from_slice(&(self.outputs.len() as u64).to_le_bytes());
        for o in &self.outputs {
            b.extend(bytes(o));
        }
        hash(&[b"YPP2-synthetic-genesis-v1", &b])
    }
}
pub fn synthetic_genesis(
    params: Arc<TreeParams>,
    wallets: &[&Wallet],
    balances: &[u16],
) -> Result<(Genesis, Vec<Opening>)> {
    if wallets.len() != balances.len() || wallets.is_empty() || wallets.len() > CAPACITY - 2 {
        return Err(Error::Capacity);
    }
    let mut owners = BTreeSet::new();
    let mut records = Vec::new();
    let mut outputs = Vec::new();
    for (w, b) in wallets.iter().zip(balances) {
        if !owners.insert(bytes(&w.address().owner)) {
            return Err(Error::Authorization);
        }
        let o = w.opening(*b, false);
        outputs.push(Output::create(&o).0);
        records.push(o);
    }
    Ok((Genesis { outputs, params }, records))
}
#[derive(Clone)]
pub struct Snapshot {
    tree: Tree,
}
impl Snapshot {
    pub fn root(&self) -> Fr {
        self.tree.root()
    }
}
#[derive(Clone)]
pub struct Record {
    pub transaction: Transaction,
    pub root: Fr,
    pub head: Digest,
}
#[derive(Clone)]
pub struct Ledger {
    tree: Tree,
    roots: VecDeque<Fr>,
    spent: BTreeSet<Digest>,
    commitments: BTreeSet<Digest>,
    records: Vec<Record>,
    head: Digest,
}
impl Ledger {
    pub fn from_genesis(g: &Genesis) -> Result<Self> {
        if g.outputs.is_empty() || g.outputs.len() > CAPACITY - 2 {
            return Err(Error::Capacity);
        }
        let mut tree = Tree::new(g.params.clone());
        let mut commitments = BTreeSet::new();
        for o in &g.outputs {
            if !commitments.insert(o.commitment) {
                return Err(Error::Invalid);
            }
            tree.append(o.commitment)?;
        }
        let root = tree.root();
        Ok(Self {
            tree,
            roots: VecDeque::from([root]),
            spent: BTreeSet::new(),
            commitments,
            records: Vec::new(),
            head: g.checkpoint(),
        })
    }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            tree: self.tree.clone(),
        }
    }
    pub fn root(&self) -> Fr {
        self.tree.root()
    }
    pub fn head(&self) -> Digest {
        self.head
    }
    pub fn records(&self) -> &[Record] {
        &self.records
    }
    pub fn leaf_count(&self) -> usize {
        self.tree.len()
    }
    pub fn marker_count(&self) -> usize {
        self.spent.len()
    }
    pub fn apply(&mut self, v: &Verifier, t: Transaction) -> Result<()> {
        if !self.roots.contains(&t.anchor) {
            return Err(Error::Expired);
        }
        if t.nullifiers.iter().any(|n| self.spent.contains(n)) {
            return Err(Error::Spent);
        }
        if self.tree.len() + 2 > CAPACITY {
            return Err(Error::Capacity);
        }
        if t.outputs
            .iter()
            .any(|o| self.commitments.contains(&o.commitment))
        {
            return Err(Error::Invalid);
        }
        v.verify(&t)?;
        // No fallible operation after mutation: capacity was checked above.
        for o in &t.outputs {
            self.tree.append(o.commitment).expect("checked capacity");
            self.commitments.insert(o.commitment);
        }
        self.spent.extend(t.nullifiers);
        let root = self.root();
        self.roots.push_back(root);
        if self.roots.len() > ROOT_WINDOW {
            self.roots.pop_front();
        }
        self.head = hash(&[b"YPP2-record-v1", &self.head, &t.encode(), &bytes(&root)]);
        self.records.push(Record {
            transaction: t,
            root,
            head: self.head,
        });
        Ok(())
    }
}

fn statement(
    params: Arc<TreeParams>,
    wallet: &Wallet,
    snapshot: &Snapshot,
    old: &Opening,
    incoming: Option<&Opening>,
    recipient: &Address,
    amount: u16,
) -> Result<(relation::Relation, Opening)> {
    let address = wallet.address();
    if old.credit || old.owner != address.owner || old.view != address.view {
        return Err(Error::Authorization);
    }
    let balance = (u32::from(old.amount) + incoming.map(|o| u32::from(o.amount)).unwrap_or(0))
        .checked_sub(u32::from(amount))
        .filter(|b| *b <= u32::from(u16::MAX))
        .ok_or(Error::Overflow)? as u16;
    let credit = incoming.cloned().unwrap_or_else(|| wallet.opening(0, true));
    if !credit.credit || credit.owner != address.owner || credit.view != address.view {
        return Err(Error::Authorization);
    }
    let first = snapshot.tree.locate(old.commitment())?;
    let second = if incoming.is_some() {
        snapshot.tree.locate(credit.commitment())?
    } else {
        Witness {
            index: 0,
            siblings: [[0; 32]; DEPTH],
        }
    };
    let new = Opening {
        amount: balance,
        salt: point(),
        ..old.clone()
    };
    let payment = Opening {
        owner: recipient.owner,
        view: recipient.view,
        amount,
        salt: point(),
        credit: true,
    };
    let (out0, r0) = Output::create(&new);
    let (out1, r1) = Output::create(&payment);
    let tx = Transaction {
        anchor: snapshot.root(),
        nullifiers: [
            nf(&wallet.view, old.commitment()),
            nf(&wallet.view, credit.commitment()),
        ],
        outputs: [out0, out1],
        proof: Proof::default(),
    };
    Ok((
        relation::Relation {
            params,
            tx,
            spend: scalar_bytes(&wallet.spend),
            view: scalar_bytes(&wallet.view),
            old: old.clone(),
            credit,
            live: incoming.is_some(),
            witnesses: [first, second],
            new: new.clone(),
            payment,
            randomness: [r0, r1],
        },
        new,
    ))
}
impl Prover {
    pub fn transfer(
        &self,
        w: &Wallet,
        snapshot: &Snapshot,
        old: &Opening,
        incoming: Option<&Opening>,
        recipient: &Address,
        amount: u16,
    ) -> Result<(Transaction, Opening)> {
        let (r, new) = statement(
            snapshot.tree.params.clone(),
            w,
            snapshot,
            old,
            incoming,
            recipient,
            amount,
        )?;
        let mut tx = r.tx.clone();
        tx.proof =
            Groth16::<Bls12_381>::create_random_proof_with_reduction(r, &self.key, &mut OsRng)
                .map_err(|_| Error::Crypto)?;
        Ok((tx, new))
    }
}
pub fn setup() -> Result<(Prover, Verifier, Arc<TreeParams>, usize)> {
    use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystem};
    let params = Arc::new(TreeHash::setup(&mut OsRng).map_err(|_| Error::Crypto)?);
    let w = Wallet::new();
    let b = Wallet::new();
    let (g, states) = synthetic_genesis(params.clone(), &[&w, &b], &[10, 0])?;
    let ledger = Ledger::from_genesis(&g)?;
    let (r, _) = statement(
        params.clone(),
        &w,
        &ledger.snapshot(),
        &states[0],
        None,
        &b.address(),
        1,
    )?;
    let cs = ConstraintSystem::<Fr>::new_ref();
    r.clone()
        .generate_constraints(cs.clone())
        .map_err(|_| Error::Crypto)?;
    if !cs.is_satisfied().map_err(|_| Error::Crypto)? {
        return Err(Error::Crypto);
    }
    let constraints = cs.num_constraints();
    drop(cs);
    let key = Groth16::<Bls12_381>::generate_random_parameters_with_reduction(r, &mut OsRng)
        .map_err(|_| Error::Crypto)?;
    let v = Verifier {
        key: prepare_verifying_key(&key.vk),
    };
    Ok((Prover { key }, v, params, constraints))
}

/// View-only authenticated history replay. It can derive nullifiers to remove
/// consumed openings, but cannot satisfy the independent spending-key relation.
#[derive(Clone)]
pub struct Scanner {
    ledger: Ledger,
    owned: Vec<Opening>,
    foreign: Vec<Opening>,
}
impl Scanner {
    pub fn new(view: &Viewer, g: &Genesis, pinned: Digest, d: &Decoder) -> Result<Self> {
        if g.checkpoint() != pinned {
            return Err(Error::Recovery);
        }
        let mut scan = Self {
            ledger: Ledger::from_genesis(g)?,
            owned: Vec::new(),
            foreign: Vec::new(),
        };
        for o in &g.outputs {
            scan.discover(view, o, false, d)?;
        }
        Ok(scan)
    }
    fn discover(&mut self, view: &Viewer, out: &Output, credit: bool, d: &Decoder) -> Result<()> {
        if let Some(o) = out.recover(view.view, credit, d)? {
            if o.owner == view.owner {
                self.owned.push(o);
            } else {
                self.foreign.push(o);
            }
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        view: &Viewer,
        v: &Verifier,
        record: &Record,
        d: &Decoder,
    ) -> Result<()> {
        let mut next = self.clone();
        next.ledger.apply(v, record.transaction.clone())?;
        if next.ledger.head() != record.head || next.ledger.root() != record.root {
            return Err(Error::Recovery);
        }
        next.owned.retain(|o| {
            !record
                .transaction
                .nullifiers
                .contains(&nf(&view.view, o.commitment()))
        });
        next.discover(view, &record.transaction.outputs[0], false, d)?;
        next.discover(view, &record.transaction.outputs[1], true, d)?;
        *self = next;
        Ok(())
    }
    pub fn require_head(&self, pinned: Digest) -> Result<()> {
        if self.ledger.head() == pinned {
            Ok(())
        } else {
            Err(Error::Recovery)
        }
    }
    pub fn cursor(&self) -> usize {
        self.ledger.records.len()
    }
    pub fn vault(&self) -> Result<&Opening> {
        let mut v = self.owned.iter().filter(|o| !o.credit);
        let first = v.next().ok_or(Error::Recovery)?;
        if v.next().is_some() {
            return Err(Error::Recovery);
        }
        Ok(first)
    }
    pub fn credits(&self) -> Vec<&Opening> {
        self.owned.iter().filter(|o| o.credit).collect()
    }
    pub fn misaddressed_count(&self) -> usize {
        self.foreign.len()
    }
}
#[cfg(test)]
mod tests;
