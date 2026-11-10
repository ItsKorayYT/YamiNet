//! Deterministic, non-financial chain foundation. No wallet or monetary state.
/// Inactive reader codec; never selected by production recovery or pruning.
#[cfg(feature = "inactive-checkpoint-v2")]
pub mod checkpoint_v2;
pub mod codec;
pub mod committee;
pub mod selection;
pub mod snapshot;
/// Isolated laboratory codec; never selected by consensus or automatic pruning.
#[cfg(feature = "experimental-snapshot-v2")]
pub mod snapshot_v2;
pub mod validators;
use codec::{Reader, Writer};
pub use committee::{Epoch, Member, ValidatorSet};
pub use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use im::OrdMap;
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeSet,
    sync::{Arc, OnceLock},
};
pub use yami_chain_naming::TestNames;
pub use yami_chain_privacy as privacy;

pub const CHAIN_ID: &str = "yamichain-local-0.2";
pub const PROTOCOL: u16 = 2;
pub const MAX_BLOCK_BYTES: usize = 65_536;
#[cfg(not(feature = "insecure-devnet-input"))]
pub const MAX_TX_BYTES: usize = 1024;
#[cfg(feature = "insecure-devnet-input")]
pub const MAX_TX_BYTES: usize = 32768;
pub const MAX_TXS: usize = 64;
pub const MAX_CERTIFICATE_BYTES: usize = 16_384;

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
    pub epochs: Vec<Epoch>,
    #[cfg(feature = "insecure-devnet-input")]
    pub live_committees: committee::LiveCommittees,
}
impl Genesis {
    pub fn local() -> Self {
        static LOCAL: std::sync::OnceLock<Genesis> = std::sync::OnceLock::new();
        LOCAL
            .get_or_init(|| Self::development_schedule(&[(1, 4)]).expect("valid local fixture"))
            .clone()
    }
    pub fn validate(&self) -> Result<()> {
        domain(&self.chain_id, self.protocol, self.features)?;
        committee::validate_epochs(&self.epochs)
    }
    pub fn canonical(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.raw(b"YC-GENESIS-2");
        // IDs are validated before a genesis can be used by State or a node.
        w.bytes(self.chain_id.as_bytes())
            .expect("genesis ID length");
        w.u16(self.protocol);
        w.u64(self.features);
        w.u32(self.epochs.len() as u32);
        for epoch in &self.epochs {
            w.u64(epoch.first_height);
            epoch.validators.put(&mut w);
        }
        #[cfg(feature = "insecure-devnet-input")]
        if let Some(scope) = self.live_committees.1 {
            w.raw(b"YC-LIVE-POS-1");
            w.raw(&scope);
        }
        w.0
    }
    pub fn id(&self) -> Digest {
        hash(b"yamichain-genesis-v2", &self.canonical())
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
    RegisterTestTld {
        label: String,
    },
    #[cfg(feature = "insecure-devnet-input")]
    PrivatePaymentCertificateV1 {
        certificate: Vec<u8>,
    },
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
            #[cfg(feature = "insecure-devnet-input")]
            Action::PrivatePaymentCertificateV1 { certificate } => {
                if certificate.len() > 30000 {
                    return Err(Error::Limit);
                }
                w.u8(240);
                w.u16(1);
                w.bytes(certificate)?;
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
            #[cfg(feature = "insecure-devnet-input")]
            240 => {
                if r.u16()? != 1 {
                    return Err(Error::Version);
                }
                Action::PrivatePaymentCertificateV1 {
                    certificate: r.bytes(30000)?.to_vec(),
                }
            }
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

#[derive(Clone, Debug)]
pub struct State {
    genesis: Arc<Genesis>,
    genesis_id: Digest,
    reserved_keys: Arc<BTreeSet<[u8; 32]>>,
    root_cache: OnceLock<Digest>,
    height: u64,
    tip: Digest,
    nonces: OrdMap<[u8; 32], u64>,
    names: TestNames,
    #[cfg(feature = "insecure-devnet-input")]
    native: Option<yamichain_devnet_cert::Ledger>,
}
impl PartialEq for State {
    fn eq(&self, other: &Self) -> bool {
        #[cfg(feature = "insecure-devnet-input")]
        if self.native.as_ref().map(|n| n.commitment())
            != other.native.as_ref().map(|n| n.commitment())
        {
            return false;
        }
        self.genesis == other.genesis
            && self.height == other.height
            && self.tip == other.tip
            && self.nonces == other.nonces
            && self.names == other.names
    }
}
impl Eq for State {}
impl State {
    #[allow(unused_mut)]
    pub fn new(mut genesis: Genesis) -> Result<Self> {
        #[cfg(feature = "insecure-devnet-input")]
        if let Some(path) = std::env::var_os("YAMI_NATIVE_POLICY") {
            if let Some(scope) = native_scope(&std::fs::read(path).map_err(|_| Error::Genesis)?)? {
                if genesis.live_committees.1.is_none() {
                    genesis.enable_live_committees(scope)?;
                }
                if genesis.live_committees.1 != Some(scope) {
                    return Err(Error::Genesis);
                }
            }
        }
        genesis.validate()?;
        let state = Self::from_validated_genesis(genesis);
        #[cfg(feature = "insecure-devnet-input")]
        let state = {
            let mut state = state;
            if let Some(path) = std::env::var_os("YAMI_NATIVE_POLICY") {
                let raw = std::fs::read(path).map_err(|_| Error::Genesis)?;
                let policy = serde_json_policy(&raw)?;
                state.native =
                    Some(yamichain_devnet_cert::Ledger::new(policy).map_err(|_| Error::Genesis)?);
            }
            state
        };
        Ok(state)
    }
    /// Separate, opt-in insecure network profile; absent from production builds.
    #[cfg(feature = "insecure-devnet-input")]
    pub fn insecure_devnet(mut genesis: Genesis, policy: &[u8]) -> Result<Self> {
        if let Some(scope) = serde_json_policy(policy)?.native_scope {
            genesis.enable_live_committees(scope)?;
        }
        genesis.validate()?;
        let mut state = Self::from_validated_genesis(genesis);
        state.native = Some(
            yamichain_devnet_cert::Ledger::new(serde_json_policy(policy)?)
                .map_err(|_| Error::Genesis)?,
        );
        Ok(state)
    }
    fn from_validated_genesis(genesis: Genesis) -> Self {
        let genesis_id = genesis.id();
        let reserved_keys = Arc::new(
            genesis
                .epochs
                .iter()
                .flat_map(|epoch| {
                    epoch
                        .validators
                        .members()
                        .iter()
                        .flat_map(|member| [member.consensus_key, member.transport_key])
                })
                .collect(),
        );
        Self {
            tip: genesis_id,
            genesis_id,
            reserved_keys,
            genesis: Arc::new(genesis),
            root_cache: OnceLock::new(),
            height: 0,
            nonces: OrdMap::new(),
            names: TestNames::default(),
            #[cfg(feature = "insecure-devnet-input")]
            native: None,
        }
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
    /// Derived only from immutable, validated application data; never persisted.
    pub fn root(&self) -> Digest {
        *self.root_cache.get_or_init(|| {
            let domain = b"yamichain-state-v1";
            let length = 32 + 8 + self.application_bytes();
            let mut hash = Sha256::new();
            hash.update((domain.len() as u64).to_be_bytes());
            hash.update(domain);
            hash.update((length as u64).to_be_bytes());
            hash.update(self.genesis_id.0);
            hash.update(self.height.to_be_bytes());
            self.visit_application_bytes(|bytes| hash.update(bytes));
            Digest(hash.finalize().into())
        })
    }
    pub fn nonce(&self, owner: &[u8; 32]) -> u64 {
        self.nonces.get(owner).copied().unwrap_or(0)
    }
    fn application_bytes(&self) -> usize {
        let size = 4 + self.nonces.len() * 40 + 4 + self.names.encoded_bytes();
        #[cfg(feature = "insecure-devnet-input")]
        let size = size + self.native.as_ref().map_or(0, |n| n.commitment().len());
        size
    }
    // Identical sorted canonical stream for hashing and checkpoint serialization.
    fn visit_application_bytes(&self, mut write: impl FnMut(&[u8])) {
        #[cfg(feature = "insecure-devnet-input")]
        if let Some(n) = &self.native {
            write(&n.commitment());
        }
        write(&(self.nonces.len() as u32).to_be_bytes());
        let mut nonce_bytes = [0; 40];
        for (key, nonce) in &self.nonces {
            nonce_bytes[..32].copy_from_slice(key);
            nonce_bytes[32..].copy_from_slice(&nonce.to_be_bytes());
            write(&nonce_bytes);
        }
        write(&(self.names.len() as u32).to_be_bytes());
        let mut name_bytes = [0; 99];
        for (label, owner) in self.names.entries() {
            name_bytes[..4].copy_from_slice(&(label.len() as u32).to_be_bytes());
            name_bytes[4..4 + label.len()].copy_from_slice(label.as_bytes());
            name_bytes[4 + label.len()..36 + label.len()].copy_from_slice(owner);
            write(&name_bytes[..36 + label.len()]);
        }
    }
    fn stage(&self, txs: &[Transaction]) -> Result<Self> {
        #[cfg(feature = "insecure-devnet-input")]
        if self.native.is_some()
            && (txs.len() != 1
                || !matches!(&txs[0].action, Action::PrivatePaymentCertificateV1 { .. }))
        {
            return Err(Error::Unsupported);
        }
        if txs.len() > MAX_TXS {
            return Err(Error::Limit);
        }
        let mut next = self.clone();
        next.root_cache = OnceLock::new();
        next.height = next.height.checked_add(1).ok_or(Error::Overflow)?;
        let mut seen = BTreeSet::new();
        for tx in txs {
            let bytes = tx.canonical()?;
            if !seen.insert(hash(b"yamichain-tx-id-v1", &bytes)) {
                return Err(Error::Duplicate);
            }
            let reserved = self.reserved_keys.contains(&tx.registry_owner);
            #[cfg(feature = "insecure-devnet-input")]
            let reserved = reserved || self.genesis.is_live_reserved_key(&tx.registry_owner);
            if reserved {
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
                #[cfg(feature = "insecure-devnet-input")]
                Action::PrivatePaymentCertificateV1 { certificate } => next
                    .native
                    .as_mut()
                    .ok_or(Error::Unsupported)?
                    .apply(certificate)
                    .map_err(|_| Error::Signature)?,
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
        self.proposed_from_stage(&next, transactions)
    }
    fn proposed_from_stage(&self, next: &Self, transactions: Vec<Transaction>) -> Result<Block> {
        let mut w = Writer::default();
        w.u32(transactions.len() as u32);
        for tx in &transactions {
            w.bytes(&tx.canonical()?)?;
        }
        let block = Block {
            header: Header {
                height: next.height,
                parent: self.tip,
                committee: self.genesis.committee_at(next.height)?.hash(),
                transactions: hash(b"yamichain-transactions-v1", &w.0),
                state: next.root(),
            },
            transactions,
        };
        block.canonical()?;
        Ok(block)
    }
    pub fn execute(&self, block: &Block) -> Result<Self> {
        let mut next = self.stage(&block.transactions)?;
        let expected = self.proposed_from_stage(&next, block.transactions.clone())?;
        if &expected != block {
            return Err(Error::Header);
        }
        next.tip = block.id()?;
        Ok(next)
    }
    /// Sustained, deterministic test-naming traffic, never financial value.
    /// Zero keeps the existing prototype workload exactly unchanged.
    pub fn development_workload_block(&self, count: usize) -> Result<Block> {
        if count == 0 {
            return self.development_block();
        }
        if count > MAX_TXS {
            return Err(Error::Limit);
        }
        let owner = development_key("registry", 0);
        let nonce = self
            .nonces
            .get(&owner.verifying_key().to_bytes())
            .copied()
            .unwrap_or(0);
        let height = self.height.checked_add(1).ok_or(Error::Overflow)?;
        let transactions = (0..count)
            .map(|i| {
                Transaction::sign(
                    &format!("test-load-{height}-{i}"),
                    nonce.checked_add(i as u64).ok_or(Error::Overflow)?,
                    height,
                    &owner,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        self.propose(transactions)
    }
    /// Distinct, deterministic laboratory accounts; no financial functionality.
    /// Existing workload/transaction semantics and all production limits are unchanged.
    pub fn development_distinct_workload_block(&self, count: usize) -> Result<Block> {
        if count == 0 {
            return self.development_block();
        }
        if count > MAX_TXS {
            return Err(Error::Limit);
        }
        let height = self.height.checked_add(1).ok_or(Error::Overflow)?;
        let transactions = (0..count)
            .map(|i| {
                let mut seed = Writer::default();
                seed.raw(&self.genesis_id.0);
                seed.u64(height);
                seed.u32(i as u32);
                let owner = SigningKey::from_bytes(&hash(b"YC-DISTINCT-LAB-ACCOUNT-1", &seed.0).0);
                Transaction::sign(&format!("test-distinct-{height}-{i}"), 0, height, &owner)
            })
            .collect::<Result<Vec<_>>>()?;
        self.propose(transactions)
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

#[cfg(test)]
mod scalability_tests;

#[cfg(feature = "insecure-devnet-input")]
fn serde_json_policy(raw: &[u8]) -> Result<yamichain_devnet_cert::WitnessPolicy> {
    yamichain_devnet_cert::parse_policy(raw).map_err(|_| Error::Genesis)
}
#[cfg(feature = "insecure-devnet-input")]
impl Transaction {
    pub fn devnet_certificate(
        certificate: Vec<u8>,
        nonce: u64,
        expiry: u64,
        owner: &SigningKey,
    ) -> Result<Self> {
        let mut tx = Self::sign("test-unused", nonce, expiry, owner)?;
        tx.action = Action::PrivatePaymentCertificateV1 { certificate };
        tx.signature = owner.sign(&tx.signing_bytes()?).to_bytes();
        Ok(tx)
    }
}

#[cfg(feature = "insecure-devnet-input")]
impl State {
    /// Call only after authenticating and durably appending the containing consensus record.
    pub fn install_finalized_committee(&self) -> Result<()> {
        if let Some(set) = self
            .native
            .as_ref()
            .and_then(|n| n.next_validators.as_ref())
        {
            let members = set
                .iter()
                .map(|(c, t)| Member {
                    consensus_key: *c,
                    transport_key: *t,
                    power: 1,
                })
                .collect();
            self.genesis.install_finalized_committee(
                self.height.checked_add(1).ok_or(Error::Overflow)?,
                ValidatorSet::new(members)?,
            )?;
        }
        Ok(())
    }
}

#[cfg(feature = "insecure-devnet-input")]
pub fn native_scope(raw: &[u8]) -> Result<Option<[u8; 32]>> {
    Ok(serde_json_policy(raw)?.native_scope)
}
