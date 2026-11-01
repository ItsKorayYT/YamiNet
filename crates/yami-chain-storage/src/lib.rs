//! Bounded immutable finalized records and durable signing preimages.
//! Callers must authenticate certificates before appending or trusting records.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
use yami_chain::{
    codec::{Reader, Writer},
    Block, Digest, Genesis, State, MAX_BLOCK_BYTES,
};

pub const MAX_RECORD_BYTES: usize = MAX_BLOCK_BYTES + 4096 + 32;

#[derive(Clone, Debug)]
pub struct FinalizedRecord {
    pub block: Block,
    pub certificate: Vec<u8>,
}
impl FinalizedRecord {
    pub fn canonical(&self) -> io::Result<Vec<u8>> {
        let mut w = Writer::default();
        w.raw(b"YC-RECORD-1");
        w.bytes(&self.block.canonical().map_err(invalid)?)
            .map_err(invalid)?;
        if self.certificate.len() > 4096 {
            return Err(invalid("certificate cap"));
        }
        w.bytes(&self.certificate).map_err(invalid)?;
        Ok(w.0)
    }
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        let mut r = Reader::new(bytes, MAX_RECORD_BYTES).map_err(invalid)?;
        if r.take(11).map_err(invalid)? != b"YC-RECORD-1" {
            return Err(invalid("record domain"));
        }
        let block = Block::decode(r.bytes(MAX_BLOCK_BYTES).map_err(invalid)?).map_err(invalid)?;
        let certificate = r.bytes(4096).map_err(invalid)?.to_vec();
        r.end().map_err(invalid)?;
        Ok(Self { block, certificate })
    }
}
fn invalid(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

pub fn read_bounded(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(invalid("symlinked record"));
    }
    let f = File::open(path)?;
    if f.metadata()?.len() > limit as u64 {
        return Err(invalid("record cap"));
    }
    let mut bytes = Vec::new();
    f.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(invalid("record cap"));
    }
    Ok(bytes)
}

