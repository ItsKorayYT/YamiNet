//! Canonical compact state codec. Authenticity comes from the caller's checkpoint,
//! never from the encoded state's own hash or a transport-supplied commitment.
use crate::*;
pub const MAX_STATE_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_STATE_SNAPSHOT_ENTRIES: usize = 100_000;
impl State {
    pub fn snapshot_len(&self) -> Result<usize> {
        if self.nonces.len() > MAX_STATE_SNAPSHOT_ENTRIES
            || self.names.len() > MAX_STATE_SNAPSHOT_ENTRIES
        {
            return Err(Error::Limit);
        }
        let length = 10 + 32 + 8 + 32 + self.application_bytes();
        if length > MAX_STATE_SNAPSHOT_BYTES {
            return Err(Error::Limit);
        }
        Ok(length)
    }
    pub fn snapshot_bytes(&self) -> Result<Vec<u8>> {
        let mut bytes = Vec::with_capacity(self.snapshot_len()?);
        self.append_snapshot(&mut bytes)?;
        Ok(bytes)
    }
    /// Append the unchanged canonical snapshot without an intermediate buffer.
    /// Limits are checked before modifying the destination.
    pub fn append_snapshot(&self, output: &mut Vec<u8>) -> Result<()> {
        let length = self.snapshot_len()?;
        output.len().checked_add(length).ok_or(Error::Limit)?;
        output.reserve(length);
        self.visit_snapshot_bytes(|bytes| output.extend_from_slice(bytes));
        Ok(())
    }
    fn visit_snapshot_bytes(&self, mut write: impl FnMut(&[u8])) {
        write(b"YC-STATE-1");
        write(&self.genesis_id.0);
        write(&self.height.to_be_bytes());
        write(&self.tip.0);
        self.visit_application_bytes(write);
    }
    fn snapshot_matches(&self, bytes: &[u8]) -> bool {
        if self.snapshot_len().ok() != Some(bytes.len()) {
            return false;
        }
        let mut offset = 0;
        let mut equal = true;
        self.visit_snapshot_bytes(|part| {
            equal &= bytes.get(offset..offset + part.len()) == Some(part);
            offset += part.len();
        });
        equal
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
        let mut state = Self::from_validated_genesis(genesis);
        let mut r = Reader::new(bytes, MAX_STATE_SNAPSHOT_BYTES)?;
        if r.take(10)? != b"YC-STATE-1"
            || r.array::<32>()? != state.genesis_id.0
            || r.u64()? != height
            || r.array::<32>()? != tip.0
            || height == 0
            || height > 1_000_000
        {
            return Err(Error::Header);
        }
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
                || state.reserved_keys.contains(&key)
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
        let mut previous = "";
        let mut names = yami_chain_naming::SortedNamesBuilder::default();
        let mut counts = std::collections::BTreeMap::<[u8; 32], u64>::new();
        for _ in 0..n {
            let label = std::str::from_utf8(r.bytes(63)?).map_err(|_| Error::Encoding)?;
            let owner = r.array::<32>()?;
            if label <= previous || !state.nonces.contains_key(&owner) {
                return Err(Error::Encoding);
            }
            names.push(label, owner).map_err(|_| Error::Naming)?;
            previous = label;
            *counts.entry(owner).or_default() += 1;
        }
        state.names = names.finish();
        r.end()?;
        // The current module only registers names; nonce counters must account
        // for every successful registration. Future modules need a new codec.
        if !counts.iter().eq(state.nonces.iter())
            || state.root() != root
            || !state.snapshot_matches(bytes)
        {
            return Err(Error::Header);
        }
        Ok(state)
    }
}
