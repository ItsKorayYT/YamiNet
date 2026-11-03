//! Canonical compact state codec. Authenticity comes from the caller's checkpoint,
//! never from the encoded state's own hash or a transport-supplied commitment.
use crate::*;
pub const MAX_STATE_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_STATE_SNAPSHOT_ENTRIES: usize = 100_000;
impl State {
    pub fn snapshot_bytes(&self) -> Result<Vec<u8>> {
        let names: Vec<_> = self
            .names
            .entries()
            .take(MAX_STATE_SNAPSHOT_ENTRIES + 1)
            .collect();
        if self.nonces.len() > MAX_STATE_SNAPSHOT_ENTRIES
            || names.len() > MAX_STATE_SNAPSHOT_ENTRIES
        {
            return Err(Error::Limit);
        }
        let mut w = Writer::default();
        w.raw(b"YC-STATE-1");
        w.raw(&self.genesis.id().0);
        w.u64(self.height);
        w.raw(&self.tip.0);
        w.u32(self.nonces.len() as u32);
        for (key, nonce) in &self.nonces {
            w.raw(key);
            w.u64(*nonce);
        }
        w.u32(names.len() as u32);
        for (label, owner) in names {
            w.bytes(label.as_bytes())?;
            w.raw(owner);
        }
        if w.0.len() > MAX_STATE_SNAPSHOT_BYTES {
            return Err(Error::Limit);
        }
        Ok(w.0)
    }
    /// `root`, `height` and `tip` MUST be independently authenticated by a BFT
    /// checkpoint/trust policy. This codec alone grants no snapshot authority.
    pub fn restore_checkpoint(
        genesis: Genesis,
        height: u64,
        tip: Digest,
        root: Digest,
        bytes: &[u8],
    ) -> Result<Self> {
        genesis.validate()?;
        let mut r = Reader::new(bytes, MAX_STATE_SNAPSHOT_BYTES)?;
        if r.take(10)? != b"YC-STATE-1"
            || r.array::<32>()? != genesis.id().0
            || r.u64()? != height
            || r.array::<32>()? != tip.0
            || height == 0
            || height > 1_000_000
        {
            return Err(Error::Header);
        }
        let mut state = State::new(genesis)?;
        state.height = height;
        state.tip = tip;
        let n = r.u32()? as usize;
        if n > MAX_STATE_SNAPSHOT_ENTRIES {
            return Err(Error::Limit);
        }
        let mut previous = None;
        for _ in 0..n {
            let key = r.array::<32>()?;
            let nonce = r.u64()?;
            if previous.is_some_and(|p| p >= key)
                || nonce == 0
                || state.genesis.is_reserved_key(&key)
            {
                return Err(Error::Encoding);
            }
            if VerifyingKey::from_bytes(&key)
                .map_err(|_| Error::Signature)?
                .is_weak()
            {
                return Err(Error::Signature);
            }
            previous = Some(key);
            state.nonces.insert(key, nonce);
        }
        let n = r.u32()? as usize;
        if n > MAX_STATE_SNAPSHOT_ENTRIES {
            return Err(Error::Limit);
        }
        let mut previous = String::new();
        let mut counts = BTreeMap::<[u8; 32], u64>::new();
        for _ in 0..n {
            let label = std::str::from_utf8(r.bytes(63)?).map_err(|_| Error::Encoding)?;
            let owner = r.array::<32>()?;
            if label <= previous.as_str() || !state.nonces.contains_key(&owner) {
                return Err(Error::Encoding);
            }
            state
                .names
                .register(label, owner)
                .map_err(|_| Error::Naming)?;
            previous = label.to_owned();
            *counts.entry(owner).or_default() += 1;
        }
        r.end()?;
        // The current module only registers names; nonce counters must account
        // for every successful registration. Future modules need a new codec.
        if counts != state.nonces || state.root() != root || state.snapshot_bytes()? != bytes {
            return Err(Error::Header);
        }
        Ok(state)
    }
}
