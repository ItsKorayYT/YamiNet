//! Experimental, linkable persistent vaults over synthetic funds. Never consensus.
//! Fixed public slots intentionally expose update graphs. See README.md.
#![forbid(unsafe_code)]
mod relation;

use age::{secrecy::ExposeSecret, x25519};
use ark_bls12_381::{Bls12_381, Fr};
use ark_groth16::{prepare_verifying_key, Groth16, PreparedVerifyingKey, Proof, ProvingKey};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use rand::{rngs::OsRng, RngCore};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
};
use zeroize::{Zeroize, Zeroizing};

pub type Digest = [u8; 32];
pub type Result<T> = std::result::Result<T, Error>;
const MAX_CIPHERTEXT: usize = 2048;
const PROOF_BYTES: usize = 192;
const TX_BYTES: usize = 1 + 1 + 32 * 5 + 12 + MAX_CIPHERTEXT * 2 + PROOF_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Authorization,
    StaleRoot,
    Conflict,
    Overflow,
    Crypto,
    Recovery,
    Malformed,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}

pub(crate) fn hash(parts: &[&[u8]]) -> Digest {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}
fn random() -> Digest {
    let mut b = [0; 32];
    OsRng.fill_bytes(&mut b);
    b
}

#[derive(Clone, Zeroize)]
#[zeroize(drop)]
pub struct State {
    owner: Digest,
    view: Digest,
    balance: u64,
    version: u64,
    blind: Digest,
}
impl State {
    pub fn balance(&self) -> u64 {
        self.balance
    }
    pub fn version(&self) -> u64 {
        self.version
    }
    pub fn commitment(&self) -> Digest {
        hash(&[&[1], &self.encode()])
    }
    fn encode(&self) -> Vec<u8> {
        [
            &self.owner[..],
            &self.view,
            &self.balance.to_le_bytes(),
            &self.version.to_le_bytes(),
            &self.blind,
        ]
        .concat()
    }
    fn decode(b: &[u8]) -> Result<Self> {
        if b.len() != 112 {
            return Err(Error::Malformed);
        }
        Ok(Self {
            owner: b[0..32].try_into().map_err(|_| Error::Malformed)?,
            view: b[32..64].try_into().map_err(|_| Error::Malformed)?,
            balance: read_u64(&b[64..72])?,
            version: read_u64(&b[72..80])?,
            blind: b[80..112].try_into().map_err(|_| Error::Malformed)?,
        })
    }
}
fn read_u64(b: &[u8]) -> Result<u64> {
    Ok(u64::from_le_bytes(
        b.try_into().map_err(|_| Error::Malformed)?,
    ))
}

#[derive(Clone, Zeroize)]
#[zeroize(drop)]
pub struct Credit {
    owner: Digest,
    view: Digest,
    amount: u64,
    salt: Digest,
}
impl Credit {
    fn encode(&self) -> Vec<u8> {
        [
            &self.owner[..],
            &self.view,
            &self.amount.to_le_bytes(),
            &self.salt,
        ]
        .concat()
    }
    fn decode(b: &[u8]) -> Result<Self> {
        if b.len() != 104 {
            return Err(Error::Malformed);
        }
        Ok(Self {
            owner: b[..32].try_into().map_err(|_| Error::Malformed)?,
            view: b[32..64].try_into().map_err(|_| Error::Malformed)?,
            amount: read_u64(&b[64..72])?,
            salt: b[72..].try_into().map_err(|_| Error::Malformed)?,
        })
    }
    pub fn amount(&self) -> u64 {
        self.amount
    }
    pub fn commitment(&self, ct: Digest) -> Digest {
        hash(&[&[2], &self.encode(), &ct])
    }
}

#[derive(Clone)]
pub struct Address {
    owner: Digest,
    recipient: x25519::Recipient,
}
impl Address {
    fn view(&self) -> Digest {
        hash(&[&[3], self.recipient.to_string().as_bytes()])
    }
}

