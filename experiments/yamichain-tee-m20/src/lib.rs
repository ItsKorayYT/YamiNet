//! Isolated SOFTWARE prototype. No production integration or genuine TDX verifier.
use chacha20poly1305::{
    aead::{Aead, Payload},
    ChaCha20Poly1305, KeyInit, Nonce,
};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use x25519_dalek::{PublicKey, StaticSecret};
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub type Id = [u8; 32];
pub fn bytes<T: Serialize>(v: &T) -> Vec<u8> {
    serde_json::to_vec(v).expect("serializable protocol type")
}
pub fn hash(v: &[u8]) -> Id {
    Sha256::digest(v).into()
}
fn check(ok: bool) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err("verification rejected".into())
    }
}
fn verify(key: Id, msg: &[u8], sig: &[u8]) -> Result<()> {
    VerifyingKey::from_bytes(&key)?.verify_strict(msg, &Signature::from_slice(sig)?)?;
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Transfer {
    pub sender: Id,
    pub recipient: Id,
    pub amount: u64,
    pub nonce: u64,
    pub chain: u64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Signed {
    pub transfer: Transfer,
    pub signature: Vec<u8>,
}
impl Signed {
    pub fn sign(t: Transfer, wallet: &SigningKey) -> Self {
        let signature = wallet
            .sign(&bytes(&("YAMI-M20-transfer-v1", &t)))
            .to_bytes()
            .to_vec();
        Self {
            transfer: t,
            signature,
        }
    }
    pub fn verify(&self) -> Result<()> {
        verify(
            self.transfer.sender,
            &bytes(&("YAMI-M20-transfer-v1", &self.transfer)),
            &self.signature,
        )
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub ephemeral: Id,
    pub nonce: [u8; 12],
    pub ciphertext: Vec<u8>,
}
fn shared(secret: &StaticSecret, public: Id, context: &[u8]) -> Result<Id> {
    let s = secret.diffie_hellman(&PublicKey::from(public));
    check(s.was_contributory())?;
    let mut k = [0; 32];
    Hkdf::<Sha256>::new(Some(b"YAMI-M20-X25519-v1"), s.as_bytes())
        .expand(context, &mut k)
        .map_err(|_| "kdf")?;
    Ok(k)
}
fn seal(key: Id, plain: &[u8], context: &[u8]) -> Result<Envelope> {
    let mut nonce = [0; 12];
    OsRng.fill_bytes(&mut nonce);
    let ciphertext = ChaCha20Poly1305::new((&key).into())
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plain,
                aad: context,
            },
        )
        .map_err(|_| "encrypt")?;
    Ok(Envelope {
        ephemeral: [0; 32],
        nonce,
        ciphertext,
    })
}
fn open(key: Id, e: &Envelope, context: &[u8]) -> Result<Vec<u8>> {
    check(e.ciphertext.len() <= 16 * 1024 * 1024)?;
    ChaCha20Poly1305::new((&key).into())
        .decrypt(
            Nonce::from_slice(&e.nonce),
            Payload {
                msg: &e.ciphertext,
                aad: context,
            },
        )
        .map_err(|_| "authentication failed".into())
}
pub fn encrypt(public: Id, plain: &[u8], context: &[u8]) -> Result<Envelope> {
    let sk = StaticSecret::random_from_rng(OsRng);
    let mut e = seal(shared(&sk, public, context)?, plain, context)?;
    e.ephemeral = PublicKey::from(&sk).to_bytes();
    Ok(e)
}
pub fn decrypt(secret: &StaticSecret, e: &Envelope, context: &[u8]) -> Result<Vec<u8>> {
    open(shared(secret, e.ephemeral, context)?, e, context)
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Account {
    pub balance: u64,
    pub nonce: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct State {
    pub chain: u64,
    pub epoch: u64,
    pub accounts: BTreeMap<Id, Account>,
}
// JSON map keys cannot represent byte arrays: canonical ordered sequence wire format.
#[derive(Serialize, Deserialize)]
struct StateWire(u64, u64, Vec<(Id, Account)>);
fn encode(s: &State) -> Vec<u8> {
    bytes(&StateWire(
        s.chain,
        s.epoch,
        s.accounts.iter().map(|(k, v)| (*k, v.clone())).collect(),
    ))
}
fn decode(b: &[u8]) -> Result<State> {
    let StateWire(chain, epoch, a) = serde_json::from_slice(b)?;
    let len = a.len();
    let accounts: BTreeMap<_, _> = a.into_iter().collect();
    check(accounts.len() == len)?;
    Ok(State {
        chain,
        epoch,
        accounts,
    })
}
pub fn transition(old: &State, tx: &Signed) -> Result<State> {
    tx.verify()?;
    let t = &tx.transfer;
    check(t.chain == old.chain && t.amount > 0 && t.sender != t.recipient)?;
    let mut s = old.clone();
    let a = s.accounts.get_mut(&t.sender).ok_or("unknown sender")?;
    check(a.nonce == t.nonce)?;
    a.balance = a
        .balance
        .checked_sub(t.amount)
        .ok_or("insufficient funds")?;
    a.nonce = a.nonce.checked_add(1).ok_or("nonce overflow")?;
    let b = s.accounts.entry(t.recipient).or_default();
    b.balance = b.balance.checked_add(t.amount).ok_or("balance overflow")?;
    s.epoch = s.epoch.checked_add(1).ok_or("epoch overflow")?;
    Ok(s)
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Head {
    pub chain: u64,
    pub epoch: u64,
    pub commitment: Id,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub previous: Head,
    pub next: Head,
    pub request: Id,
    pub signature: Vec<u8>,
}
impl Receipt {
    fn message(&self) -> Vec<u8> {
        bytes(&(
            "YAMI-M20-SOFTWARE-receipt-v1",
            &self.previous,
            &self.next,
            self.request,
        ))
    }
    pub fn verify(&self, pinned: Id, expected: &Head) -> Result<()> {
        check(
            &self.previous == expected
                && self.next.chain == expected.chain
                && expected.epoch.checked_add(1) == Some(self.next.epoch),
        )?;
        verify(pinned, &self.message(), &self.signature)
    }
}
#[derive(Serialize, Deserialize)]
struct Snapshot {
    encrypted: Envelope,
    receipt: Option<Receipt>,
}
pub struct Engine {
    state: State,
    head: Head,
    path: PathBuf,
    storage: Id,
    signer: SigningKey,
    transport: StaticSecret,
    poisoned: bool,
    _lock: File,
}
impl Engine {
    pub fn public(&self) -> Id {
        PublicKey::from(&self.transport).to_bytes()
    }
    pub fn head(&self) -> Head {
        self.head.clone()
    }
    pub fn receipt_key(&self) -> Id {
        self.signer.verifying_key().to_bytes()
    }
    /// Secrets supplied by caller; never persist wallet keys. External head must be independently durable.
    pub fn load(
        path: &Path,
        storage: Id,
        signer: SigningKey,
        transport: StaticSecret,
        expected: &Head,
    ) -> Result<Self> {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock)?;
        let raw = fs::read(path)?;
        let snap: Snapshot = serde_json::from_slice(&raw)?;
        check(hash(&bytes(&snap.encrypted)) == expected.commitment)?;
        let state = decode(&open(storage, &snap.encrypted, b"YAMI-M20-state-v1")?)?;
        check(state.chain == expected.chain && state.epoch == expected.epoch)?;
        Ok(Self {
            state,
            head: expected.clone(),
            path: path.into(),
            storage,
            signer,
            transport,
            poisoned: false,
            _lock: lock,
        })
    }
    pub fn create(
        path: &Path,
        state: State,
        storage: Id,
        signer: SigningKey,
        transport: StaticSecret,
    ) -> Result<Self> {
        let encrypted = seal(storage, &encode(&state), b"YAMI-M20-state-v1")?;
        let head = Head {
            chain: state.chain,
            epoch: state.epoch,
            commitment: hash(&bytes(&encrypted)),
        };
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(&bytes(&Snapshot {
            encrypted,
            receipt: None,
        }))?;
        file.sync_all()?;
        File::open(path.parent().ok_or("parent")?)?.sync_all()?;
        Self::load(path, storage, signer, transport, &head)
    }
    pub fn execute(&mut self, encrypted: &Envelope) -> Result<Receipt> {
        self.execute_with_fault(encrypted, |_| Ok(()))
    }
    /// Harness hook for deterministic interruption tests. Errors poison this instance.
    pub fn execute_with_fault(
        &mut self,
        encrypted: &Envelope,
        mut hook: impl FnMut(&str) -> Result<()>,
    ) -> Result<Receipt> {
        check(!self.poisoned)?;
        let tx: Signed = serde_json::from_slice(&decrypt(
            &self.transport,
            encrypted,
            b"YAMI-M20-payment-v1",
        )?)?;
        let state = transition(&self.state, &tx)?;
        let encrypted_state = seal(self.storage, &encode(&state), b"YAMI-M20-state-v1")?;
        let mut r = Receipt {
            previous: self.head.clone(),
            next: Head {
                chain: state.chain,
                epoch: state.epoch,
                commitment: hash(&bytes(&encrypted_state)),
            },
            request: hash(&bytes(encrypted)),
            signature: vec![],
        };
        r.signature = self.signer.sign(&r.message()).to_bytes().to_vec();
        self.poisoned = true;
        let temp = self.path.with_extension("pending");
        let mut f = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temp)?;
        f.write_all(&bytes(&Snapshot {
            encrypted: encrypted_state,
            receipt: Some(r.clone()),
        }))?;
        f.sync_all()?;
        hook("synced_pending")?;
        fs::rename(temp, &self.path)?;
        hook("renamed")?;
        File::open(self.path.parent().ok_or("parent")?)?.sync_all()?;
        hook("synced_directory")?;
        self.state = state;
        self.head = r.next.clone();
        self.poisoned = false;
        Ok(r)
    }
    pub fn query(&self, q: &Query) -> Result<Envelope> {
        check(q.head == self.head)?;
        verify(q.account, &q.message(), &q.signature)?;
        let mut response = QueryResponse {
            account: self
                .state
                .accounts
                .get(&q.account)
                .cloned()
                .unwrap_or_default(),
            query_hash: hash(&q.message()),
            signature: vec![],
        };
        response.signature = self.signer.sign(&response.message()).to_bytes().to_vec();
        encrypt(q.reply_key, &bytes(&response), &q.message())
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Query {
    pub account: Id,
    pub reply_key: Id,
    pub head: Head,
    pub challenge: Id,
    pub signature: Vec<u8>,
}
impl Query {
    pub fn message(&self) -> Vec<u8> {
        bytes(&(
            "YAMI-M20-query-v1",
            self.account,
            self.reply_key,
            &self.head,
            self.challenge,
        ))
    }
    pub fn sign(&mut self, w: &SigningKey) {
        self.signature = w.sign(&self.message()).to_bytes().to_vec();
    }
}
/// Independently checks authorization, conservation, ranges and unchanged accounts,
/// but requires plaintext disclosure: research oracle, NOT a private public proof.
pub fn audit_disclosed(old: &State, tx: &Signed, proposed: &State) -> Result<()> {
    check(transition(old, tx)? == *proposed)
}
#[derive(Clone, Serialize, Deserialize)]
pub struct SoftwareEvidence {
    pub measurement: Id,
    pub identity: Id,
    pub key: Id,
    pub challenge: Id,
    pub issued: u64,
    pub expires: u64,
}
pub fn verify_software(
    e: &SoftwareEvidence,
    approved: Id,
    identity: Id,
    key: Id,
    challenge: Id,
    now: u64,
) -> Result<()> {
    check(
        e.measurement == approved
            && e.identity == identity
            && e.key == key
            && e.challenge == challenge
            && e.issued <= now
            && now <= e.expires
            && e.expires.checked_sub(e.issued).is_some_and(|d| d <= 60),
    )
}
/// Fail closed: synthetic evidence is never accepted as Intel TDX evidence.
pub fn verify_tdx_quote(_quote: &[u8]) -> Result<()> {
    Err("TDX DCAP verification unavailable; SOFTWARE ONLY; production gate closed".into())
}

#[derive(Clone, Serialize, Deserialize)]
pub struct QueryResponse {
    pub account: Account,
    pub query_hash: Id,
    pub signature: Vec<u8>,
}
impl QueryResponse {
    fn message(&self) -> Vec<u8> {
        bytes(&("YAMI-M20-query-response-v1", &self.account, self.query_hash))
    }
    pub fn verify(&self, pinned: Id, q: &Query) -> Result<()> {
        check(self.query_hash == hash(&q.message()))?;
        verify(pinned, &self.message(), &self.signature)
    }
}

/// M21 software-only 3-of-4 extension.
pub mod quorum;

/// Private process harness for reproducible software-only quorum tests.
pub mod software_harness;

/// M22 wallet-local Intel QVL verification and attestation-gated delivery.
pub mod attestation;
