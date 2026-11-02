//! Bounded immutable finalized records and durable signing preimages.
//! Callers must authenticate certificates before appending or trusting records.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};
use yami_chain::{
    codec::{Reader, Writer},
    Block, Digest, Genesis, State, MAX_BLOCK_BYTES, MAX_CERTIFICATE_BYTES,
};

pub const MAX_RECORD_BYTES: usize = MAX_BLOCK_BYTES + MAX_CERTIFICATE_BYTES + 32;
/// A budget is a refusal bound, never permission to skip verification or return
/// partially replayed state as a trusted tip. Archive operators can raise it.
#[derive(Clone, Copy, Debug)]
pub struct ReplayBudget {
    pub max_records: u64,
    pub max_bytes: u64,
}
impl Default for ReplayBudget {
    fn default() -> Self {
        Self {
            max_records: 1_000_000,
            max_bytes: 20 * 1024 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WriteStage {
    FileSync,
    Renamed,
    DirectorySync,
}

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
        if self.certificate.len() > MAX_CERTIFICATE_BYTES {
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
        let certificate = r.bytes(MAX_CERTIFICATE_BYTES).map_err(invalid)?.to_vec();
        r.end().map_err(invalid)?;
        Ok(Self { block, certificate })
    }
}
fn invalid(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

pub fn read_bounded(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(invalid("record must be a regular file"));
    }
    let f = File::open(path)?;
    if f.metadata()?.len() > limit as u64 {
        return Err(invalid("record cap"));
    }
    let mut bytes = Vec::new();
    f.take((limit as u64).saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(invalid("record cap"));
    }
    Ok(bytes)
}

/// Exclusive OS lock is released by process exit, including abrupt termination.
pub struct Journal {
    path: PathBuf,
    _lock: File,
    write_lock: Mutex<()>,
}
impl Journal {
    pub fn open(path: &Path) -> io::Result<Self> {
        fs::create_dir_all(path)?;
        if fs::symlink_metadata(path)?.file_type().is_symlink() {
            return Err(invalid("symlinked store"));
        }
        let lock_path = path.join("store.lock");
        if fs::symlink_metadata(&lock_path).is_ok_and(|m| !m.file_type().is_file()) {
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
            write_lock: Mutex::new(()),
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
        self.write_once_inner(name, bytes, |_| Ok(()))
    }
    // The hook is private and used only to exercise I/O failure boundaries.
    fn write_once_inner(
        &self,
        name: &str,
        bytes: &[u8],
        mut boundary: impl FnMut(WriteStage) -> io::Result<()>,
    ) -> io::Result<()> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| invalid("poisoned store lock"))?;
        if !name
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        {
            return Err(invalid("invalid record name"));
        }
        let limit = if name == "genesis.bin" {
            yami_chain::committee::MAX_GENESIS_BYTES
        } else {
            MAX_RECORD_BYTES
        };
        if bytes.len() > limit {
            return Err(invalid("record cap"));
        }
        let final_path = self.path.join(name);
        match fs::symlink_metadata(&final_path) {
            Ok(_) => {
                if read_bounded(&final_path, limit)? != bytes {
                    return Err(invalid("conflicting durable record"));
                }
                // A previous attempt may have renamed successfully but failed
                // directory sync. Never release a signature on an unsynced retry.
                boundary(WriteStage::FileSync)?;
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&final_path)?
                    .sync_all()?;
                boundary(WriteStage::DirectorySync)?;
                return self.sync_dir();
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let temp = self.path.join(format!(".pending-{name}"));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let result = (|| {
            file.write_all(bytes)?;
            boundary(WriteStage::FileSync)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temp, &final_path)?;
            boundary(WriteStage::Renamed)?;
            boundary(WriteStage::DirectorySync)?;
            self.sync_dir()
        })();
        if result.is_err() {
            // Only the temporary name is uncommitted. Preserve a destination
            // whose rename succeeded, even when its durability is uncertain.
            let _ = fs::remove_file(&temp);
        }
        result
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
        self.replay_with_budget(genesis, ReplayBudget::default(), verify)
    }
    pub fn replay_with_budget(
        &self,
        genesis: Genesis,
        budget: ReplayBudget,
        verify: impl Fn(&FinalizedRecord) -> io::Result<()>,
    ) -> io::Result<State> {
        let mut state = State::new(genesis.clone()).map_err(invalid)?;
        self.write_once("genesis.bin", &genesis.canonical())?;
        let mut remaining = budget.max_bytes;
        // Replay in height order with constant history-index memory. A second
        // streaming directory pass rejects gaps and noncanonical filenames.
        while let Some(record) = self.record(
            state
                .height()
                .checked_add(1)
                .ok_or_else(|| invalid("height overflow"))?,
        )? {
            if state.height() >= budget.max_records {
                return Err(invalid("replay record budget exceeded"));
            }
            remaining = remaining
                .checked_sub(record.canonical()?.len() as u64)
                .ok_or_else(|| invalid("replay byte budget exceeded"))?;
            if record.block.header.height != state.height() + 1 {
                return Err(invalid("record height mismatch"));
            }
            verify(&record)?;
            state = state.execute(&record.block).map_err(invalid)?;
        }
        for entry in fs::read_dir(&self.path)? {
            let name = entry?.file_name();
            let name = name.to_string_lossy();
            if let Some(height) = name
                .strip_prefix("finalized-")
                .and_then(|s| s.strip_suffix(".bin"))
            {
                let value = height.parse::<u64>().map_err(invalid)?;
                if name != format!("finalized-{value:020}.bin")
                    || value == 0
                    || value > state.height()
                {
                    return Err(invalid("noncanonical filename or finalized history gap"));
                }
            }
        }
        Ok(state)
    }
    /// Delete only reproducible payloads at already authenticated finalized heights.
    /// Signing records, evidence, WAL and certified history are never removed.
    pub fn remove_finalized_candidates(&self, finalized: u64) -> io::Result<()> {
        for entry in fs::read_dir(&self.path)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let tail = name
                .strip_prefix("candidate-")
                .or_else(|| name.strip_prefix("proposal-"));
            if let Some(height) = tail
                .and_then(|s| s.split('-').next())
                .and_then(|s| s.parse::<u64>().ok())
            {
                if height <= finalized {
                    fs::remove_file(entry.path())?;
                }
            }
        }
        self.sync_dir()
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
    #[test]
    fn invalid_names_and_non_regular_records_are_rejected() {
        let path = dir();
        let j = Journal::open(&path).unwrap();
        for name in ["", ".", "..", "../outside", ".pending-hidden", "a/b"] {
            assert!(j.write_once(name, b"bad").is_err());
        }
        assert!(read_bounded(&path, 100).is_err());
        fs::write(path.join("oversize"), [0; 17]).unwrap();
        assert!(read_bounded(&path.join("oversize"), 16).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(path.join("absent"), path.join("dangling")).unwrap();
            assert!(j.write_once("dangling", b"must not replace").is_err());
            assert!(fs::symlink_metadata(path.join("dangling"))
                .unwrap()
                .file_type()
                .is_symlink());
        }
        drop(j);
        fs::remove_dir_all(path).unwrap();
    }
}

#[cfg(test)]
mod genesis_limits {
    use super::*;
    #[test]
    fn full_scheduled_genesis_is_bounded_and_invalid_genesis_does_not_poison_store() {
        let path = std::env::temp_dir().join(format!("yami-large-genesis-{}", std::process::id()));
        let journal = Journal::open(&path).unwrap();
        let mut invalid_genesis = Genesis::local();
        invalid_genesis.epochs.clear();
        assert!(journal.replay(invalid_genesis, |_| Ok(())).is_err());
        assert!(!path.join("genesis.bin").exists());
        let mut genesis = Genesis::development_schedule(&[(1, 100)]).unwrap();
        let set = genesis.epochs[0].validators.clone();
        genesis.epochs = (1..=64)
            .map(|first_height| yami_chain::Epoch {
                first_height,
                validators: set.clone(),
            })
            .collect();
        assert!(genesis.canonical().len() > MAX_RECORD_BYTES);
        assert_eq!(
            journal
                .replay(genesis.clone(), |_| Ok(()))
                .unwrap()
                .height(),
            0
        );
        assert_eq!(journal.replay(genesis, |_| Ok(())).unwrap().height(), 0);
        drop(journal);
        fs::remove_dir_all(path).unwrap();
    }
}

#[cfg(test)]
mod io_boundaries {
    use super::*;
    #[test]
    fn failed_temporary_writes_retry_and_uncertain_renames_resync_without_equivocation() {
        let path = std::env::temp_dir().join(format!("yami-io-boundaries-{}", std::process::id()));
        let journal = Journal::open(&path).unwrap();
        for stage in [
            WriteStage::FileSync,
            WriteStage::Renamed,
            WriteStage::DirectorySync,
        ] {
            let name = format!("signed-1-0-{stage:?}.bin");
            assert!(journal
                .write_once_inner(&name, b"reserved", |at| {
                    if at == stage {
                        Err(io::Error::other("injected storage failure"))
                    } else {
                        Ok(())
                    }
                })
                .is_err());
            assert!(!path.join(format!(".pending-{name}")).exists());
            let mut directory_synced = false;
            journal
                .write_once_inner(&name, b"reserved", |at| {
                    if at == WriteStage::DirectorySync {
                        directory_synced = true;
                    }
                    Ok(())
                })
                .unwrap();
            assert!(directory_synced);
            assert!(journal.write_once(&name, b"conflicting preimage").is_err());
        }
        drop(journal);
        let journal = Journal::open(&path).unwrap();
        assert!(journal
            .write_once("signed-1-0-Renamed.bin", b"conflicting preimage")
            .is_err());
        drop(journal);
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn replay_budgets_refuse_without_trusting_partial_history() {
        let path = std::env::temp_dir().join(format!("yami-replay-budget-{}", std::process::id()));
        let journal = Journal::open(&path).unwrap();
        let genesis = Genesis::local();
        let mut state = State::new(genesis.clone()).unwrap();
        let mut bytes = 0;
        for _ in 0..3 {
            let block = state.development_block().unwrap();
            state = state.execute(&block).unwrap();
            let record = FinalizedRecord {
                block,
                certificate: vec![],
            };
            bytes += record.canonical().unwrap().len() as u64;
            journal.append(&record).unwrap();
        }
        for budget in [
            ReplayBudget {
                max_records: 2,
                max_bytes: bytes,
            },
            ReplayBudget {
                max_records: 3,
                max_bytes: bytes - 1,
            },
        ] {
            assert!(journal
                .replay_with_budget(genesis.clone(), budget, |_| Ok(()))
                .is_err());
        }
        assert_eq!(
            journal
                .replay_with_budget(
                    genesis,
                    ReplayBudget {
                        max_records: 3,
                        max_bytes: bytes
                    },
                    |_| Ok(())
                )
                .unwrap(),
            state
        );
        drop(journal);
        fs::remove_dir_all(path).unwrap();
    }
}

/// Bounded, read-only pressure diagnostics. Candidates remain safety-relevant
/// until finality. This sample is not an atomic snapshot or a deletion policy.
#[derive(Debug, Default)]
pub struct StorageFootprint {
    pub entries: u64,
    pub bytes: u64,
    pub signing_bytes: u64,
    pub evidence_bytes: u64,
    pub wal_bytes: u64,
    pub retained_candidate_bytes: u64,
}
impl Journal {
    pub fn storage_footprint(&self, max_entries: u64) -> io::Result<StorageFootprint> {
        let mut report = StorageFootprint::default();
        let mut directories = vec![self.path.clone()];
        while let Some(directory) = directories.pop() {
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                report.entries = report
                    .entries
                    .checked_add(1)
                    .ok_or_else(|| invalid("entry overflow"))?;
                if report.entries > max_entries {
                    return Err(invalid("storage inspection budget exceeded"));
                }
                let metadata = match fs::symlink_metadata(entry.path()) {
                    Ok(m) => m,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(e),
                };
                if metadata.is_dir() && entry.path() == self.path.join("wal") {
                    directories.push(entry.path());
                    continue;
                }
                if !metadata.file_type().is_file() {
                    return Err(invalid("unexpected storage entry type"));
                }
                let size = metadata.len();
                report.bytes = report
                    .bytes
                    .checked_add(size)
                    .ok_or_else(|| invalid("storage size overflow"))?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if entry.path().parent() == Some(self.path.join("wal").as_path()) {
                    report.wal_bytes += size;
                } else if name.starts_with("signed-") {
                    report.signing_bytes += size;
                } else if name.starts_with("equivocation-") {
                    report.evidence_bytes += size;
                } else if name.starts_with("candidate-") || name.starts_with("proposal-") {
                    report.retained_candidate_bytes += size;
                }
            }
        }
        Ok(report)
    }
}
#[cfg(test)]
mod pressure_tests {
    use super::*;
    #[test]
    fn bounded_pressure_sampling_keeps_all_safety_records() {
        let path = std::env::temp_dir().join(format!("yami-pressure-{}", std::process::id()));
        let journal = Journal::open(&path).unwrap();
        for name in [
            "signed-fixture.bin",
            "equivocation-fixture.bin",
            "candidate-fixture.bin",
        ] {
            journal.write_once(name, &[1; 8]).unwrap();
        }
        fs::create_dir(path.join("wal")).unwrap();
        fs::write(path.join("wal/consensus.wal"), [2; 16]).unwrap();
        assert!(journal.storage_footprint(1).is_err());
        let report = journal.storage_footprint(20).unwrap();
        assert_eq!(
            (
                report.signing_bytes,
                report.evidence_bytes,
                report.retained_candidate_bytes,
                report.wal_bytes
            ),
            (8, 8, 8, 16)
        );
        assert_eq!(fs::read(path.join("signed-fixture.bin")).unwrap(), [1; 8]);
        drop(journal);
        fs::remove_dir_all(path).unwrap();
    }
}
