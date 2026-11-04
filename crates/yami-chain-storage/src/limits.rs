//! Refusal limits; never delete safety data to satisfy a quota.
use super::*;
pub const MIN_FREE_INODES: u64 = 128;
pub const MAX_COMPACT_BASE_BYTES: usize = 17 * 1024 * 1024;
#[derive(Clone, Copy, Debug)]
pub struct StorageLimits {
    pub max_bytes: u64,
    pub reserve_bytes: u64,
}
impl Default for StorageLimits {
    fn default() -> Self {
        Self {
            max_bytes: 18_000_000_000,
            reserve_bytes: 64 * 1024 * 1024,
        }
    }
}
// Filesystems reporting no inode accounting (f_files == 0) still fail closed
// on actual create/write/fsync errors. A preflight cannot replace an OS quota.
#[cfg(unix)]
fn check_filesystem_reserve(available: u128, files: u64, free: u64, needed: u64) -> io::Result<()> {
    if available < u128::from(needed) {
        return Err(io::Error::new(
            io::ErrorKind::StorageFull,
            "filesystem reserve exhausted",
        ));
    }
    if files != 0 && free < MIN_FREE_INODES {
        return Err(io::Error::new(
            io::ErrorKind::StorageFull,
            "filesystem inode reserve exhausted",
        ));
    }
    Ok(())
}
/// Conservative allocation credits are never trusted across restart. Rebuild
/// from actual files, then precharge BEFORE I/O; failed/uncertain writes overcount.
#[derive(Default)]
pub(super) struct Accounting {
    pub(super) footprint: Option<StorageFootprint>,
    writes: u16,
    audits: u64,
}
impl Journal {
    /// Explicit/periodic reconciliation includes all files and externally written
    /// WAL bytes. This is not an atomic filesystem quota against outside writers.
    pub fn ensure_capacity(&self, additional: u64) -> io::Result<()> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| invalid("poisoned store lock"))?;
        let measured = self.storage_footprint(DEFAULT_ENTRY_BUDGET);
        {
            let mut account = self
                .accounting
                .lock()
                .map_err(|_| invalid("poisoned storage accounting"))?;
            account.footprint = measured.as_ref().ok().cloned();
            account.writes = 0;
            account.audits += 1;
        }
        self.check_capacity(&measured?, additional, 1, 2)
    }
    /// Live filesystem/WAL preflight using conservative credits. The periodic
    /// explicit audit still reconciles outside writers; this does not replace it.
    pub fn check_cached_capacity(&self) -> io::Result<()> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| invalid("poisoned store lock"))?;
        self.reserve_capacity(0, 0, 0)
    }
    pub fn accounting_audits(&self) -> io::Result<u64> {
        Ok(self
            .accounting
            .lock()
            .map_err(|_| invalid("poisoned storage accounting"))?
            .audits)
    }
    // All callers hold write_lock. A fresh audit every 256 write credits bounds
    // conservative excess. WAL is sampled live on every credit, even in between.
    pub(super) fn reserve_capacity(
        &self,
        additional: u64,
        allocations: u64,
        entries: u64,
    ) -> io::Result<()> {
        let mut account = self
            .accounting
            .lock()
            .map_err(|_| invalid("poisoned storage accounting"))?;
        if account.footprint.is_none() || account.writes >= 256 {
            account.footprint = Some(self.storage_footprint(DEFAULT_ENTRY_BUDGET)?);
            account.writes = 0;
            account.audits += 1;
        }
        let footprint = account
            .footprint
            .as_mut()
            .ok_or_else(|| invalid("missing storage accounting"))?;
        // The Malachite writer is independent of Journal. Never cache its length.
        let mut wal_bytes = 0u64;
        let mut wal_allocated = 0u64;
        let mut wal_entries = 0u64;
        match fs::read_dir(self.path.join("wal")) {
            Ok(wal) => {
                for entry in wal {
                    let m = fs::symlink_metadata(entry?.path())?;
                    if !m.is_file() {
                        return Err(invalid("unexpected WAL entry type"));
                    }
                    wal_entries += 1;
                    if wal_entries > DEFAULT_ENTRY_BUDGET {
                        return Err(invalid("WAL entry work limit"));
                    }
                    wal_bytes = wal_bytes
                        .checked_add(m.len())
                        .ok_or_else(|| invalid("WAL byte overflow"))?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt as _;
                        wal_allocated = wal_allocated
                            .checked_add(m.blocks().saturating_mul(512))
                            .ok_or_else(|| invalid("WAL allocation overflow"))?;
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        // Conservatively add the whole live WAL, rather than subtracting a stale
        // sample: an external truncation can never create negative accounting.
        let mut checked = footprint.clone();
        checked.bytes = checked.bytes.saturating_add(wal_bytes);
        checked.allocated_bytes = checked.allocated_bytes.saturating_add(wal_allocated);
        checked.entries = checked
            .entries
            .saturating_add(wal_entries.saturating_add(1));
        self.check_capacity(&checked, additional, allocations, entries.saturating_add(2))?;
        footprint.bytes = footprint.bytes.saturating_add(additional);
        footprint.allocated_bytes = footprint
            .allocated_bytes
            .saturating_add(self.allocation_charge(additional, allocations)?);
        footprint.entries = footprint.entries.saturating_add(entries);
        account.writes += 1;
        Ok(())
    }
    fn allocation_charge(&self, additional: u64, allocations: u64) -> io::Result<u64> {
        #[cfg(unix)]
        let block = rustix::fs::statvfs(&self.path)?.f_frsize.max(4096);
        #[cfg(not(unix))]
        let block = 4096;
        Ok(additional
            .div_ceil(block)
            .saturating_mul(block)
            .saturating_add(allocations.saturating_sub(1).saturating_mul(block)))
    }
    fn check_capacity(
        &self,
        footprint: &StorageFootprint,
        additional: u64,
        allocations: u64,
        entries: u64,
    ) -> io::Result<()> {
        if footprint.entries.saturating_add(entries) > DEFAULT_ENTRY_BUDGET {
            return Err(io::Error::new(
                io::ErrorKind::StorageFull,
                "store entry reserve exhausted; safety data retained",
            ));
        }
        if footprint
            .bytes
            .checked_add(additional)
            .and_then(|n| n.checked_add(self.limits.reserve_bytes))
            .is_none_or(|n| n > self.limits.max_bytes)
        {
            return Err(io::Error::new(
                io::ErrorKind::StorageFull,
                "store quota/reserve exhausted; safety data retained",
            ));
        }
        #[cfg(unix)]
        {
            let stats = rustix::fs::statvfs(&self.path)?;
            let allocation = self.allocation_charge(additional, allocations)?;
            if footprint
                .allocated_bytes
                .saturating_add(allocation)
                .saturating_add(self.limits.reserve_bytes)
                > self.limits.max_bytes
            {
                return Err(io::Error::new(
                    io::ErrorKind::StorageFull,
                    "allocated store quota/reserve exhausted; safety data retained",
                ));
            }
            check_filesystem_reserve(
                u128::from(stats.f_bavail) * u128::from(stats.f_frsize),
                stats.f_files,
                stats.f_favail,
                allocation.saturating_add(self.limits.reserve_bytes),
            )?;
        }
        Ok(())
    }
    pub fn replace_network_hints(&self, bytes: &[u8]) -> io::Result<()> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| invalid("poisoned store lock"))?;
        if bytes.len() > 32 + 1 + 100 * 110 {
            return Err(invalid("network hints cap"));
        }
        self.signing_replace_metadata("network-hints.bin", bytes)
    }
    fn signing_replace_metadata(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        self.reserve_capacity(bytes.len() as u64, 1, 1)?;
        let temp = self.path.join(format!(".pending-{name}"));
        let result = (|| {
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            f.write_all(bytes)?;
            f.sync_all()?;
            fs::rename(&temp, self.path.join(name))?;
            self.sync_dir()
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
    /// Atomic replacement of the ONE active compact base only. Caller has
    /// authenticated the replacement; fsync must succeed before any deletion.
    pub fn install_compact_base(&self, bytes: &[u8]) -> io::Result<()> {
        if self.is_segmented_archive() {
            return Err(invalid("segmented archive cannot be pruned"));
        }
        self.install_compact_base_inner(bytes, |_| Ok(()))
    }
    fn install_compact_base_inner(
        &self,
        bytes: &[u8],
        mut boundary: impl FnMut(WriteStage) -> io::Result<()>,
    ) -> io::Result<()> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| invalid("poisoned store lock"))?;
        if bytes.len() > MAX_COMPACT_BASE_BYTES {
            return Err(invalid("compact base cap"));
        }
        self.reserve_capacity(bytes.len() as u64, 1, 1)?;
        let destination = self.path.join("compact-base.bin");
        if fs::symlink_metadata(&destination).is_ok_and(|m| !m.file_type().is_file()) {
            return Err(invalid("invalid compact base type"));
        }
        let temporary = self.path.join(".pending-compact-base.bin");
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let result = (|| {
            f.write_all(bytes)?;
            boundary(WriteStage::FileSync)?;
            f.sync_all()?;
            fs::rename(&temporary, destination)?;
            boundary(WriteStage::Renamed)?;
            boundary(WriteStage::DirectorySync)?;
            self.sync_dir()
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
    /// Durable minimum trusted height/block/root. Publish after the new base and
    /// before unlinking, so replacement by an older valid base fails closed.
    pub fn install_compact_marker(&self, bytes: &[u8]) -> io::Result<()> {
        if self.is_segmented_archive() {
            return Err(invalid("segmented archive cannot be pruned"));
        }
        if bytes.len() != 104 {
            return Err(invalid("compact marker size"));
        }
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| invalid("poisoned store lock"))?;
        self.reserve_capacity(104, 1, 1)?;
        let destination = self.path.join("compacted.bin");
        if fs::symlink_metadata(&destination).is_ok_and(|m| !m.file_type().is_file()) {
            return Err(invalid("invalid compact marker type"));
        }
        let temporary = self.path.join(".pending-compacted.bin");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, destination)?;
            self.sync_dir()
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
    /// Only the consensus checkpoint capability calls this after fsync of the
    /// verified base. Never matches signing reservations, evidence or WAL.
    pub fn remove_checkpoint_history(
        &self,
        verify_base: impl Fn(&[u8]) -> io::Result<State>,
    ) -> io::Result<()> {
        self.remove_checkpoint_history_observed(verify_base, || {})
    }
    pub fn remove_checkpoint_history_observed(
        &self,
        verify_base: impl Fn(&[u8]) -> io::Result<State>,
        mut removed: impl FnMut(),
    ) -> io::Result<()> {
        if self.is_segmented_archive() {
            return Err(invalid("segmented archive cannot be pruned"));
        }
        if !self.path.join("compacted.bin").is_file()
            || !self.path.join("compact-base.bin").is_file()
        {
            return Err(invalid("durable compact base required"));
        }
        let base = verify_base(&read_bounded(
            &self.path.join("compact-base.bin"),
            MAX_COMPACT_BASE_BYTES,
        )?)?;
        let marker = read_bounded(&self.path.join("compacted.bin"), 104)?;
        let mut r = Reader::new(&marker, 104).map_err(invalid)?;
        if r.array::<32>().map_err(invalid)? != base.genesis().id().0
            || r.u64().map_err(invalid)? != base.height()
            || r.array::<32>().map_err(invalid)? != base.tip().0
            || r.array::<32>().map_err(invalid)? != base.root().0
        {
            return Err(invalid("pruning marker mismatch"));
        }
        r.end().map_err(invalid)?;
        let floor = base.height();
        for entry in fs::read_dir(&self.path)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            for prefix in ["finalized", "receipt"] {
                if let Some(value) = name
                    .strip_prefix(&format!("{prefix}-"))
                    .and_then(|s| s.strip_suffix(".bin"))
                    .and_then(|s| s.parse::<u64>().ok())
                {
                    if value > 0 && value <= floor && name == format!("{prefix}-{value:020}.bin") {
                        fs::remove_file(entry.path())?;
                        removed();
                    }
                }
            }
        }
        self.sync_dir()
    }
}

/// Publish a bounded export atomically without overwriting any existing file.
/// An interrupted unlinked temporary export never becomes an accepted snapshot.
pub fn publish_checkpoint_export(path: &Path, bytes: &[u8]) -> io::Result<()> {
    publish_checkpoint_export_observed(path, bytes, || {})
}
pub fn publish_checkpoint_export_observed(
    path: &Path,
    bytes: &[u8],
    mut temporary_written: impl FnMut(),
) -> io::Result<()> {
    if bytes.len() > MAX_COMPACT_BASE_BYTES {
        return Err(invalid("checkpoint export cap"));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| invalid("checkpoint filename"))?;
    let temporary = parent.join(format!(".pending-export-{filename}"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(bytes)?;
        temporary_written();
        file.sync_all()?;
        fs::hard_link(&temporary, path)?;
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    let _ = fs::remove_file(temporary);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn directory() -> PathBuf {
        static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "yc-limits-{}-{}",
            std::process::id(),
            ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
    }
    #[cfg(unix)]
    #[test]
    fn physical_blocks_and_inode_reserves_refuse_before_writes() {
        assert!(check_filesystem_reserve(10000, 1000, 127, 1).is_err());
        assert!(check_filesystem_reserve(10000, 1000, 128, 1).is_ok());
        assert!(check_filesystem_reserve(0, 0, 0, 1).is_err());
        assert!(check_filesystem_reserve(10000, 0, 0, 1).is_ok());
        let path = directory();
        let j = Journal::open(&path).unwrap();
        j.write_once("signed-00000000000000000001-0000000000-1.bin", b"a")
            .unwrap();
        j.write_once("signed-00000000000000000002-0000000000-1.bin", b"b")
            .unwrap();
        let footprint = j.storage_footprint(100).unwrap();
        assert_eq!(footprint.bytes, 2);
        assert!(footprint.allocated_bytes > footprint.bytes);
        drop(j);
        let j = Journal::open_with_limits(
            &path,
            100,
            StorageLimits {
                max_bytes: footprint.allocated_bytes,
                reserve_bytes: 0,
            },
        )
        .unwrap();
        assert_eq!(
            j.write_once("signed-00000000000000000003-0000000000-1.bin", b"c")
                .unwrap_err()
                .kind(),
            io::ErrorKind::StorageFull
        );
        drop(j);
        let j = Journal::open(&path).unwrap();
        assert!(j.reserve_signature(1, 0, 1, b"conflict").is_err());
        drop(j);
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn quota_refusal_keeps_signature_reservations_and_committed_data() {
        let path = directory();
        let j = Journal::open_with_limits(
            &path,
            100,
            StorageLimits {
                max_bytes: 9000,
                reserve_bytes: 128,
            },
        )
        .unwrap();
        j.write_once("signed-00000000000000000001-0000000000-1.bin", &[7; 5000])
            .unwrap();
        let error = j
            .write_once("signed-00000000000000000002-0000000000-1.bin", &[8; 5000])
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::StorageFull);
        j.write_once("signed-00000000000000000001-0000000000-1.bin", &[7; 5000])
            .unwrap();
        assert!(!path
            .join("signed-00000000000000000002-0000000000-1.bin")
            .exists());
        assert_eq!(
            fs::read(path.join("signed-00000000000000000001-0000000000-1.bin")).unwrap(),
            vec![7; 5000]
        );
        drop(j);
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn failed_reconciliation_and_live_wal_sampling_cannot_leave_an_under_count() {
        let path = directory();
        let j = Journal::open_with_limits(
            &path,
            100,
            StorageLimits {
                max_bytes: 20000,
                reserve_bytes: 0,
            },
        )
        .unwrap();
        j.write_once("retained.bin", b"a").unwrap();
        fs::create_dir(path.join("wal")).unwrap();
        fs::write(path.join("wal/consensus.wal"), [0; 16000]).unwrap();
        let audits = j.accounting_audits().unwrap();
        assert!(j.write_once("refused.bin", b"b").is_err());
        assert_eq!(j.accounting_audits().unwrap(), audits);
        fs::write(path.join("external.bin"), [0; 20000]).unwrap();
        assert!(j.ensure_capacity(0).is_err());
        assert!(j.write_once("refused.bin", b"b").is_err());
        assert!(!path.join("refused.bin").exists());
        j.write_once("retained.bin", b"a").unwrap();
        drop(j);
        let j = Journal::open(&path).unwrap();
        assert_eq!(fs::read(path.join("retained.bin")).unwrap(), b"a");
        drop(j);
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn replacement_interruptions_keep_old_or_complete_new_base() {
        for stage in [
            WriteStage::FileSync,
            WriteStage::Renamed,
            WriteStage::DirectorySync,
        ] {
            let path = directory();
            let j = Journal::open(&path).unwrap();
            j.install_compact_base(b"old authenticated bytes").unwrap();
            assert!(j
                .install_compact_base_inner(b"new authenticated bytes", |at| if at == stage {
                    Err(io::Error::new(
                        io::ErrorKind::StorageFull,
                        "injected I/O interruption",
                    ))
                } else {
                    Ok(())
                })
                .is_err());
            let expected = if stage == WriteStage::FileSync {
                b"old authenticated bytes"
            } else {
                b"new authenticated bytes"
            };
            assert_eq!(fs::read(path.join("compact-base.bin")).unwrap(), expected);
            assert!(!path.join(".pending-compact-base.bin").exists());
            drop(j);
            fs::remove_dir_all(path).unwrap();
        }
    }
    #[test]
    fn exports_never_overwrite_and_truncated_pending_is_not_published() {
        let path = directory();
        fs::create_dir(&path).unwrap();
        let output = path.join("checkpoint.bin");
        publish_checkpoint_export(&output, b"complete").unwrap();
        assert!(publish_checkpoint_export(&output, b"replacement").is_err());
        assert_eq!(fs::read(&output).unwrap(), b"complete");
        let next = path.join("next.bin");
        fs::write(path.join(".pending-export-next.bin"), b"partial").unwrap();
        assert!(publish_checkpoint_export(&next, b"complete").is_err());
        assert!(!next.exists());
        fs::remove_dir_all(path).unwrap();
    }
}