pub struct Wallet {
    secret: Zeroizing<Digest>,
    viewing: x25519::Identity,
}
impl Default for Wallet {
    fn default() -> Self {
        Self::new()
    }
}
impl Wallet {
    pub fn new() -> Self {
        Self {
            secret: Zeroizing::new(random()),
            viewing: x25519::Identity::generate(),
        }
    }
    pub fn address(&self) -> Address {
        Address {
            owner: relation::owner(*self.secret),
            recipient: self.viewing.to_public(),
        }
    }
    fn owns(&self, s: &State) -> bool {
        s.owner == self.address().owner && s.view == self.address().view()
    }
    pub fn viewing_authority(&self) -> ViewingAuthority {
        ViewingAuthority {
            owner: self.address().owner,
            viewing: self.viewing.clone(),
        }
    }
    fn initial(&self, balance: u64) -> State {
        let a = self.address();
        State {
            owner: a.owner,
            view: a.view(),
            balance,
            version: 0,
            blind: random(),
        }
    }
    /// Encrypt a portable secret backup to a separately retained age identity.
    pub fn backup(&self, recipient: &x25519::Recipient) -> Result<Vec<u8>> {
        let key = self.viewing.to_string();
        let mut data = Zeroizing::new(b"YPP-backup-v1".to_vec());
        data.extend_from_slice(&*self.secret);
        data.extend_from_slice(key.expose_secret().as_bytes());
        encrypt(recipient, &data)
    }
    pub fn restore(identity: &x25519::Identity, encrypted: &[u8]) -> Result<Self> {
        let data = decrypt(identity, encrypted)?;
        let prefix = b"YPP-backup-v1";
        if !data.starts_with(prefix) || data.len() != prefix.len() + 32 + 74 {
            return Err(Error::Recovery);
        }
        let secret = data[prefix.len()..prefix.len() + 32]
            .try_into()
            .map_err(|_| Error::Recovery)?;
        let text = std::str::from_utf8(&data[prefix.len() + 32..]).map_err(|_| Error::Recovery)?;
        Ok(Self {
            secret: Zeroizing::new(secret),
            viewing: text.parse().map_err(|_| Error::Recovery)?,
        })
    }
}
/// Can scan and recover openings, but cannot authorize a transition.
#[derive(Clone)]
pub struct ViewingAuthority {
    owner: Digest,
    viewing: x25519::Identity,
}
impl ViewingAuthority {
    fn view(&self) -> Digest {
        hash(&[&[3], self.viewing.to_public().to_string().as_bytes()])
    }
    fn owns(&self, s: &State) -> bool {
        s.owner == self.owner && s.view == self.view()
    }
}
fn encrypt(recipient: &x25519::Recipient, data: &[u8]) -> Result<Vec<u8>> {
    let e = age::Encryptor::with_recipients(std::iter::once(recipient as &dyn age::Recipient))
        .map_err(|_| Error::Crypto)?;
    let mut out = Vec::new();
    let mut w = e.wrap_output(&mut out).map_err(|_| Error::Crypto)?;
    w.write_all(data).map_err(|_| Error::Crypto)?;
    w.finish().map_err(|_| Error::Crypto)?;
    if out.len() > MAX_CIPHERTEXT {
        return Err(Error::Malformed);
    }
    Ok(out)
}
fn decrypt(identity: &x25519::Identity, encrypted: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    if encrypted.len() > MAX_CIPHERTEXT {
        return Err(Error::Malformed);
    }
    let d = age::Decryptor::new(encrypted).map_err(|_| Error::Recovery)?;
    let mut r = d
        .decrypt(std::iter::once(identity as &dyn age::Identity))
        .map_err(|_| Error::Recovery)?;
    let mut out = Zeroizing::new(Vec::new());
    // Read to EOF to authenticate the age final chunk; bound both directions.
    r.by_ref()
        .take(MAX_CIPHERTEXT as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|_| Error::Recovery)?;
    if out.len() > MAX_CIPHERTEXT {
        return Err(Error::Malformed);
    }
    Ok(out)
}

