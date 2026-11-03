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
impl Journal {
    /// Includes retained history, safety files, WAL and pending files. Reserve
    /// room for the largest atomic checkpoint replacement and in-flight WAL I/O.
    pub fn ensure_capacity(&self, additional: u64) -> io::Result<()> {
        let footprint = self.storage_footprint(DEFAULT_ENTRY_BUDGET)?;
        // A new atomic write needs one temporary entry. Preserve room for a
        // receipt/marker too; never allow a successful write to prevent reopen.
        if footprint.entries.saturating_add(2) > DEFAULT_ENTRY_BUDGET {
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
            let block = stats.f_frsize.max(4096);
            let allocation = additional.div_ceil(block).saturating_mul(block);
            // Tiny immutable safety files can consume a full block each. Logical
            // length alone substantially undercounts their SSD requirements.
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
    /// Atomic replacement of the ONE active compact base only. Caller has
    /// authenticated the replacement; fsync must succeed before any deletion.
    pub fn install_compact_base(&self, bytes: &[u8]) -> io::Result<()> {
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
        self.ensure_capacity(bytes.len() as u64)?;
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
        if bytes.len() != 104 {
            return Err(invalid("compact marker size"));
        }
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| invalid("poisoned store lock"))?;
        self.ensure_capacity(104)?;
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
        j.reserve_signature(1, 0, 1, b"a").unwrap();
        j.reserve_signature(2, 0, 1, b"b").unwrap();
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
            j.reserve_signature(3, 0, 1, b"c").unwrap_err().kind(),
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
        j.reserve_signature(1, 0, 1, &[7; 5000]).unwrap();
        let error = j.reserve_signature(2, 0, 1, &[8; 5000]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::StorageFull);
        j.reserve_signature(1, 0, 1, &[7; 5000]).unwrap();
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