/// Exclusive OS lock is released by process exit, including abrupt termination.
pub struct Journal {
    path: PathBuf,
    _lock: File,
}
impl Journal {
    pub fn open(path: &Path) -> io::Result<Self> {
        fs::create_dir_all(path)?;
        if fs::symlink_metadata(path)?.file_type().is_symlink() {
            return Err(invalid("symlinked store"));
        }
        let lock_path = path.join("store.lock");
        if lock_path.exists() && fs::symlink_metadata(&lock_path)?.file_type().is_symlink() {
            return Err(invalid("symlinked lock"));
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        lock.try_lock()
            .map_err(|e| io::Error::other(format!("store already in use: {e}")))?;
        let journal = Self {
            path: path.to_owned(),
            _lock: lock,
        };
        // Only this process can mutate these files now. Unrenamed records were
        // never acknowledged. Malachite WAL has its own recovery semantics.
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if entry
                .file_name()
                .to_str()
                .is_some_and(|n| n.starts_with(".pending-"))
            {
                fs::remove_file(entry.path())?;
            }
        }
        journal.sync_dir()?;
        Ok(journal)
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    fn sync_dir(&self) -> io::Result<()> {
        #[cfg(unix)]
        File::open(&self.path)?.sync_all()?;
        Ok(())
    }
    /// Under the store lock; never replace a committed destination.
    pub fn write_once(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        {
            return Err(invalid("invalid record name"));
        }
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(invalid("record cap"));
        }
        let final_path = self.path.join(name);
        if final_path.exists() {
            if read_bounded(&final_path, MAX_RECORD_BYTES)? == bytes {
                return Ok(());
            }
            return Err(invalid("conflicting durable record"));
        }
        let temp = self.path.join(format!(".pending-{name}"));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, &final_path)?;
        self.sync_dir()
    }
    pub fn record(&self, height: u64) -> io::Result<Option<FinalizedRecord>> {
        let path = self.path.join(format!("finalized-{height:020}.bin"));
        match read_bounded(&path, MAX_RECORD_BYTES) {
            Ok(bytes) => FinalizedRecord::decode(&bytes).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    pub fn append(&self, record: &FinalizedRecord) -> io::Result<()> {
        self.write_once(
            &format!("finalized-{:020}.bin", record.block.header.height),
            &record.canonical()?,
        )
    }
    pub fn replay(
        &self,
        genesis: Genesis,
        verify: impl Fn(&FinalizedRecord) -> io::Result<()>,
    ) -> io::Result<State> {
        self.write_once("genesis.bin", &genesis.canonical())?;
        let mut heights = Vec::new();
        for entry in fs::read_dir(&self.path)? {
            let name = entry?.file_name();
            let name = name.to_string_lossy();
            if let Some(height) = name
                .strip_prefix("finalized-")
                .and_then(|s| s.strip_suffix(".bin"))
            {
                let value = height.parse::<u64>().map_err(invalid)?;
                if name != format!("finalized-{value:020}.bin") {
                    return Err(invalid("noncanonical filename"));
                }
                heights.push(value);
            }
        }
        heights.sort_unstable();
        let mut state = State::new(genesis).map_err(invalid)?;
        for height in heights {
            if height
                != state
                    .height()
                    .checked_add(1)
                    .ok_or_else(|| invalid("height overflow"))?
            {
                return Err(invalid("finalized history gap"));
            }
            let record = self
                .record(height)?
                .ok_or_else(|| invalid("missing record"))?;
            if record.block.header.height != height {
                return Err(invalid("record height mismatch"));
            }
            verify(&record)?;
            state = state.execute(&record.block).map_err(invalid)?;
        }
        Ok(state)
    }
    /// Persist BEFORE producing/releasing the signature, even if later work fails.
    pub fn reserve_signature(
        &self,
        height: u64,
        round: u32,
        step: u8,
        preimage: &[u8],
    ) -> io::Result<()> {
        self.write_once(
            &format!("signed-{height:020}-{round:010}-{step}.bin"),
            preimage,
        )
    }
}

/// Snapshot imports must be checked against an independently authenticated checkpoint.
pub struct Checkpoint {
    pub genesis: Digest,
    pub height: u64,
    pub block: Digest,
    pub state: Digest,
}
pub trait SnapshotVerifier {
    fn verify(&self, checkpoint: &Checkpoint, snapshot: &[u8]) -> io::Result<()>;
}
pub trait Pruner {
    fn prune_before(&mut self, checkpoint: &Checkpoint) -> io::Result<()>;
}
/// No snapshot trust/pruning policy is approved in 0.1.
pub struct NoPruning;
impl Pruner for NoPruning {
    fn prune_before(&mut self, _: &Checkpoint) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "pruning requires reviewed checkpoint and evidence policy",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static ID: AtomicU64 = AtomicU64::new(0);
    fn dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "yami-chain-storage-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ))
    }
    #[test]
    fn exclusive_lock_orphan_and_signer_recovery() {
        let path = dir();
        let j = Journal::open(&path).unwrap();
        assert!(Journal::open(&path).is_err());
        j.reserve_signature(1, 0, 1, b"first").unwrap();
        fs::write(
            path.join(".pending-finalized-00000000000000000001.bin"),
            b"torn",
        )
        .unwrap();
        drop(j);
        let j = Journal::open(&path).unwrap();
        j.reserve_signature(1, 0, 1, b"first").unwrap();
        assert!(j.reserve_signature(1, 0, 1, b"conflict").is_err());
        assert!(!path
            .join(".pending-finalized-00000000000000000001.bin")
            .exists());
        drop(j);
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn replay_rejects_gaps_and_corruption() {
        let path = dir();
        let j = Journal::open(&path).unwrap();
        let genesis = Genesis::local();
        let state = State::new(genesis.clone()).unwrap();
        let first = state.development_block().unwrap();
        let second = state.execute(&first).unwrap().development_block().unwrap();
        j.append(&FinalizedRecord {
            block: second,
            certificate: vec![],
        })
        .unwrap();
        assert!(j.replay(genesis.clone(), |_| Ok(())).is_err());
        j.append(&FinalizedRecord {
            block: first,
            certificate: vec![],
        })
        .unwrap();
        assert_eq!(j.replay(genesis.clone(), |_| Ok(())).unwrap().height(), 2);
        assert!(j
            .replay(genesis.clone(), |_| Err(invalid("bad certificate")))
            .is_err());
        fs::write(path.join("finalized-00000000000000000001.bin"), b"broken").unwrap();
        assert!(j.replay(genesis, |_| Ok(())).is_err());
        drop(j);
        fs::remove_dir_all(path).unwrap();
    }
}
