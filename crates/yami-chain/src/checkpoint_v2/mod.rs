//! Inactive, separately versioned checkpoint codec. No production recovery/pruning.
//! Forked from the lab codec to freeze its bytes and preserve strict separation.
//! Only the complete canonical root authenticates peer transport contents.
use crate::*;
use std::{
    collections::BTreeMap,
    io::{self, Read},
};
pub mod store;
pub const MAX_ENTRIES: u32 = 500_000;
pub const MAX_CHUNK_BYTES: usize = 256 * 1024;
pub const MAX_CHUNK_RECORDS: u32 = 4096;
pub const MAX_CHUNKS: usize = 512;
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
pub const MAX_PAYLOAD_BYTES: u64 = 80 * 1024 * 1024;
const DOMAIN: &[u8] = b"YC-INACTIVE-CHECKPOINT2-1";
fn invalid(message: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateAnchor {
    pub genesis: Digest,
    pub height: u64,
    pub tip: Digest,
    pub root: Digest,
}
impl StateAnchor {
    pub fn canonical(self) -> Vec<u8> {
        let mut w = Writer::default();
        w.raw(b"YC-INACTIVE-CHECKPOINT2-STATE-1");
        w.raw(&self.genesis.0);
        w.u64(self.height);
        w.raw(&self.tip.0);
        w.raw(&self.root.0);
        w.0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub state: StateAnchor,
    pub manifest: Digest,
}
impl Anchor {
    pub fn canonical(self) -> Vec<u8> {
        let mut w = Writer::default();
        w.raw(b"YC-INACTIVE-CHECKPOINT2-ANCHOR-1");
        w.raw(&self.state.genesis.0);
        w.u64(self.state.height);
        w.raw(&self.state.tip.0);
        w.raw(&self.state.root.0);
        w.raw(&self.manifest.0);
        w.0
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Chunk {
    kind: u8,
    first: u32,
    records: u32,
    bytes: u32,
    digest: Digest,
}
impl Chunk {
    fn header(&self, index: u32) -> Vec<u8> {
        let mut w = Writer::default();
        w.u32(index);
        w.u8(self.kind);
        w.u32(self.first);
        w.u32(self.records);
        w.u32(self.bytes);
        w.0
    }
    fn digest(&self, index: u32, payload: &[u8]) -> Digest {
        let mut bytes = self.header(index);
        bytes.extend_from_slice(payload);
        hash(b"YC-INACTIVE-CHECKPOINT2-CHUNK-1", &bytes)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    state: StateAnchor,
    accounts: u32,
    names: u32,
    total: u64,
    chunks: Vec<Chunk>,
}
impl Manifest {
    pub fn anchor(&self) -> Anchor {
        Anchor {
            state: self.state,
            manifest: hash(b"YC-INACTIVE-CHECKPOINT2-MANIFEST-1", &self.canonical()),
        }
    }
    pub fn chunks(&self) -> usize {
        self.chunks.len()
    }
    pub fn payload_bytes(&self) -> u64 {
        self.total
    }
    pub fn accounts(&self) -> u32 {
        self.accounts
    }
    pub fn names(&self) -> u32 {
        self.names
    }
    pub fn canonical(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.raw(DOMAIN);
        w.raw(&self.state.genesis.0);
        w.u64(self.state.height);
        w.raw(&self.state.tip.0);
        w.raw(&self.state.root.0);
        w.u32(self.accounts);
        w.u32(self.names);
        w.u64(self.total);
        w.u32(self.chunks.len() as u32);
        for c in &self.chunks {
            w.u8(c.kind);
            w.u32(c.first);
            w.u32(c.records);
            w.u32(c.bytes);
            w.raw(&c.digest.0);
        }
        w.0
    }
    /// Untrusted framing only. The state anchor must come from independently
    /// verified chain context. Success authenticates no chunk or application state.
    pub fn decode_transport(bytes: &[u8], state: StateAnchor) -> io::Result<Self> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(invalid("transport manifest cap before hashing"));
        }
        Self::decode(
            bytes,
            Anchor {
                state,
                manifest: hash(b"YC-INACTIVE-CHECKPOINT2-MANIFEST-1", bytes),
            },
        )
    }
    pub fn decode(bytes: &[u8], expected: Anchor) -> io::Result<Self> {
        if bytes.len() > MAX_MANIFEST_BYTES
            || hash(b"YC-INACTIVE-CHECKPOINT2-MANIFEST-1", bytes) != expected.manifest
        {
            return Err(invalid("manifest cap/transport commitment"));
        }
        let mut r = Reader::new(bytes, MAX_MANIFEST_BYTES).map_err(invalid)?;
        if r.take(DOMAIN.len()).map_err(invalid)? != DOMAIN {
            return Err(invalid("inactive manifest domain"));
        }
        let state = StateAnchor {
            genesis: Digest(r.array().map_err(invalid)?),
            height: r.u64().map_err(invalid)?,
            tip: Digest(r.array().map_err(invalid)?),
            root: Digest(r.array().map_err(invalid)?),
        };
        if state != expected.state || state.height == 0 || state.height > 1_000_000 {
            return Err(invalid("manifest independent state anchor"));
        }
        let accounts = r.u32().map_err(invalid)?;
        let names = r.u32().map_err(invalid)?;
        let total = r.u64().map_err(invalid)?;
        let n = r.u32().map_err(invalid)? as usize;
        if accounts > MAX_ENTRIES
            || names > MAX_ENTRIES
            || accounts > names
            || total > MAX_PAYLOAD_BYTES
            || n > MAX_CHUNKS
        {
            return Err(invalid("inactive allocation/work bounds"));
        }
        let mut chunks = Vec::with_capacity(n);
        let mut counters = [0u32; 2];
        let mut sum = 0u64;
        let mut last = 0;
        for _ in 0..n {
            let c = Chunk {
                kind: r.u8().map_err(invalid)?,
                first: r.u32().map_err(invalid)?,
                records: r.u32().map_err(invalid)?,
                bytes: r.u32().map_err(invalid)?,
                digest: Digest(r.array().map_err(invalid)?),
            };
            if c.kind > 1
                || c.kind < last
                || c.records == 0
                || c.records > MAX_CHUNK_RECORDS
                || c.bytes as usize > MAX_CHUNK_BYTES
                || c.first != counters[c.kind as usize]
            {
                return Err(invalid("chunk order/count/length"));
            }
            let (min, max) = if c.kind == 0 { (40, 40) } else { (42, 99) };
            if c.bytes < c.records * min || c.bytes > c.records * max {
                return Err(invalid("chunk impossible record lengths"));
            }
            counters[c.kind as usize] = c
                .first
                .checked_add(c.records)
                .ok_or_else(|| invalid("chunk count overflow"))?;
            sum = sum
                .checked_add(u64::from(c.bytes))
                .ok_or_else(|| invalid("payload overflow"))?;
            if sum > MAX_PAYLOAD_BYTES {
                return Err(invalid("payload budget"));
            }
            last = c.kind;
            chunks.push(c);
        }
        r.end().map_err(invalid)?;
        if counters != [accounts, names] || sum != total {
            return Err(invalid("manifest completeness"));
        }
        let manifest = Self {
            state,
            accounts,
            names,
            total,
            chunks,
        };
        if manifest.canonical() != bytes {
            return Err(invalid("manifest noncanonical"));
        }
        Ok(manifest)
    }
    pub fn read_chunk<R: Read>(&self, index: usize, mut input: R) -> io::Result<Vec<u8>> {
        let c = self
            .chunks
            .get(index)
            .ok_or_else(|| invalid("chunk index"))?;
        let mut bytes = vec![0; c.bytes as usize];
        input.read_exact(&mut bytes)?;
        let mut extra = [0];
        if input.read(&mut extra)? != 0 {
            return Err(invalid("chunk extra bytes"));
        }
        if c.digest(index as u32, &bytes) != c.digest {
            return Err(invalid("chunk transport integrity"));
        }
        Ok(bytes)
    }
}
/// Opaque inactive result. No Deref, into-State, production checkpoint or pruner
/// capability is exposed. v1 serialization remains subject to all v1 limits.
#[derive(Clone, Debug)]
pub struct InactiveState {
    state: State,
}
impl InactiveState {
    pub fn from_verified_state(state: &State) -> io::Result<Self> {
        if state.nonces.len() > MAX_ENTRIES as usize
            || state.names.len() > MAX_ENTRIES as usize
            || state.height == 0
            || state.height > 1_000_000
            || (state.application_bytes() - 8) as u64 > MAX_PAYLOAD_BYTES
        {
            return Err(invalid("inactive state bounds"));
        }
        Ok(Self {
            state: state.clone(),
        })
    }
    pub fn anchor(&self) -> StateAnchor {
        StateAnchor {
            genesis: self.state.genesis_id,
            height: self.state.height,
            tip: self.state.tip,
            root: self.state.root(),
        }
    }
    pub fn accounts(&self) -> usize {
        self.state.nonces.len()
    }
    pub fn names(&self) -> usize {
        self.state.names.len()
    }
    pub fn name(&self, label: &str) -> Option<&[u8; 32]> {
        self.state.names.get(label)
    }
    pub fn nonce(&self, owner: &[u8; 32]) -> u64 {
        self.state.nonce(owner)
    }
    pub fn next_empty_block_root(&self) -> Result<Digest> {
        Ok(self.state.propose(Vec::new())?.header.state)
    }
    pub fn v1_snapshot(&self) -> Result<Vec<u8>> {
        self.state.snapshot_bytes()
    }
    pub fn uncached_root(&self) -> Digest {
        let mut state = self.state.clone();
        state.root_cache = OnceLock::new();
        state.root()
    }
    /// Controlled inactive fixtures or independently anchored records only.
    /// This validates structure/root, but does NOT supply a BFT certificate.
    pub fn from_ordered_records(
        genesis: Genesis,
        expected: StateAnchor,
        accounts: impl IntoIterator<Item = ([u8; 32], u64)>,
        names: impl IntoIterator<Item = (String, [u8; 32])>,
    ) -> io::Result<Self> {
        let mut builder = Builder::new(genesis, expected)?;
        for (key, nonce) in accounts {
            builder.account(key, nonce)?;
        }
        for (label, owner) in names {
            builder.name(&label, owner)?;
        }
        builder.finish(expected.root)
    }
}
struct Builder {
    state: State,
    previous_key: Option<[u8; 32]>,
    previous_name: String,
    counts: BTreeMap<[u8; 32], u64>,
    bytes: u64,
}
impl Builder {
    fn new(genesis: Genesis, expected: StateAnchor) -> io::Result<Self> {
        genesis.validate().map_err(invalid)?;
        if genesis.id() != expected.genesis || expected.height == 0 || expected.height > 1_000_000 {
            return Err(invalid("independent genesis/height"));
        }
        let mut state = State::from_validated_genesis(genesis);
        state.height = expected.height;
        state.tip = expected.tip;
        Ok(Self {
            state,
            previous_key: None,
            previous_name: String::new(),
            counts: BTreeMap::new(),
            bytes: 0,
        })
    }
    fn charge(&mut self, n: u64) -> io::Result<()> {
        self.bytes = self
            .bytes
            .checked_add(n)
            .ok_or_else(|| invalid("state byte overflow"))?;
        if self.bytes > MAX_PAYLOAD_BYTES {
            return Err(invalid("state allocation bound"));
        }
        Ok(())
    }
    fn account(&mut self, key: [u8; 32], nonce: u64) -> io::Result<()> {
        if self.state.nonces.len() >= MAX_ENTRIES as usize
            || self.previous_key.is_some_and(|last| last >= key)
            || nonce == 0
            || self.state.reserved_keys.contains(&key)
            || !self.previous_name.is_empty()
        {
            return Err(invalid("account count/order/nonce/role"));
        }
        if VerifyingKey::from_bytes(&key).map_err(invalid)?.is_weak() {
            return Err(invalid("weak account key"));
        }
        self.charge(40)?;
        self.state.nonces.insert(key, nonce);
        self.previous_key = Some(key);
        Ok(())
    }
    fn name(&mut self, label: &str, owner: [u8; 32]) -> io::Result<()> {
        if self.state.names.len() >= MAX_ENTRIES as usize
            || label <= self.previous_name.as_str()
            || !self.state.nonces.contains_key(&owner)
        {
            return Err(invalid("name count/order/owner"));
        }
        yami_chain_naming::validate_label(label).map_err(|e| invalid(format!("{e:?}")))?;
        self.charge((36 + label.len()) as u64)?;
        self.state
            .names
            .register(label, owner)
            .map_err(|e| invalid(format!("{e:?}")))?;
        self.previous_name = label.to_owned();
        *self.counts.entry(owner).or_default() += 1;
        Ok(())
    }
    fn finish(self, root: Digest) -> io::Result<InactiveState> {
        if !self.counts.iter().eq(self.state.nonces.iter()) || self.state.root() != root {
            return Err(invalid("registration counts/independent flat state root"));
        }
        Ok(InactiveState { state: self.state })
    }
}
pub fn import<R: Read>(
    genesis: Genesis,
    expected: Anchor,
    manifest: &Manifest,
    mut open: impl FnMut(u32) -> io::Result<R>,
) -> io::Result<InactiveState> {
    // Validate even a Manifest constructed by this module before any allocation.
    let manifest = Manifest::decode(&manifest.canonical(), expected)?;
    let mut builder = Builder::new(genesis, expected.state)?;
    for (i, c) in manifest.chunks.iter().enumerate() {
        let bytes = manifest.read_chunk(i, open(i as u32)?)?;
        let mut r = Reader::new(&bytes, MAX_CHUNK_BYTES).map_err(invalid)?;
        for _ in 0..c.records {
            if c.kind == 0 {
                builder.account(r.array().map_err(invalid)?, r.u64().map_err(invalid)?)?;
            } else {
                let label = std::str::from_utf8(r.bytes(63).map_err(invalid)?).map_err(invalid)?;
                builder.name(label, r.array().map_err(invalid)?)?;
            }
        }
        r.end().map_err(invalid)?;
    }
    if builder.state.nonces.len() != manifest.accounts as usize
        || builder.state.names.len() != manifest.names as usize
        || builder.bytes != manifest.total
    {
        return Err(invalid("decoded completeness"));
    }
    builder.finish(expected.state.root)
}
pub fn export(
    state: &InactiveState,
    mut write: impl FnMut(u32, &[u8]) -> io::Result<()>,
) -> io::Result<Manifest> {
    let mut manifest = Manifest {
        state: state.anchor(),
        accounts: state.accounts().try_into().map_err(invalid)?,
        names: state.names().try_into().map_err(invalid)?,
        total: 0,
        chunks: Vec::new(),
    };
    let mut buffer = Vec::with_capacity(MAX_CHUNK_BYTES);
    let mut records = 0u32;
    let mut first = 0u32;
    fn flush(
        manifest: &mut Manifest,
        buffer: &mut Vec<u8>,
        kind: u8,
        first: &mut u32,
        records: &mut u32,
        write: &mut impl FnMut(u32, &[u8]) -> io::Result<()>,
    ) -> io::Result<()> {
        if *records == 0 {
            return Ok(());
        }
        if manifest.chunks.len() >= MAX_CHUNKS {
            return Err(invalid("chunk work bound"));
        }
        let mut c = Chunk {
            kind,
            first: *first,
            records: *records,
            bytes: buffer.len() as u32,
            digest: Digest::default(),
        };
        let index = manifest.chunks.len() as u32;
        c.digest = c.digest(index, buffer);
        write(index, buffer)?;
        manifest.total += buffer.len() as u64;
        manifest.chunks.push(c);
        *first += *records;
        *records = 0;
        buffer.clear();
        Ok(())
    }
    for (key, nonce) in &state.state.nonces {
        if records == MAX_CHUNK_RECORDS || buffer.len() + 40 > MAX_CHUNK_BYTES {
            flush(
                &mut manifest,
                &mut buffer,
                0,
                &mut first,
                &mut records,
                &mut write,
            )?;
        }
        buffer.extend_from_slice(key);
        buffer.extend_from_slice(&nonce.to_be_bytes());
        records += 1;
    }
    flush(
        &mut manifest,
        &mut buffer,
        0,
        &mut first,
        &mut records,
        &mut write,
    )?;
    first = 0;
    for (label, owner) in state.state.names.entries() {
        let n = 36 + label.len();
        if records == MAX_CHUNK_RECORDS || buffer.len() + n > MAX_CHUNK_BYTES {
            flush(
                &mut manifest,
                &mut buffer,
                1,
                &mut first,
                &mut records,
                &mut write,
            )?;
        }
        buffer.extend_from_slice(&(label.len() as u32).to_be_bytes());
        buffer.extend_from_slice(label.as_bytes());
        buffer.extend_from_slice(owner);
        records += 1;
    }
    flush(
        &mut manifest,
        &mut buffer,
        1,
        &mut first,
        &mut records,
        &mut write,
    )?;
    Manifest::decode(&manifest.canonical(), manifest.anchor())
}
#[cfg(test)]
mod tests;
