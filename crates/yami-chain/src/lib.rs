//! Deterministic, non-financial chain foundation. No wallet or monetary state.
pub mod codec;
use codec::{Reader, Writer};
pub use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
pub use yami_chain_naming::TestNames;
pub use yami_chain_privacy as privacy;

pub const CHAIN_ID: &str = "yamichain-local-0.1";
pub const PROTOCOL: u16 = 1;
pub const MAX_BLOCK_BYTES: usize = 65_536;
pub const MAX_TX_BYTES: usize = 1024;
pub const MAX_TXS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Encoding,
    Limit,
    Chain,
    Version,
    Overflow,
    Signature,
    Nonce,
    Expired,
    Duplicate,
    Naming,
    Header,
    Genesis,
    Unsupported,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "chain validation: {self:?}")
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Digest(pub [u8; 32]);
impl std::fmt::Display for Digest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}
pub fn hash(domain: &[u8], bytes: &[u8]) -> Digest {
    let mut h = Sha256::new();
    h.update((domain.len() as u64).to_be_bytes());
    h.update(domain);
    h.update((bytes.len() as u64).to_be_bytes());
    h.update(bytes);
    Digest(h.finalize().into())
}

/// Arithmetic utility only: not spendable value or a public transfer type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Amount(pub u128);
impl Amount {
    pub fn checked_add(self, other: Self) -> Result<Self> {
        self.0.checked_add(other.0).map(Self).ok_or(Error::Overflow)
    }
    pub fn checked_sub(self, other: Self) -> Result<Self> {
        self.0.checked_sub(other.0).map(Self).ok_or(Error::Overflow)
    }
}

