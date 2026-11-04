//! Opt-in immutable history segments. Hashes detect local damage; certificates
//! and execution remain the caller's authentication boundary.
use super::*;
use std::io::{Seek, SeekFrom};

pub const ARCHIVE_SEGMENT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_HEIGHT: u64 = 1_000_000;
const FENCE: &[u8] = b"YC-SEGMENTED-ARCHIVE-1";
const MANIFEST: &str = "archive-manifest.bin";
const ACTIVE: &str = "archive-active.bin";
const TIP: &str = "archive-tip.bin";
const INTENT: &str = "archive-intent.bin";
const FRAME_OVERHEAD: usize = 44;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveStage {
    Manifest,
    Segment,
    Tip,
    Active,
    Fence,
    Intent,
    Cleanup,
}
#[derive(Clone, Copy, Debug)]
pub struct ArchiveAnchor {
    pub height: u64,
    pub root: Digest,
}
#[derive(Clone, Debug)]
pub struct ArchiveStats {
    pub height: u64,
    pub root: Digest,
    pub segments: u64,
    pub migrated_height: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Head {
    height: u64,
    segment: u64,
    end: u64,
    root: Digest,
}
impl Default for Head {
    fn default() -> Self {
        Self {
            height: 0,
            segment: 0,
            end: 0,
            root: Digest([0; 32]),
        }
    }
}
impl Head {
    fn bytes(self) -> Vec<u8> {
        let mut w = Writer::default();
        w.raw(b"YC-ARCHIVE-TIP-1");
        w.u64(self.height);
        w.u64(self.segment);
        w.u64(self.end);
        w.raw(&self.root.0);
        checked(w.0)
    }
    fn decode(bytes: &[u8]) -> io::Result<Self> {
        let body = unchecked(bytes)?;
        let mut r = Reader::new(body, 256).map_err(invalid)?;
        if r.take(16).map_err(invalid)? != b"YC-ARCHIVE-TIP-1" {
            return Err(invalid("archive tip domain"));
        }
        let h = Self {
            height: r.u64().map_err(invalid)?,
            segment: r.u64().map_err(invalid)?,
            end: r.u64().map_err(invalid)?,
            root: Digest(r.array().map_err(invalid)?),
        };
        r.end().map_err(invalid)?;
        if h.height > MAX_HEIGHT
            || h.end > ARCHIVE_SEGMENT_BYTES
            || (h.height == 0 && h != Self::default())
            || (h.height > 0 && (h.segment >= h.height || h.end == 0))
        {
            return Err(invalid("archive tip bounds"));
        }
        Ok(h)
    }
    fn next(self, frame: &[u8]) -> io::Result<Self> {
        let height = self
            .height
            .checked_add(1)
            .filter(|h| *h <= MAX_HEIGHT)
            .ok_or_else(|| invalid("archive height work limit"))?;
        if frame.len() as u64 > ARCHIVE_SEGMENT_BYTES {
            return Err(invalid("archive frame cap"));
        }
        let mut segment = self.segment;
        let mut end = self.end;
        if end + frame.len() as u64 > ARCHIVE_SEGMENT_BYTES {
            segment += 1;
            end = 0;
        }
        let mut bytes = self.root.0.to_vec();
        bytes.extend_from_slice(frame);
        Ok(Self {
            height,
            segment,
            end: end + frame.len() as u64,
            root: yami_chain::hash(b"YC-ARCHIVE-CHAIN-1", &bytes),
        })
    }
}
#[derive(Clone)]
struct Position {
    segment: u64,
    offset: u64,
    length: usize,
    digest: Digest,
    root: Digest,
}
pub(super) struct ArchiveState {
    head: Head,
    index: Vec<Position>,
    lengths: Vec<u64>,
    migrated: Head,
    manifest: Vec<u8>,
    genesis: Vec<u8>,
}
fn checked(mut bytes: Vec<u8>) -> Vec<u8> {
    let hash = yami_chain::hash(b"YC-ARCHIVE-METADATA-1", &bytes);
    bytes.extend_from_slice(&hash.0);
    bytes
}
fn unchecked(bytes: &[u8]) -> io::Result<&[u8]> {
    let n = bytes
        .len()
        .checked_sub(32)
        .ok_or_else(|| invalid("archive metadata truncated"))?;
    if yami_chain::hash(b"YC-ARCHIVE-METADATA-1", &bytes[..n]).0 != bytes[n..] {
        return Err(invalid("archive metadata checksum"));
    }
    Ok(&bytes[..n])
}
fn segment_name(n: u64) -> String {
    format!("archive-segment-{n:020}.bin")
}
fn frame(record: &FinalizedRecord) -> io::Result<Vec<u8>> {
    let bytes = record.canonical()?;
    let mut w = Writer::default();
    w.u64(record.block.header.height);
    w.bytes(&bytes).map_err(invalid)?;
    w.raw(&yami_chain::hash(b"YC-DURABLE-RECORD-1", &bytes).0);
    Ok(w.0)
}
fn decode_frame(bytes: &[u8], height: u64) -> io::Result<FinalizedRecord> {
    let mut r = Reader::new(bytes, MAX_RECORD_BYTES + FRAME_OVERHEAD).map_err(invalid)?;
    if r.u64().map_err(invalid)? != height {
        return Err(invalid("archive frame height"));
    }
    let record = r.bytes(MAX_RECORD_BYTES).map_err(invalid)?;
    if r.array::<32>().map_err(invalid)? != yami_chain::hash(b"YC-DURABLE-RECORD-1", record).0 {
        return Err(invalid("archive receipt mismatch"));
    }
    r.end().map_err(invalid)?;
    let record = FinalizedRecord::decode(record)?;
    if record.block.header.height != height {
        return Err(invalid("archive record height"));
    }
    Ok(record)
}
impl Journal {
    /// Compare the canonical genesis through the local storage-version fence.
    /// The fence changes neither genesis encoding on the wire nor its hash.
    pub fn ensure_genesis(&self, genesis: &Genesis) -> io::Result<()> {
        let bytes = genesis.canonical();
        if self.path.join(MANIFEST).exists() {
            let actual = read_bounded(
                &self.path.join("genesis.bin"),
                yami_chain::committee::MAX_GENESIS_BYTES + FENCE.len(),
            )?;
            if actual != bytes && actual != [FENCE, bytes.as_slice()].concat() {
                return Err(invalid("archive genesis mismatch"));
            }
            Ok(())
        } else {
            self.write_once("genesis.bin", &bytes)
        }
    }
    pub fn verify_genesis(&self, genesis: &Genesis) -> io::Result<()> {
        let actual = read_bounded(
            &self.path.join("genesis.bin"),
            yami_chain::committee::MAX_GENESIS_BYTES + FENCE.len(),
        )?;
        let canonical = genesis.canonical();
        let expected = if self.is_segmented_archive() && actual.starts_with(FENCE) {
            [FENCE, canonical.as_slice()].concat()
        } else {
            canonical
        };
        if actual != expected {
            return Err(invalid("genesis mismatch"));
        }
        Ok(())
    }
    /// The anchor must be retained independently of the directory being restored.
    /// A complete internally consistent local rollback needs this external trust.
    pub fn verify_archive_anchor(&self, anchor: ArchiveAnchor) -> io::Result<()> {
        let a = self.archive.lock().map_err(|_| invalid("archive lock"))?;
        let a = a
            .as_ref()
            .ok_or_else(|| invalid("archive anchor requires segmented history"))?;
        self.check_archive_controls(a)?;
        let root = if anchor.height == 0 {
            Digest([0; 32])
        } else {
            a.index
                .get(usize::try_from(anchor.height - 1).map_err(invalid)?)
                .ok_or_else(|| invalid("archive below independent anchor"))?
                .root
        };
        if root != anchor.root {
            return Err(invalid("archive independent anchor conflict"));
        }
        Ok(())
    }
    pub fn archive_stats(&self) -> io::Result<Option<ArchiveStats>> {
        let a = self
            .archive
            .lock()
            .map_err(|_| invalid("archive lock poisoned"))?;
        Ok(a.as_ref().map(|a| ArchiveStats {
            height: a.head.height,
            root: a.head.root,
            segments: if a.head.height == 0 {
                0
            } else {
                a.head.segment + 1
            },
            migrated_height: a.migrated.height,
        }))
    }
    pub fn is_segmented_archive(&self) -> bool {
        fs::symlink_metadata(self.path.join(MANIFEST)).is_ok()
            || self.archive.lock().map_or(true, |a| a.is_some())
    }
    fn archive_file(&self, name: &str, bytes: &[u8], replace: bool) -> io::Result<()> {
        let path = self.path.join(name);
        if !replace && path.try_exists()? {
            if read_bounded(&path, yami_chain::committee::MAX_GENESIS_BYTES + 1024)? != bytes {
                return Err(invalid("conflicting archive metadata"));
            }
            File::open(path)?.sync_all()?;
            return self.sync_dir();
        }
        if fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file()) {
            return Err(invalid("archive metadata type"));
        }
        self.reserve_capacity(bytes.len() as u64, 1, 1)?;
        let temp = self.path.join(format!(".pending-{name}"));
        let result = (|| {
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            f.write_all(bytes)?;
            f.sync_all()?;
            fs::rename(&temp, path)?;
            self.sync_dir()
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
    // Complete inventory of both namespaces, including the presence/absence of
    // each legacy receipt. No migration accepts a missing/gapped source record.
    fn archive_inventory(&self) -> io::Result<(Head, Digest)> {
        let mut records = 0;
        let mut receipts = 0;
        for (i, e) in fs::read_dir(&self.path)?.enumerate() {
            if i as u64 >= DEFAULT_ENTRY_BUDGET {
                return Err(invalid("archive inventory budget"));
            }
            let e = e?;
            let n = e.file_name();
            let n = n.to_string_lossy();
            for (prefix, map) in [("finalized", &mut records), ("receipt", &mut receipts)] {
                if let Some(s) = n.strip_prefix(&format!("{prefix}-")) {
                    let h = s
                        .strip_suffix(".bin")
                        .ok_or_else(|| invalid("archive legacy filename"))?
                        .parse::<u64>()
                        .map_err(invalid)?;
                    if h == 0 || h > MAX_HEIGHT || n != format!("{prefix}-{h:020}.bin") {
                        return Err(invalid("archive legacy height"));
                    }
                    *map = (*map).max(h);
                }
            }
        }
        let mut head = Head::default();
        let mut inventory = Digest([0; 32]);
        for h in 1..=records {
            let bytes = read_bounded(
                &self.path.join(format!("finalized-{h:020}.bin")),
                MAX_RECORD_BYTES,
            )?;
            let record = FinalizedRecord::decode(&bytes)?;
            if record.canonical()? != bytes || record.block.header.height != h {
                return Err(invalid("archive noncanonical source"));
            }
            let f = frame(&record)?;
            head = head.next(&f)?;
            let mut input = inventory.0.to_vec();
            input.extend_from_slice(&f);
            match read_bounded(&self.path.join(format!("receipt-{h:020}.bin")), 32) {
                Ok(b) => {
                    if b != yami_chain::hash(b"YC-DURABLE-RECORD-1", &bytes).0 {
                        return Err(invalid("archive migration receipt"));
                    }
                    input.push(1);
                    input.extend_from_slice(&b);
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => input.push(0),
                Err(e) => return Err(e),
            }
            inventory = yami_chain::hash(b"YC-ARCHIVE-INVENTORY-1", &input);
        }
        if receipts > records {
            return Err(invalid("archive orphan receipt"));
        }
        Ok((head, inventory))
    }
    fn manifest(&self) -> io::Result<(Head, Digest, Vec<u8>, Vec<u8>)> {
        let bytes = read_bounded(
            &self.path.join(MANIFEST),
            yami_chain::committee::MAX_GENESIS_BYTES + 1024,
        )?;
        let body = unchecked(&bytes)?;
        let mut r = Reader::new(body, body.len()).map_err(invalid)?;
        if r.take(FENCE.len()).map_err(invalid)? != FENCE {
            return Err(invalid("archive manifest domain"));
        }
        let head = Head::decode(r.bytes(256).map_err(invalid)?)?;
        let inventory = Digest(r.array().map_err(invalid)?);
        let genesis = r
            .bytes(yami_chain::committee::MAX_GENESIS_BYTES)
            .map_err(invalid)?
            .to_vec();
        r.end().map_err(invalid)?;
        Ok((head, inventory, genesis, bytes))
    }
    /// Caller supplies complete certificate authentication. Full replay and source
    /// inventory checks finish before a manifest or version fence is published.
    pub fn migrate_archive(
        &self,
        genesis: Genesis,
        verify: impl Fn(&FinalizedRecord) -> io::Result<()>,
    ) -> io::Result<ArchiveStats> {
        self.migrate_archive_observed(genesis, verify, |_| {})
    }
    pub fn migrate_archive_observed(
        &self,
        genesis: Genesis,
        verify: impl Fn(&FinalizedRecord) -> io::Result<()>,
        mut observe: impl FnMut(ArchiveStage),
    ) -> io::Result<ArchiveStats> {
        if self.path.join("compact-base.bin").exists() || self.path.join("compacted.bin").exists() {
            return Err(invalid("cannot segment a pruned store"));
        }
        if let Some(s) = self.archive_stats()? {
            self.replay(genesis, verify)?;
            return Ok(s);
        }
        let state = self.replay(genesis.clone(), verify)?;
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| invalid("archive write lock"))?;
        let (expected, inventory) = self.archive_inventory()?;
        if expected.height != state.height() {
            return Err(invalid("archive replay inventory mismatch"));
        }
        let mut w = Writer::default();
        w.raw(FENCE);
        w.bytes(&expected.bytes()).map_err(invalid)?;
        w.raw(&inventory.0);
        w.bytes(&genesis.canonical()).map_err(invalid)?;
        self.archive_file(MANIFEST, &checked(w.0), false)?;
        observe(ArchiveStage::Manifest);
        // Construct and fsync each deterministic segment once. Resume only an
        // exact prefix, never overwrite conflicting staging or unknown tails.
        let mut head = Head::default();
        let mut segment = Vec::new();
        for h in 1..=expected.height {
            let record = FinalizedRecord::decode(&read_bounded(
                &self.path.join(format!("finalized-{h:020}.bin")),
                MAX_RECORD_BYTES,
            )?)?;
            let f = frame(&record)?;
            let next = head.next(&f)?;
            if next.segment != head.segment {
                self.archive_stage_segment(head.segment, &segment)?;
                observe(ArchiveStage::Segment);
                segment.clear();
            }
            segment.extend_from_slice(&f);
            head = next;
        }
        if !segment.is_empty() {
            self.archive_stage_segment(head.segment, &segment)?;
            observe(ArchiveStage::Segment);
        }
        if head != expected || self.archive_inventory()? != (expected, inventory) {
            return Err(invalid("archive migration changed source"));
        }
        self.archive_file(TIP, &head.bytes(), false)?;
        observe(ArchiveStage::Tip);
        self.archive_file(ACTIVE, &head.bytes(), false)?;
        observe(ArchiveStage::Active);
        self.load_archive(false)?;
        self.archive_file(
            "genesis.bin",
            &[FENCE, genesis.canonical().as_slice()].concat(),
            true,
        )?;
        observe(ArchiveStage::Fence);
        let a = self.load_archive(true)?;
        *self.archive.lock().map_err(|_| invalid("archive lock"))? = Some(a);
        self.archive_stats()?
            .ok_or_else(|| invalid("archive activation failed"))
    }
    fn archive_stage_segment(&self, number: u64, expected: &[u8]) -> io::Result<()> {
        let path = self.path.join(segment_name(number));
        if fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file()) {
            return Err(invalid("archive staging type"));
        }
        self.reserve_capacity(expected.len() as u64, 1, 1)?;
        let mut f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let len = f.metadata()?.len();
        if len > expected.len() as u64 {
            return Err(invalid("archive staging unknown tail"));
        }
        let mut prefix = vec![0; len as usize];
        f.read_exact(&mut prefix)?;
        if prefix != expected[..prefix.len()] {
            return Err(invalid("archive staging conflicting prefix"));
        }
        f.write_all(&expected[prefix.len()..])?;
        f.sync_all()?;
        self.sync_dir()
    }
    fn archive_append_bytes(&self, old: Head, next: Head, bytes: &[u8]) -> io::Result<()> {
        let offset = next.end - bytes.len() as u64;
        if next.height != old.height + 1 {
            return Err(invalid("archive append sequence"));
        }
        let p = self.path.join(segment_name(next.segment));
        if fs::symlink_metadata(&p).is_ok_and(|m| !m.is_file()) {
            return Err(invalid("archive segment type"));
        }
        self.reserve_capacity(bytes.len() as u64, 1, 1)?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(p)?;
        let len = file.metadata()?.len();
        if len < offset || len > next.end {
            return Err(invalid("archive unexpected tail"));
        }
        file.seek(SeekFrom::Start(offset))?;
        let mut existing = vec![0; ((len - offset) as usize).min(bytes.len())];
        file.read_exact(&mut existing)?;
        if existing != bytes[..existing.len()] {
            return Err(invalid("archive conflicting prefix"));
        }
        file.write_all(&bytes[existing.len()..])?;
        file.sync_all()?;
        self.sync_dir()
    }
    fn load_archive(&self, require_fence: bool) -> io::Result<ArchiveState> {
        let (migrated, inventory, genesis, manifest) = self.manifest()?;
        if self.archive_inventory()? != (migrated, inventory) {
            return Err(invalid("archive retained source changed"));
        }
        if require_fence
            && read_bounded(&self.path.join("genesis.bin"), genesis.len() + FENCE.len())?
                != [FENCE, genesis.as_slice()].concat()
        {
            return Err(invalid("archive downgrade fence missing"));
        }
        if Head::decode(&read_bounded(&self.path.join(ACTIVE), 256)?)? != migrated {
            return Err(invalid("archive activation mismatch"));
        }
        let target = Head::decode(&read_bounded(&self.path.join(TIP), 256)?)?;
        if target.height < migrated.height {
            return Err(invalid("archive tip rollback"));
        }
        let mut head = Head::default();
        let mut index = Vec::new();
        let mut lengths = Vec::new();
        let mut observed_migration = head;
        for n in 0..=target.segment {
            if target.height == 0 {
                break;
            }
            let p = self.path.join(segment_name(n));
            if !fs::symlink_metadata(&p)?.is_file() {
                return Err(invalid("archive segment type"));
            }
            let mut f = File::open(p)?;
            let len = f.metadata()?.len();
            lengths.push(len);
            if len == 0 || len > ARCHIVE_SEGMENT_BYTES {
                return Err(invalid("archive segment length"));
            }
            let mut offset = 0;
            while offset < len {
                let mut hdr = [0; 12];
                f.read_exact(&mut hdr)?;
                let size = u32::from_be_bytes(hdr[8..12].try_into().map_err(invalid)?) as usize;
                if size > MAX_RECORD_BYTES || size == 0 {
                    return Err(invalid("archive frame length"));
                }
                let mut bytes = vec![0; size + FRAME_OVERHEAD];
                bytes[..12].copy_from_slice(&hdr);
                f.read_exact(&mut bytes[12..])?;
                decode_frame(&bytes, head.height + 1)?;
                let next = head.next(&bytes)?;
                if next.segment != n || next.end != offset + bytes.len() as u64 {
                    return Err(invalid("archive segment boundary"));
                }
                index.push(Position {
                    segment: n,
                    offset,
                    length: bytes.len(),
                    digest: yami_chain::hash(b"YC-ARCHIVE-FRAME-1", &bytes),
                    root: next.root,
                });
                head = next;
                offset = head.end;
                if head.height == migrated.height {
                    observed_migration = head;
                }
            }
        }
        if head != target || observed_migration != migrated {
            return Err(invalid("archive tip/prefix mismatch"));
        }
        // Reject extra, noncanonical or missing segments and unknown namespace
        // files rather than treating them as unacknowledged history to delete.
        for e in fs::read_dir(&self.path)? {
            let e = e?;
            let name = e.file_name();
            let name = name.to_string_lossy();
            if let Some(s) = name.strip_prefix("archive-segment-") {
                let n = s
                    .strip_suffix(".bin")
                    .ok_or_else(|| invalid("archive segment name"))?
                    .parse::<u64>()
                    .map_err(invalid)?;
                if name != segment_name(n) || target.height == 0 || n > target.segment {
                    return Err(invalid("archive extra segment"));
                }
            } else if name.starts_with("archive-")
                && ![MANIFEST, ACTIVE, TIP, INTENT].contains(&name.as_ref())
            {
                return Err(invalid("archive unknown entry"));
            }
        }
        Ok(ArchiveState {
            head,
            index,
            lengths,
            migrated,
            manifest,
            genesis,
        })
    }
    pub(super) fn recover_archive(&self) -> io::Result<()> {
        if !self.path.join(MANIFEST).try_exists()? {
            for e in fs::read_dir(&self.path)? {
                if e?.file_name().to_string_lossy().starts_with("archive-") {
                    return Err(invalid("archive manifest missing"));
                }
            }
            // A lost manifest must not make a versioned genesis a legacy store.
            if self.path.join("genesis.bin").exists()
                && read_bounded(
                    &self.path.join("genesis.bin"),
                    yami_chain::committee::MAX_GENESIS_BYTES + FENCE.len(),
                )?
                .starts_with(FENCE)
            {
                return Err(invalid("archive fence without manifest"));
            }
            return Ok(());
        }
        let (migration, _, _, _) = self.manifest()?;
        let genesis = read_bounded(
            &self.path.join("genesis.bin"),
            yami_chain::committee::MAX_GENESIS_BYTES + FENCE.len(),
        )?;
        if !genesis.starts_with(FENCE) {
            if self.path.join(INTENT).exists() {
                return Err(invalid("archive intent before activation"));
            }
            if self.path.join(TIP).exists()
                && Head::decode(&read_bounded(&self.path.join(TIP), 256)?)? != migration
            {
                return Err(invalid("archive fence rollback after activation"));
            }
            return Ok(()); // interrupted migration: authenticated command resumes
        }
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| invalid("archive write lock"))?;
        self.recover_archive_intent()?;
        let a = self.load_archive(true)?;
        *self.archive.lock().map_err(|_| invalid("archive lock"))? = Some(a);
        Ok(())
    }
    fn recover_archive_intent(&self) -> io::Result<()> {
        let bytes = match read_bounded(&self.path.join(INTENT), MAX_RECORD_BYTES + 1024) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        let body = unchecked(&bytes)?;
        let mut r = Reader::new(body, body.len()).map_err(invalid)?;
        let old = Head::decode(r.bytes(256).map_err(invalid)?)?;
        let next = Head::decode(r.bytes(256).map_err(invalid)?)?;
        let f = r
            .bytes(MAX_RECORD_BYTES + FRAME_OVERHEAD)
            .map_err(invalid)?;
        r.end().map_err(invalid)?;
        if old.next(f)? != next {
            return Err(invalid("archive intent head"));
        }
        decode_frame(f, next.height)?;
        let current = Head::decode(&read_bounded(&self.path.join(TIP), 256)?)?;
        if current != old && current != next {
            return Err(invalid("archive intent predecessor"));
        }
        self.archive_append_bytes(old, next, f)?;
        self.archive_file(TIP, &next.bytes(), true)?;
        // Verify full stream before removing the only recovery intent.
        self.load_archive(true)?;
        fs::remove_file(self.path.join(INTENT))?;
        self.sync_dir()
    }
    fn check_archive_controls(&self, a: &ArchiveState) -> io::Result<()> {
        if a.head.height > 0 {
            let metadata = fs::symlink_metadata(self.path.join(segment_name(a.head.segment)))?;
            if !metadata.is_file() || metadata.len() != a.head.end {
                return Err(invalid("archive head segment changed"));
            }
        } else if self.path.join(segment_name(0)).try_exists()? {
            return Err(invalid("archive unexpected initial segment"));
        }
        if read_bounded(&self.path.join(TIP), 256)? != a.head.bytes()
            || read_bounded(&self.path.join(ACTIVE), 256)? != a.migrated.bytes()
            || read_bounded(&self.path.join(MANIFEST), a.manifest.len())? != a.manifest
            || read_bounded(
                &self.path.join("genesis.bin"),
                a.genesis.len() + FENCE.len(),
            )? != [FENCE, a.genesis.as_slice()].concat()
            || self.path.join(INTENT).exists()
        {
            return Err(invalid("archive controls changed; reopen for recovery"));
        }
        Ok(())
    }
    pub(super) fn archive_record(
        &self,
        height: u64,
    ) -> io::Result<Option<Option<FinalizedRecord>>> {
        let a = self.archive.lock().map_err(|_| invalid("archive lock"))?;
        let Some(a) = a.as_ref() else { return Ok(None) };
        self.check_archive_controls(a)?;
        let Some(pos) = height
            .checked_sub(1)
            .and_then(|h| usize::try_from(h).ok())
            .and_then(|h| a.index.get(h))
        else {
            return Ok(Some(None));
        };
        let p = self.path.join(segment_name(pos.segment));
        let metadata = fs::symlink_metadata(&p)?;
        if !metadata.is_file() || metadata.len() != a.lengths[pos.segment as usize] {
            return Err(invalid("archive indexed segment changed"));
        }
        let mut file = File::open(p)?;
        file.seek(SeekFrom::Start(pos.offset))?;
        let mut bytes = vec![0; pos.length];
        file.read_exact(&mut bytes)?;
        if yami_chain::hash(b"YC-ARCHIVE-FRAME-1", &bytes) != pos.digest {
            return Err(invalid("archive indexed record changed"));
        }
        Ok(Some(Some(decode_frame(&bytes, height)?)))
    }
    pub(super) fn append_archive(&self, record: &FinalizedRecord) -> io::Result<()> {
        self.append_archive_observed(record, |_| {})
    }
    pub fn append_archive_observed(
        &self,
        record: &FinalizedRecord,
        mut observe: impl FnMut(ArchiveStage),
    ) -> io::Result<()> {
        let f = frame(record)?;
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| invalid("archive write lock"))?;
        let mut a = self.archive.lock().map_err(|_| invalid("archive lock"))?;
        let a = a.as_mut().ok_or_else(|| {
            invalid("archive migration incomplete; resume authenticated migration")
        })?;
        self.check_archive_controls(a)?;
        let height = record.block.header.height;
        if height <= a.head.height {
            let pos = height
                .checked_sub(1)
                .and_then(|h| a.index.get(h as usize))
                .ok_or_else(|| invalid("archive height"))?;
            if yami_chain::hash(b"YC-ARCHIVE-FRAME-1", &f) != pos.digest {
                return Err(invalid("conflicting archive record"));
            }
            let mut file = File::open(self.path.join(segment_name(pos.segment)))?;
            file.seek(SeekFrom::Start(pos.offset))?;
            let mut actual = vec![0; pos.length];
            file.read_exact(&mut actual)?;
            if actual != f {
                return Err(invalid("archive retry corruption"));
            }
            file.sync_all()?;
            return self.sync_dir();
        }
        if height != a.head.height + 1 {
            return Err(invalid("archive append gap"));
        }
        let next = a.head.next(&f)?;
        self.reserve_capacity((f.len() * 2 + 1024) as u64, 4, 4)?;
        let mut w = Writer::default();
        w.bytes(&a.head.bytes()).map_err(invalid)?;
        w.bytes(&next.bytes()).map_err(invalid)?;
        w.bytes(&f).map_err(invalid)?;
        self.archive_file(INTENT, &checked(w.0), false)?;
        observe(ArchiveStage::Intent);
        self.archive_append_bytes(a.head, next, &f)?;
        observe(ArchiveStage::Segment);
        self.archive_file(TIP, &next.bytes(), true)?;
        observe(ArchiveStage::Tip);
        fs::remove_file(self.path.join(INTENT))?;
        self.sync_dir()?;
        observe(ArchiveStage::Cleanup);
        a.index.push(Position {
            segment: next.segment,
            offset: next.end - f.len() as u64,
            length: f.len(),
            digest: yami_chain::hash(b"YC-ARCHIVE-FRAME-1", &f),
            root: next.root,
        });
        if next.segment as usize == a.lengths.len() {
            a.lengths.push(next.end);
        } else {
            a.lengths[next.segment as usize] = next.end;
        }
        a.head = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static ID: AtomicU64 = AtomicU64::new(0);
    fn dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "yami-archive-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ))
    }
    fn records(n: u64, certificate: usize) -> Vec<FinalizedRecord> {
        let mut s = State::new(Genesis::local()).unwrap();
        (0..n)
            .map(|_| {
                let block = s.development_block().unwrap();
                s = s.execute(&block).unwrap();
                FinalizedRecord {
                    block,
                    certificate: vec![7; certificate],
                }
            })
            .collect()
    }
    fn legacy(j: &Journal, r: &[FinalizedRecord]) {
        for r in r {
            j.append(r).unwrap();
        }
    }
    fn source(path: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        fs::read_dir(path)
            .unwrap()
            .map(Result::unwrap)
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                (n.starts_with("finalized-")
                    || n.starts_with("receipt-")
                    || n.starts_with("signing-")
                    || n.starts_with("equivocation-"))
                .then(|| (n, fs::read(e.path()).unwrap()))
            })
            .collect()
    }
    #[test]
    fn roundtrip_retains_every_source_byte_and_indexes_new_history() {
        let p = dir();
        let r = records(12, 1);
        let j = Journal::open(&p).unwrap();
        legacy(&j, &r[..5]);
        j.reserve_signature(9, 0, 2, b"irrevocable").unwrap();
        j.write_once("equivocation-fixture.bin", b"evidence")
            .unwrap();
        let before = source(&p);
        let s = j.migrate_archive(Genesis::local(), |_| Ok(())).unwrap();
        assert_eq!(s.height, 5);
        assert_eq!(before, source(&p));
        assert!(j.write_once("archive-evil.bin", b"evil").is_err());
        assert!(j
            .write_once("finalized-00000000000000000006.bin", b"evil")
            .is_err());
        for r in &r[5..] {
            j.append(r).unwrap();
            j.append(r).unwrap();
        }
        for r in &r {
            assert_eq!(
                j.record(r.block.header.height)
                    .unwrap()
                    .unwrap()
                    .canonical()
                    .unwrap(),
                r.canonical().unwrap()
            );
        }
        assert!(j.record(0).unwrap().is_none());
        assert!(j.record(13).unwrap().is_none());
        let state = j.replay(Genesis::local(), |_| Ok(())).unwrap();
        assert_eq!(state.height(), 12);
        assert!(j.install_compact_base(b"cannot prune").is_err());
        drop(j);
        let j = Journal::open(&p).unwrap();
        assert_eq!(j.replay(Genesis::local(), |_| Ok(())).unwrap(), state);
        assert!(j.reserve_signature(9, 0, 2, b"conflict").is_err());
        assert_eq!(before, source(&p));
        assert!(fs::read(p.join("genesis.bin")).unwrap().starts_with(FENCE));
        drop(j);
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn external_anchor_detects_complete_consistent_rollback_and_wrong_history() {
        let p = dir();
        let j = Journal::open(&p).unwrap();
        let r = records(4, 0);
        legacy(&j, &r[..3]);
        j.migrate_archive(Genesis::local(), |_| Ok(())).unwrap();
        let old_tip = fs::read(p.join(TIP)).unwrap();
        let old_segment = fs::read(p.join(segment_name(0))).unwrap();
        j.append(&r[3]).unwrap();
        let s = j.archive_stats().unwrap().unwrap();
        let anchor = ArchiveAnchor {
            height: s.height,
            root: s.root,
        };
        j.verify_archive_anchor(anchor).unwrap();
        assert!(j
            .verify_archive_anchor(ArchiveAnchor {
                height: 3,
                root: s.root
            })
            .is_err());
        drop(j);
        fs::write(p.join(TIP), old_tip).unwrap();
        fs::write(p.join(segment_name(0)), old_segment).unwrap();
        let j = Journal::open(&p).unwrap();
        assert_eq!(j.replay(Genesis::local(), |_| Ok(())).unwrap().height(), 3);
        assert!(j.verify_archive_anchor(anchor).is_err());
        drop(j);
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn migration_authenticates_before_publication_and_rejects_incomplete_sources() {
        for case in 0..4 {
            let p = dir();
            let r = records(3, 0);
            let j = Journal::open(&p).unwrap();
            legacy(&j, &r);
            match case {
                0 => {}
                1 => {
                    fs::remove_file(p.join("finalized-00000000000000000002.bin")).unwrap();
                }
                2 => {
                    fs::write(p.join("receipt-00000000000000000003.bin"), [0; 32]).unwrap();
                }
                _ => {
                    fs::write(p.join("finalized-4.bin"), r[2].canonical().unwrap()).unwrap();
                }
            }
            assert!(j
                .migrate_archive(Genesis::local(), |_| if case == 0 {
                    Err(invalid("hostile certificate"))
                } else {
                    Ok(())
                })
                .is_err());
            assert!(!p.join(MANIFEST).exists());
            drop(j);
            fs::remove_dir_all(p).unwrap();
        }
    }
    #[test]
    fn crash_child() {
        let Some(p) = std::env::var_os("YAMI_ARCHIVE_CHILD") else {
            return;
        };
        let stage = std::env::var("YAMI_ARCHIVE_STAGE").unwrap();
        let mode = std::env::var("YAMI_ARCHIVE_MODE").unwrap();
        let j = Journal::open(Path::new(&p)).unwrap();
        let crash = |at: ArchiveStage| {
            if format!("{at:?}") == stage {
                std::process::exit(86);
            }
        };
        if mode == "migration" {
            j.migrate_archive_observed(Genesis::local(), |_| Ok(()), crash)
                .unwrap();
        } else {
            j.append_archive_observed(&records(4, 0)[3], crash).unwrap();
        }
        panic!("boundary not reached");
    }
    fn crash(p: &Path, mode: &str, stage: &str) {
        let s = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "archive::tests::crash_child", "--nocapture"])
            .env("YAMI_ARCHIVE_CHILD", p)
            .env("YAMI_ARCHIVE_STAGE", stage)
            .env("YAMI_ARCHIVE_MODE", mode)
            .status()
            .unwrap();
        assert_eq!(s.code(), Some(86));
    }
    #[test]
    fn every_migration_publication_resumes_without_dropping_history() {
        for stage in ["Manifest", "Segment", "Tip", "Active", "Fence"] {
            let p = dir();
            let j = Journal::open(&p).unwrap();
            legacy(&j, &records(3, 0));
            let before = source(&p);
            drop(j);
            crash(&p, "migration", stage);
            let j = Journal::open(&p).unwrap();
            j.migrate_archive(Genesis::local(), |_| Ok(())).unwrap();
            assert_eq!(j.replay(Genesis::local(), |_| Ok(())).unwrap().height(), 3);
            assert_eq!(source(&p), before);
            drop(j);
            fs::remove_dir_all(p).unwrap();
        }
    }
    #[test]
    fn every_append_boundary_recovers_exactly_once_and_refuses_conflicts() {
        for stage in ["Intent", "Segment", "Tip", "Cleanup"] {
            let p = dir();
            let j = Journal::open(&p).unwrap();
            legacy(&j, &records(3, 0));
            j.migrate_archive(Genesis::local(), |_| Ok(())).unwrap();
            drop(j);
            crash(&p, "append", stage);
            let j = Journal::open(&p).unwrap();
            let r = records(4, 0);
            assert_eq!(j.replay(Genesis::local(), |_| Ok(())).unwrap().height(), 4);
            j.append(&r[3]).unwrap();
            let mut evil = r[3].clone();
            evil.certificate.push(99);
            assert!(j.append(&evil).is_err());
            assert!(!p.join(INTENT).exists());
            drop(j);
            fs::remove_dir_all(p).unwrap();
        }
    }
    #[test]
    fn interrupted_migration_missing_or_changed_source_refuses() {
        for receipt in [false, true] {
            let p = dir();
            let j = Journal::open(&p).unwrap();
            legacy(&j, &records(3, 0));
            drop(j);
            crash(&p, "migration", "Manifest");
            fs::remove_file(p.join(if receipt {
                "receipt-00000000000000000001.bin"
            } else {
                "finalized-00000000000000000003.bin"
            }))
            .unwrap();
            let j = Journal::open(&p).unwrap();
            assert!(j.migrate_archive(Genesis::local(), |_| Ok(())).is_err());
            assert!(j.append(&records(4, 0)[3]).is_err());
            assert!(!fs::read(p.join("genesis.bin")).unwrap().starts_with(FENCE));
            drop(j);
            fs::remove_dir_all(p).unwrap();
        }
    }
    #[test]
    fn corrupt_missing_extra_segments_and_selective_rollback_refuse() {
        for case in 0..8 {
            let p = dir();
            let j = Journal::open(&p).unwrap();
            legacy(&j, &records(3, 0));
            j.migrate_archive(Genesis::local(), |_| Ok(())).unwrap();
            let old_tip = fs::read(p.join(TIP)).unwrap();
            j.append(&records(4, 0)[3]).unwrap();
            drop(j);
            match case {
                0 => {
                    fs::remove_file(p.join(segment_name(0))).unwrap();
                }
                1 => {
                    let mut b = fs::read(p.join(segment_name(0))).unwrap();
                    b[20] ^= 1;
                    fs::write(p.join(segment_name(0)), b).unwrap();
                }
                2 => {
                    fs::write(p.join(segment_name(1)), b"unknown tail").unwrap();
                }
                3 => {
                    fs::write(p.join(TIP), old_tip).unwrap();
                }
                4 => {
                    fs::remove_file(p.join(MANIFEST)).unwrap();
                }
                5 => {
                    fs::remove_file(p.join(ACTIVE)).unwrap();
                }
                6 => {
                    fs::remove_file(p.join("receipt-00000000000000000001.bin")).unwrap();
                }
                _ => {
                    fs::write(p.join("genesis.bin"), Genesis::local().canonical()).unwrap();
                }
            }
            assert!(Journal::open(&p).is_err(), "case {case}");
            fs::remove_dir_all(p).unwrap();
        }
    }
    #[test]
    fn indexed_reads_bind_original_bytes_and_unknown_tails_are_not_truncated() {
        let p = dir();
        let j = Journal::open(&p).unwrap();
        let r = records(3, 0);
        legacy(&j, &r);
        j.migrate_archive(Genesis::local(), |_| Ok(())).unwrap();
        let mut evil = r[0].clone();
        evil.certificate.push(4);
        let mut b = frame(&evil).unwrap();
        let mut old = fs::read(p.join(segment_name(0))).unwrap();
        b.extend_from_slice(&old[frame(&r[0]).unwrap().len()..]);
        fs::write(p.join(segment_name(0)), b).unwrap();
        assert!(j.record(1).is_err());
        old.push(0);
        fs::write(p.join(segment_name(0)), &old).unwrap();
        drop(j);
        assert!(Journal::open(&p).is_err());
        assert_eq!(fs::read(p.join(segment_name(0))).unwrap(), old);
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn staged_partial_prefix_resumes_and_unknown_bytes_never_activate() {
        for corrupt in [false, true] {
            let p = dir();
            let j = Journal::open(&p).unwrap();
            let r = records(3, 0);
            legacy(&j, &r);
            drop(j);
            crash(&p, "migration", "Manifest");
            let mut prefix = frame(&r[0]).unwrap()[..17].to_vec();
            if corrupt {
                prefix[0] ^= 1;
            }
            fs::write(p.join(segment_name(0)), &prefix).unwrap();
            let j = Journal::open(&p).unwrap();
            let result = j.migrate_archive(Genesis::local(), |_| Ok(()));
            if corrupt {
                assert!(result.is_err());
                assert_eq!(fs::read(p.join(segment_name(0))).unwrap(), prefix);
                assert!(!fs::read(p.join("genesis.bin")).unwrap().starts_with(FENCE));
            } else {
                assert_eq!(result.unwrap().height, 3);
            }
            drop(j);
            fs::remove_dir_all(p).unwrap();
        }
    }
    #[test]
    fn exact_partial_intents_resume_but_unexpected_prefixes_stay_untouched() {
        for corrupt in [false, true] {
            let p = dir();
            let j = Journal::open(&p).unwrap();
            legacy(&j, &records(3, 0));
            j.migrate_archive(Genesis::local(), |_| Ok(())).unwrap();
            let original = fs::read(p.join(segment_name(0))).unwrap();
            drop(j);
            crash(&p, "append", "Intent");
            let mut bytes = original.clone();
            let mut suffix = frame(&records(4, 0)[3]).unwrap()[..17].to_vec();
            if corrupt {
                suffix[0] ^= 1;
            }
            bytes.extend_from_slice(&suffix);
            fs::write(p.join(segment_name(0)), &bytes).unwrap();
            if corrupt {
                assert!(Journal::open(&p).is_err());
                assert_eq!(fs::read(p.join(segment_name(0))).unwrap(), bytes);
                assert!(p.join(INTENT).exists());
            } else {
                let j = Journal::open(&p).unwrap();
                assert_eq!(j.replay(Genesis::local(), |_| Ok(())).unwrap().height(), 4);
                drop(j);
            }
            fs::remove_dir_all(p).unwrap();
        }
    }
    // Invoked by the Docker harness on a genuinely bounded tmpfs. Normal unit
    // runs skip the external filesystem fixture; no fault is simulated here.
    #[test]
    fn actual_enospc_child() {
        let Some(path) = std::env::var_os("YAMI_ARCHIVE_ENOSPC_PATH") else {
            return;
        };
        let p = PathBuf::from(path);
        let mode = std::env::var("YAMI_ARCHIVE_ENOSPC_MODE").unwrap();
        let kind = std::env::var("YAMI_ARCHIVE_ENOSPC_KIND").unwrap();
        let limits = StorageLimits {
            max_bytes: 128 * 1024 * 1024,
            reserve_bytes: 0,
        };
        let j = Journal::open_with_limits(&p, DEFAULT_ENTRY_BUDGET, limits).unwrap();
        let r = records(4, 0);
        legacy(&j, &r[..3]);
        if mode != "migration" {
            j.migrate_archive(Genesis::local(), |_| Ok(())).unwrap();
        }
        let before = source(&p);
        let tip = if mode != "migration" {
            Some(fs::read(p.join(TIP)).unwrap())
        } else {
            None
        };
        drop(j);
        if mode == "migration" {
            crash(&p, "migration", "Manifest");
        }
        if mode == "recovery" {
            crash(&p, "append", "Intent");
        }
        let mut fillers = Vec::new();
        if kind == "blocks" {
            let filler = p.join("pressure-filler");
            let mut f = File::create(&filler).unwrap();
            let chunk = vec![0u8; 65536];
            loop {
                match f.write_all(&chunk) {
                    Ok(()) => (),
                    Err(e) => {
                        assert_eq!(e.raw_os_error(), Some(28));
                        break;
                    }
                }
            }
            fillers.push(filler);
        } else {
            for n in 0u64.. {
                let filler = p.join(format!("pressure-inode-{n}"));
                match File::create(&filler) {
                    Ok(_) => fillers.push(filler),
                    Err(e) => {
                        assert_eq!(e.raw_os_error(), Some(28));
                        break;
                    }
                }
            }
        }
        if mode == "recovery" {
            assert!(Journal::open_with_limits(&p, DEFAULT_ENTRY_BUDGET, limits).is_err());
            assert!(p.join(INTENT).exists());
        } else {
            let j = Journal::open_with_limits(&p, DEFAULT_ENTRY_BUDGET, limits).unwrap();
            if mode == "migration" {
                assert!(j.migrate_archive(Genesis::local(), |_| Ok(())).is_err());
                assert!(!fs::read(p.join("genesis.bin")).unwrap().starts_with(FENCE));
            } else {
                assert!(j.append(&r[3]).is_err());
                j.append(&r[0]).unwrap();
                assert_eq!(fs::read(p.join(TIP)).unwrap(), tip.unwrap());
            }
            drop(j);
        }
        assert_eq!(source(&p), before);
        for f in fillers {
            fs::remove_file(f).unwrap();
        }
        let j = Journal::open_with_limits(&p, DEFAULT_ENTRY_BUDGET, limits).unwrap();
        if mode == "migration" {
            j.migrate_archive(Genesis::local(), |_| Ok(())).unwrap();
        }
        j.append(&r[3]).unwrap();
        assert_eq!(j.replay(Genesis::local(), |_| Ok(())).unwrap().height(), 4);
        assert_eq!(source(&p), before);
    }
    #[test]
    fn rollover_and_quota_refusal_preserve_committed_data_and_resume() {
        let p = dir();
        let j = Journal::open(&p).unwrap();
        j.migrate_archive(Genesis::local(), |_| Ok(())).unwrap();
        let r = records(270, MAX_CERTIFICATE_BYTES);
        legacy(&j, &r);
        assert!(j.archive_stats().unwrap().unwrap().segments > 1);
        drop(j);
        let j = Journal::open_with_limits(
            &p,
            DEFAULT_ENTRY_BUDGET,
            StorageLimits {
                max_bytes: 1,
                reserve_bytes: 0,
            },
        )
        .unwrap();
        let before = fs::read(p.join(TIP)).unwrap();
        let next = records(271, MAX_CERTIFICATE_BYTES).pop().unwrap();
        assert!(j.append(&next).is_err());
        assert_eq!(before, fs::read(p.join(TIP)).unwrap());
        j.append(&r[0]).unwrap();
        drop(j);
        let j = Journal::open(&p).unwrap();
        j.append(&next).unwrap();
        assert_eq!(
            j.replay(Genesis::local(), |_| Ok(())).unwrap().height(),
            271
        );
        drop(j);
        fs::remove_dir_all(p).unwrap();
    }
}