/// An ephemeral local trusted setup. Discarded toxic waste is not a ceremony.
/// Production use would require a reviewed setup lifecycle and independent audit.
pub struct Prover {
    key: ProvingKey<Bls12_381>,
}
#[derive(Clone)]
pub struct Verifier {
    key: PreparedVerifyingKey<Bls12_381>,
}
pub fn constraint_count() -> Result<usize> {
    use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystem};
    let cs = ConstraintSystem::<Fr>::new_ref();
    relation::blank()
        .generate_constraints(cs.clone())
        .map_err(|_| Error::Crypto)?;
    Ok(cs.num_constraints())
}
pub fn setup() -> Result<(Prover, Verifier)> {
    let key = Groth16::<Bls12_381>::generate_random_parameters_with_reduction(
        relation::blank(),
        &mut OsRng,
    )
    .map_err(|_| Error::Crypto)?;
    let vk = prepare_verifying_key(&key.vk);
    Ok((Prover { key }, Verifier { key: vk }))
}

/// All fields here are observable by a validator. No private opening is stored.
#[derive(Clone)]
pub struct Transaction {
    pub debit: bool,
    pub slot: u8,
    pub anchor: Digest,
    pub old: Digest,
    pub new: Digest,
    pub credit: Digest,
    pub credit_ciphertext: Vec<u8>,
    pub state_ciphertext: Vec<u8>,
    pub binding: Digest,
    pub proof: Vec<u8>,
}
impl Transaction {
    fn inputs(&self) -> Vec<Fr> {
        relation::inputs(
            self.debit,
            self.old,
            self.new,
            self.credit,
            hash(&[&self.credit_ciphertext]),
            self.context(),
            self.binding,
        )
    }
    fn context(&self) -> Digest {
        let mut b = vec![u8::from(self.debit), self.slot];
        for d in [self.anchor, self.old, self.new, self.credit] {
            b.extend_from_slice(&d);
        }
        for ct in [&self.credit_ciphertext, &self.state_ciphertext] {
            b.extend_from_slice(&(ct.len() as u32).to_le_bytes());
            b.extend_from_slice(ct);
        }
        hash(&[b"YPP-context-v1", &b])
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut b = vec![u8::from(self.debit), self.slot];
        for d in [self.anchor, self.old, self.new, self.credit, self.binding] {
            b.extend_from_slice(&d);
        }
        for v in [&self.credit_ciphertext, &self.state_ciphertext, &self.proof] {
            b.extend_from_slice(&(v.len() as u32).to_le_bytes());
            b.extend_from_slice(v);
        }
        b
    }
    pub fn decode(mut b: &[u8]) -> Result<Self> {
        if b.len() > TX_BYTES || b.len() < 130 {
            return Err(Error::Malformed);
        }
        let mode = take(&mut b, 1)?[0];
        if mode > 1 {
            return Err(Error::Malformed);
        }
        let slot = take(&mut b, 1)?[0];
        if slot > 1 {
            return Err(Error::Malformed);
        }
        let anchor = take_digest(&mut b)?;
        let old = take_digest(&mut b)?;
        let new = take_digest(&mut b)?;
        let credit = take_digest(&mut b)?;
        let binding = take_digest(&mut b)?;
        let credit_ciphertext = take_vec(&mut b, MAX_CIPHERTEXT)?;
        let state_ciphertext = take_vec(&mut b, MAX_CIPHERTEXT)?;
        let proof = take_vec(&mut b, PROOF_BYTES)?;
        if !b.is_empty() || proof.len() != PROOF_BYTES {
            return Err(Error::Malformed);
        }
        Ok(Self {
            debit: mode == 1,
            slot,
            anchor,
            old,
            new,
            credit,
            credit_ciphertext,
            state_ciphertext,
            binding,
            proof,
        })
    }
}
fn take<'a>(b: &mut &'a [u8], n: usize) -> Result<&'a [u8]> {
    if n > b.len() {
        return Err(Error::Malformed);
    }
    let (v, rest) = b.split_at(n);
    *b = rest;
    Ok(v)
}
fn take_digest(b: &mut &[u8]) -> Result<Digest> {
    take(b, 32)?.try_into().map_err(|_| Error::Malformed)
}
fn take_vec(b: &mut &[u8], max: usize) -> Result<Vec<u8>> {
    let n = u32::from_le_bytes(take(b, 4)?.try_into().map_err(|_| Error::Malformed)?) as usize;
    if n > max {
        return Err(Error::Malformed);
    }
    Ok(take(b, n)?.to_vec())
}