/// Public development fixture, NOT secure keys. Role labels prevent incidental reuse.
pub fn development_key(role: &str, index: u8) -> SigningKey {
    let mut material = role.as_bytes().to_vec();
    material.push(index);
    SigningKey::from_bytes(&hash(b"yamichain-public-development-key-v1", &material).0)
}
pub fn verify(key: &[u8; 32], bytes: &[u8], signature: &[u8; 64]) -> Result<()> {
    VerifyingKey::from_bytes(key)
        .map_err(|_| Error::Signature)?
        .verify_strict(bytes, &ed25519_dalek::Signature::from_bytes(signature))
        .map_err(|_| Error::Signature)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Genesis {
    pub chain_id: String,
    pub protocol: u16,
    pub features: u64,
    pub validators: [[u8; 32]; 4],
    pub transport_keys: [[u8; 32]; 4],
}
impl Genesis {
    pub fn local() -> Self {
        static LOCAL: std::sync::OnceLock<Genesis> = std::sync::OnceLock::new();
        LOCAL
            .get_or_init(|| Self {
                chain_id: CHAIN_ID.into(),
                protocol: PROTOCOL,
                features: 0,
                validators: std::array::from_fn(|i| {
                    development_key("consensus", i as u8)
                        .verifying_key()
                        .to_bytes()
                }),
                transport_keys: std::array::from_fn(|i| {
                    development_key("transport", i as u8)
                        .verifying_key()
                        .to_bytes()
                }),
            })
            .clone()
    }
    pub fn validate(&self) -> Result<()> {
        domain(&self.chain_id, self.protocol, self.features)?;
        let keys: BTreeSet<_> = self
            .validators
            .iter()
            .chain(self.transport_keys.iter())
            .collect();
        if keys.len() != 8 {
            return Err(Error::Genesis);
        }
        for key in keys {
            VerifyingKey::from_bytes(key).map_err(|_| Error::Genesis)?;
        }
        // Only the exact public four-validator local fixture is authorized in 0.1.
        if self != &Self::local() {
            return Err(Error::Genesis);
        }
        Ok(())
    }
    pub fn canonical(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.bytes(self.chain_id.as_bytes()).expect("bounded chain ID");
        w.u16(self.protocol);
        w.u64(self.features);
        for key in self.validators.iter().chain(self.transport_keys.iter()) {
            w.raw(key);
        }
        w.0
    }
    pub fn id(&self) -> Digest {
        hash(b"yamichain-genesis-v1", &self.canonical())
    }
    pub fn committee_hash(&self) -> Digest {
        let mut keys = self.validators;
        keys.sort();
        hash(b"yamichain-fixed-committee-v1", &keys.concat())
    }
}
fn domain(chain: &str, protocol: u16, features: u64) -> Result<()> {
    if chain != CHAIN_ID {
        return Err(Error::Chain);
    }
    if protocol != PROTOCOL || features != 0 {
        return Err(Error::Version);
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    RegisterTestTld { label: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transaction {
    pub chain_id: String,
    pub protocol: u16,
    pub features: u64,
    pub expiry: u64,
    pub nonce: u64,
    pub registry_owner: [u8; 32],
    pub action: Action,
    pub signature: [u8; 64],
}
impl Transaction {
    pub fn signing_bytes(&self) -> Result<Vec<u8>> {
        domain(&self.chain_id, self.protocol, self.features)?;
        let mut w = Writer::default();
        w.raw(b"YC-TX-1");
        w.bytes(self.chain_id.as_bytes())?;
        w.u16(self.protocol);
        w.u64(self.features);
        w.u64(self.expiry);
        w.u64(self.nonce);
        w.raw(&self.registry_owner);
        match &self.action {
            Action::RegisterTestTld { label } => {
                if label.len() > 63 {
                    return Err(Error::Limit);
                }
                w.u8(1);
                w.bytes(label.as_bytes())?;
            }
        }
        Ok(w.0)
    }
    pub fn sign(label: &str, nonce: u64, expiry: u64, owner: &SigningKey) -> Result<Self> {
        let mut tx = Self {
            chain_id: CHAIN_ID.into(),
            protocol: PROTOCOL,
            features: 0,
            expiry,
            nonce,
            registry_owner: owner.verifying_key().to_bytes(),
            action: Action::RegisterTestTld {
                label: label.into(),
            },
            signature: [0; 64],
        };
        tx.signature = owner.sign(&tx.signing_bytes()?).to_bytes();
        Ok(tx)
    }
    pub fn canonical(&self) -> Result<Vec<u8>> {
        let mut bytes = self.signing_bytes()?;
        bytes.extend(self.signature);
        if bytes.len() > MAX_TX_BYTES {
            return Err(Error::Limit);
        }
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes, MAX_TX_BYTES)?;
        if r.take(7)? != b"YC-TX-1" {
            return Err(Error::Encoding);
        }
        let chain_id = std::str::from_utf8(r.bytes(32)?)
            .map_err(|_| Error::Encoding)?
            .into();
        let protocol = r.u16()?;
        let features = r.u64()?;
        let expiry = r.u64()?;
        let nonce = r.u64()?;
        let registry_owner = r.array()?;
        let action = match r.u8()? {
            1 => Action::RegisterTestTld {
                label: std::str::from_utf8(r.bytes(63)?)
                    .map_err(|_| Error::Encoding)?
                    .into(),
            },
            _ => return Err(Error::Unsupported),
        };
        let signature = r.array()?;
        r.end()?;
        let tx = Self {
            chain_id,
            protocol,
            features,
            expiry,
            nonce,
            registry_owner,
            action,
            signature,
        };
        if tx.canonical()? != bytes {
            return Err(Error::Encoding);
        }
        Ok(tx)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub height: u64,
    pub parent: Digest,
    pub committee: Digest,
    pub transactions: Digest,
    pub state: Digest,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub header: Header,
    pub transactions: Vec<Transaction>,
}
impl Block {
    pub fn canonical(&self) -> Result<Vec<u8>> {
        if self.transactions.len() > MAX_TXS {
            return Err(Error::Limit);
        }
        let mut w = Writer::default();
        w.raw(b"YC-BLOCK-1");
        w.bytes(CHAIN_ID.as_bytes())?;
        w.u16(PROTOCOL);
        w.u64(0);
        w.u64(self.header.height);
        for d in [
            self.header.parent,
            self.header.committee,
            self.header.transactions,
            self.header.state,
        ] {
            w.raw(&d.0);
        }
        w.u32(self.transactions.len() as u32);
        for tx in &self.transactions {
            w.bytes(&tx.canonical()?)?;
        }
        if w.0.len() > MAX_BLOCK_BYTES {
            return Err(Error::Limit);
        }
        Ok(w.0)
    }
    pub fn id(&self) -> Result<Digest> {
        Ok(hash(b"yamichain-block-v1", &self.canonical()?))
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes, MAX_BLOCK_BYTES)?;
        if r.take(10)? != b"YC-BLOCK-1" {
            return Err(Error::Encoding);
        }
        let chain = std::str::from_utf8(r.bytes(32)?).map_err(|_| Error::Encoding)?;
        domain(chain, r.u16()?, r.u64()?)?;
        let header = Header {
            height: r.u64()?,
            parent: Digest(r.array()?),
            committee: Digest(r.array()?),
            transactions: Digest(r.array()?),
            state: Digest(r.array()?),
        };
        let count = r.u32()? as usize;
        if count > MAX_TXS {
            return Err(Error::Limit);
        }
        let mut transactions = Vec::with_capacity(count);
        for _ in 0..count {
            transactions.push(Transaction::decode(r.bytes(MAX_TX_BYTES)?)?);
        }
        r.end()?;
        Ok(Self {
            header,
            transactions,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    genesis: Genesis,
    height: u64,
    tip: Digest,
    nonces: BTreeMap<[u8; 32], u64>,
    names: TestNames,
}
impl State {
    pub fn new(genesis: Genesis) -> Result<Self> {
        genesis.validate()?;
        Ok(Self {
            tip: genesis.id(),
            genesis,
            height: 0,
            nonces: BTreeMap::new(),
            names: TestNames::default(),
        })
    }
    pub fn height(&self) -> u64 {
        self.height
    }
    pub fn tip(&self) -> Digest {
        self.tip
    }
    pub fn genesis(&self) -> &Genesis {
        &self.genesis
    }
    pub fn names(&self) -> &TestNames {
        &self.names
    }
    pub fn root(&self) -> Digest {
        let mut w = Writer::default();
        w.raw(&self.genesis.id().0);
        w.u64(self.height);
        w.u32(self.nonces.len() as u32);
        for (key, nonce) in &self.nonces {
            w.raw(key);
            w.u64(*nonce);
        }
        let entries: Vec<_> = self.names.entries().collect();
        w.u32(entries.len() as u32);
        for (label, owner) in entries {
            w.bytes(label.as_bytes()).expect("bounded label");
            w.raw(owner);
        }
        hash(b"yamichain-state-v1", &w.0)
    }
    fn stage(&self, txs: &[Transaction]) -> Result<Self> {
        if txs.len() > MAX_TXS {
            return Err(Error::Limit);
        }
        let mut next = self.clone();
        next.height = next.height.checked_add(1).ok_or(Error::Overflow)?;
        let mut seen = BTreeSet::new();
        for tx in txs {
            let bytes = tx.canonical()?;
            if !seen.insert(hash(b"yamichain-tx-id-v1", &bytes)) {
                return Err(Error::Duplicate);
            }
            if self.genesis.validators.contains(&tx.registry_owner)
                || self.genesis.transport_keys.contains(&tx.registry_owner)
            {
                return Err(Error::Signature);
            }
            verify(&tx.registry_owner, &tx.signing_bytes()?, &tx.signature)?;
            if tx.expiry < next.height {
                return Err(Error::Expired);
            }
            let nonce = next.nonces.get(&tx.registry_owner).copied().unwrap_or(0);
            if tx.nonce != nonce {
                return Err(Error::Nonce);
            }
            // Version 1 module dispatch: only the non-financial test naming module.
            match &tx.action {
                Action::RegisterTestTld { label } => next
                    .names
                    .register(label, tx.registry_owner)
                    .map_err(|_| Error::Naming)?,
            }
            next.nonces.insert(
                tx.registry_owner,
                nonce.checked_add(1).ok_or(Error::Overflow)?,
            );
        }
        Ok(next)
    }
    pub fn propose(&self, transactions: Vec<Transaction>) -> Result<Block> {
        let next = self.stage(&transactions)?;
        let mut w = Writer::default();
        w.u32(transactions.len() as u32);
        for tx in &transactions {
            w.bytes(&tx.canonical()?)?;
        }
        let block = Block {
            header: Header {
                height: next.height,
                parent: self.tip,
                committee: self.genesis.committee_hash(),
                transactions: hash(b"yamichain-transactions-v1", &w.0),
                state: next.root(),
            },
            transactions,
        };
        block.canonical()?;
        Ok(block)
    }
    pub fn execute(&self, block: &Block) -> Result<Self> {
        let expected = self.propose(block.transactions.clone())?;
        if &expected != block {
            return Err(Error::Header);
        }
        let mut next = self.stage(&block.transactions)?;
        next.tip = block.id()?;
        Ok(next)
    }
    /// Repeatable local non-financial workload, identical for every proposer/round.
    pub fn development_block(&self) -> Result<Block> {
        let txs = if self.height == 0 {
            vec![Transaction::sign(
                "test-local",
                0,
                1,
                &development_key("registry", 0),
            )?]
        } else {
            vec![]
        };
        self.propose(txs)
    }
}