impl Verifier {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.key
            .vk
            .serialize_compressed(&mut out)
            .map_err(|_| Error::Crypto)?;
        Ok(out)
    }
    pub fn decode(data: &[u8]) -> Result<Self> {
        // BLS12-381 compressed VK: alpha G1, three G2 points, u64 vector
        // length, then gamma_abc G1 points. Bound the encoded vector length
        // before arkworks deserializes it, avoiding attacker-sized allocation.
        let expected_points = relation::PUBLIC_INPUTS + 1;
        let expected_bytes = 48 + 3 * 96 + 8 + expected_points * 48;
        if data.len() != expected_bytes
            || data.get(336..344) != Some(&(expected_points as u64).to_le_bytes()[..])
        {
            return Err(Error::Malformed);
        }
        let mut input = data;
        let vk = ark_groth16::VerifyingKey::<Bls12_381>::deserialize_compressed(&mut input)
            .map_err(|_| Error::Malformed)?;
        if !input.is_empty() || vk.gamma_abc_g1.len() != expected_points {
            return Err(Error::Malformed);
        }
        Ok(Self {
            key: prepare_verifying_key(&vk),
        })
    }
    pub fn verify(&self, t: &Transaction) -> Result<()> {
        if t.slot > 1
            || t.proof.len() != PROOF_BYTES
            || t.credit_ciphertext.len() > MAX_CIPHERTEXT
            || t.state_ciphertext.len() > MAX_CIPHERTEXT
        {
            return Err(Error::Malformed);
        }
        let p =
            Proof::<Bls12_381>::deserialize_compressed(&t.proof[..]).map_err(|_| Error::Invalid)?;
        if !Groth16::<Bls12_381>::verify_proof(&self.key, &p, &t.inputs())
            .map_err(|_| Error::Invalid)?
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

/// In-place two-leaf authenticated state. Requiring the exact current root
/// serializes updates; roots are not a substitute for a production consensus.
#[derive(Clone)]
pub struct Ledger {
    leaves: [Digest; 2],
    pending: BTreeSet<Digest>,
    seen: BTreeSet<Digest>,
    head: Digest,
    records: Vec<Record>,
}
#[derive(Clone)]
pub struct Genesis {
    pub leaves: [Digest; 2],
    pub ciphertexts: [Vec<u8>; 2],
}
impl Genesis {
    pub fn checkpoint(&self) -> Digest {
        hash(&[
            b"YPP-synthetic-genesis-v1",
            &self.leaves.concat(),
            &(self.ciphertexts[0].len() as u64).to_le_bytes(),
            &self.ciphertexts[0],
            &(self.ciphertexts[1].len() as u64).to_le_bytes(),
            &self.ciphertexts[1],
        ])
    }
}
#[derive(Clone)]
pub struct Record {
    pub transaction: Transaction,
    pub head: Digest,
    pub root: Digest,
}
impl Ledger {
    pub fn from_genesis(g: &Genesis) -> Self {
        Self {
            leaves: g.leaves,
            pending: BTreeSet::new(),
            seen: BTreeSet::new(),
            head: g.checkpoint(),
            records: Vec::new(),
        }
    }
    pub fn root(&self) -> Digest {
        hash(&[&[4], &self.leaves[0], &self.leaves[1]])
    }
    pub fn head(&self) -> Digest {
        self.head
    }
    pub fn records(&self) -> &[Record] {
        &self.records
    }
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
    pub fn marker_count(&self) -> usize {
        self.seen.len()
    }
    pub fn apply(&mut self, v: &Verifier, t: Transaction) -> Result<()> {
        if t.slot > 1 {
            return Err(Error::Malformed);
        }
        if t.anchor != self.root() {
            return Err(Error::StaleRoot);
        }
        if self.leaves[t.slot as usize] != t.old {
            return Err(Error::Conflict);
        }
        if t.debit && self.seen.contains(&t.credit) || !t.debit && !self.pending.contains(&t.credit)
        {
            return Err(Error::Conflict);
        }
        v.verify(&t)?;
        // Everything fallible precedes the atomic in-memory mutation.
        self.leaves[t.slot as usize] = t.new;
        if t.debit {
            self.seen.insert(t.credit);
            self.pending.insert(t.credit);
        } else {
            self.pending.remove(&t.credit);
        }
        self.head = hash(&[b"YPP-record-v1", &self.head, &t.encode(), &self.root()]);
        self.records.push(Record {
            transaction: t,
            head: self.head,
            root: self.root(),
        });
        Ok(())
    }
}
pub fn synthetic_genesis(
    alice: &Wallet,
    bob: &Wallet,
    balances: [u64; 2],
) -> Result<(Genesis, [State; 2])> {
    if alice.address().owner == bob.address().owner {
        return Err(Error::Invalid);
    }
    let states = [alice.initial(balances[0]), bob.initial(balances[1])];
    let g = Genesis {
        leaves: [states[0].commitment(), states[1].commitment()],
        ciphertexts: [
            encrypt(&alice.address().recipient, &states[0].encode())?,
            encrypt(&bob.address().recipient, &states[1].encode())?,
        ],
    };
    Ok((g, states))
}

impl Prover {
    pub fn debit(
        &self,
        wallet: &Wallet,
        ledger: &Ledger,
        slot: u8,
        old: &State,
        recipient: &Address,
        amount: u64,
    ) -> Result<(Transaction, State)> {
        if amount == 0 {
            return Err(Error::Invalid);
        }
        let balance = old.balance.checked_sub(amount).ok_or(Error::Overflow)?;
        let credit = Credit {
            owner: recipient.owner,
            view: recipient.view(),
            amount,
            salt: random(),
        };
        let ciphertext = encrypt(&recipient.recipient, &credit.encode())?;
        self.transition(wallet, ledger, slot, old, balance, credit, ciphertext, true)
    }
    pub fn absorb(
        &self,
        wallet: &Wallet,
        ledger: &Ledger,
        slot: u8,
        old: &State,
        incoming: &Incoming,
    ) -> Result<(Transaction, State)> {
        if !ledger.pending.contains(&incoming.commitment) {
            return Err(Error::Conflict);
        }
        let balance = old
            .balance
            .checked_add(incoming.credit.amount)
            .ok_or(Error::Overflow)?;
        if incoming.credit.owner != wallet.address().owner
            || incoming.credit.view != wallet.address().view()
        {
            return Err(Error::Authorization);
        }
        self.transition(
            wallet,
            ledger,
            slot,
            old,
            balance,
            incoming.credit.clone(),
            incoming.ciphertext.clone(),
            false,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn transition(
        &self,
        wallet: &Wallet,
        ledger: &Ledger,
        slot: u8,
        old: &State,
        balance: u64,
        credit: Credit,
        ciphertext: Vec<u8>,
        debit: bool,
    ) -> Result<(Transaction, State)> {
        if !wallet.owns(old) {
            return Err(Error::Authorization);
        }
        if slot > 1 || ledger.leaves[slot as usize] != old.commitment() {
            return Err(Error::Conflict);
        }
        let new = State {
            balance,
            version: old.version.checked_add(1).ok_or(Error::Overflow)?,
            blind: random(),
            ..old.clone()
        };
        let ct_hash = hash(&[&ciphertext]);
        let mut t = Transaction {
            debit,
            slot,
            anchor: ledger.root(),
            old: old.commitment(),
            new: new.commitment(),
            credit: credit.commitment(ct_hash),
            credit_ciphertext: ciphertext,
            state_ciphertext: encrypt(&wallet.address().recipient, &new.encode())?,
            binding: [0; 32],
            proof: Vec::new(),
        };
        t.binding = hash(&[&[5], &*wallet.secret, &t.context()]);
        let r = relation::Relation {
            debit,
            old: old.clone(),
            new: new.clone(),
            secret: *wallet.secret,
            credit,
            ciphertext_hash: ct_hash,
            old_commitment: t.old,
            new_commitment: t.new,
            credit_commitment: t.credit,
            context: t.context(),
            binding: t.binding,
        };
        let p = Groth16::<Bls12_381>::create_random_proof_with_reduction(r, &self.key, &mut OsRng)
            .map_err(|_| Error::Crypto)?;
        p.serialize_compressed(&mut t.proof)
            .map_err(|_| Error::Crypto)?;
        Ok((t, new))
    }
}

#[derive(Clone)]
pub struct Incoming {
    pub commitment: Digest,
    credit: Credit,
    ciphertext: Vec<u8>,
}
impl Incoming {
    pub fn amount(&self) -> u64 {
        self.credit.amount
    }
}
/// Recovery authenticates history from a pinned synthetic genesis and a trusted
/// final head, replaying every proof. No address or view key is sent to validators.
/// A truncated prefix can be resumed, but cannot be called current without a head.
#[derive(Clone)]
pub struct Scanner {
    ledger: Ledger,
    pub states: [Option<State>; 2],
    incoming: Vec<Incoming>,
}
impl Scanner {
    pub fn new(
        wallet: &ViewingAuthority,
        genesis: &Genesis,
        trusted_genesis: Digest,
    ) -> Result<Self> {
        if genesis.checkpoint() != trusted_genesis {
            return Err(Error::Recovery);
        }
        let mut states = [None, None];
        for (i, ct) in genesis.ciphertexts.iter().enumerate() {
            if let Ok(data) = decrypt(&wallet.viewing, ct) {
                let state = State::decode(&data)?;
                if state.commitment() != genesis.leaves[i] || !wallet.owns(&state) {
                    return Err(Error::Recovery);
                }
                states[i] = Some(state);
            }
        }
        Ok(Self {
            ledger: Ledger::from_genesis(genesis),
            states,
            incoming: Vec::new(),
        })
    }
    pub fn advance(
        &mut self,
        wallet: &ViewingAuthority,
        verifier: &Verifier,
        record: &Record,
    ) -> Result<()> {
        // Transactional cursor: failure leaves all wallet state unchanged.
        let mut next = self.clone();
        next.ledger.apply(verifier, record.transaction.clone())?;
        if next.ledger.head() != record.head || next.ledger.root() != record.root {
            return Err(Error::Recovery);
        }
        let t = &record.transaction;
        if let Ok(data) = decrypt(&wallet.viewing, &t.state_ciphertext) {
            let state = State::decode(&data)?;
            if state.commitment() != t.new || !wallet.owns(&state) {
                return Err(Error::Recovery);
            }
            next.states[t.slot as usize] = Some(state);
        } else if next.states[t.slot as usize].is_some() {
            // Do not silently keep a stale opening after a withheld/corrupt update.
            return Err(Error::Recovery);
        }
        if t.debit {
            if let Ok(data) = decrypt(&wallet.viewing, &t.credit_ciphertext) {
                let credit = Credit::decode(&data)?;
                if credit.commitment(hash(&[&t.credit_ciphertext])) != t.credit
                    || credit.owner != wallet.owner
                    || credit.view != wallet.view()
                {
                    return Err(Error::Recovery);
                }
                next.incoming.push(Incoming {
                    commitment: t.credit,
                    credit,
                    ciphertext: t.credit_ciphertext.clone(),
                });
            }
        } else {
            next.incoming.retain(|x| x.commitment != t.credit);
        }
        *self = next;
        Ok(())
    }
    pub fn require_head(&self, trusted_head: Digest) -> Result<()> {
        if self.ledger.head() == trusted_head {
            Ok(())
        } else {
            Err(Error::Recovery)
        }
    }
    pub fn incoming(&self) -> &[Incoming] {
        &self.incoming
    }
    pub fn cursor(&self) -> usize {
        self.ledger.records.len()
    }
}

#[cfg(test)]
mod tests;
