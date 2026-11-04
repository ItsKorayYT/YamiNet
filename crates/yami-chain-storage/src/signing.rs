//! Append-only authorization frames, separately committed, with recoverable intent.
//! No local format can detect rollback of ALL its data and trusted metadata.
use super::*;
use std::{
    collections::BTreeMap,
    io::{Seek, SeekFrom},
};

pub const SIGNING_SEGMENT_BYTES: usize = 1024 * 1024;
pub const MAX_SIGNING_RESERVATIONS: u64 = 1_000_000;
const FRAME_HEADER: usize = 21;
const COMMIT_BYTES: usize = 88;
const HEADER: &[u8] = b"YC-SIGN-COMMITS-1";
const FENCE: &[u8] = b"YC-SIGN2";
const MIGRATION: &str = "signing-migration.bin";
const ACTIVE: &str = "signing-active.bin";
const COMMITS: &str = "signing-commits.bin";
const TIP: &str = "signing-tip.bin";
const INTENT: &str = "signing-intent.bin";
type Key = (u64, u32, u8);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SigningStage {
    MigrationIntent,
    Fence,
    MigrationSegment,
    MigrationVerified,
    Active,
    Intent,
    Segment,
    Commit,
    Tip,
    Cleared,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SigningAnchor {
    pub reservations: u64,
    pub root: Digest,
}
#[derive(Clone, Copy)]
struct Position {
    segment: u32,
    start: u64,
    len: usize,
    sequence: u64,
    root: Digest,
}
pub(super) struct SigningState {
    index: BTreeMap<Key, Position>,
    root: Digest,
    segment: u32,
    end: u64,
    legacy_count: u64,
    header: Vec<u8>,
    preimage_bytes: u64,
}
#[derive(Clone, Debug)]
pub struct SigningStats {
    pub reservations: u64,
    pub preimage_bytes: u64,
    pub segments: u32,
    pub legacy_files: u64,
    pub root: Digest,
}
fn chain(previous: Digest, frame: &[u8]) -> Digest {
    let mut w = Writer::default();
    w.raw(&previous.0);
    w.raw(frame);
    yami_chain::hash(b"YC-SIGN-CHAIN-1", &w.0)
}
fn segment_name(n: u32) -> String {
    format!("signing-segment-{n:08}.bin")
}
fn legacy_name(k: Key) -> String {
    format!("signed-{:020}-{:010}-{}.bin", k.0, k.1, k.2)
}
fn frame(k: Key, preimage: &[u8]) -> io::Result<Vec<u8>> {
    if preimage.len() > MAX_RECORD_BYTES {
        return Err(invalid("signing preimage cap"));
    }
    let mut w = Writer::default();
    w.raw(b"SR01");
    w.u64(k.0);
    w.u32(k.1);
    w.u8(k.2);
    w.bytes(preimage).map_err(invalid)?;
    Ok(w.0)
}
fn decode_frame(bytes: &[u8]) -> io::Result<(Key, &[u8])> {
    let mut r = Reader::new(bytes, MAX_RECORD_BYTES + FRAME_HEADER).map_err(invalid)?;
    if r.take(4).map_err(invalid)? != b"SR01" {
        return Err(invalid("signing frame domain"));
    }
    let k = (
        r.u64().map_err(invalid)?,
        r.u32().map_err(invalid)?,
        r.u8().map_err(invalid)?,
    );
    let preimage = r.bytes(MAX_RECORD_BYTES).map_err(invalid)?;
    r.end().map_err(invalid)?;
    Ok((k, preimage))
}
fn commit(sequence: u64, segment: u32, end: u64, root: Digest) -> Vec<u8> {
    let mut w = Writer::default();
    w.raw(b"SC01");
    w.u64(sequence);
    w.u32(segment);
    w.u64(end);
    w.raw(&root.0);
    w.raw(&yami_chain::hash(b"YC-SIGN-COMMIT-1", &w.0).0);
    w.0
}
fn decode_commit(bytes: &[u8]) -> io::Result<(u64, u32, u64, Digest)> {
    let mut r = Reader::new(bytes, COMMIT_BYTES).map_err(invalid)?;
    if bytes.len() != COMMIT_BYTES || r.take(4).map_err(invalid)? != b"SC01" {
        return Err(invalid("signing commit frame"));
    }
    let out = (
        r.u64().map_err(invalid)?,
        r.u32().map_err(invalid)?,
        r.u64().map_err(invalid)?,
        Digest(r.array().map_err(invalid)?),
    );
    if r.array::<32>().map_err(invalid)? != yami_chain::hash(b"YC-SIGN-COMMIT-1", &bytes[..56]).0 {
        return Err(invalid("signing commit checksum"));
    }
    r.end().map_err(invalid)?;
    Ok(out)
}
fn regular(path: &Path) -> io::Result<fs::Metadata> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_file() {
        return Err(invalid("signing file type"));
    }
    Ok(m)
}
fn read_at(path: &Path, start: u64, len: usize) -> io::Result<Vec<u8>> {
    regular(path)?;
    let mut f = File::open(path)?;
    f.seek(SeekFrom::Start(start))?;
    let mut bytes = vec![0; len];
    f.read_exact(&mut bytes)?;
    Ok(bytes)
}
fn fence(index: u8, active: bool, migration: &[u8]) -> Vec<u8> {
    let mut w = Writer::default();
    w.raw(FENCE);
    w.u8(u8::from(active));
    w.u8(index);
    w.raw(&yami_chain::hash(b"YC-SIGN-MIGRATION-1", migration).0);
    w.0
}
/// Reads only the identity binding, never authorizes signing or log recovery.
pub fn validator_index_at(path: &Path) -> io::Result<Option<u8>> {
    let bytes = match read_bounded(&path.join("validator-index.bin"), 64) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let index = if bytes.len() == 1 {
        bytes[0]
    } else if bytes.len() == 42 && bytes.starts_with(FENCE) && bytes[8] <= 1 {
        bytes[9]
    } else {
        return Err(invalid("validator index format"));
    };
    if index == 255 && bytes.len() == 42 {
        Ok(None)
    } else if index < yami_chain::committee::MAX_VALIDATORS as u8 {
        Ok(Some(index))
    } else {
        Err(invalid("validator index bound"))
    }
}
fn legacy(path: &Path) -> io::Result<(Vec<Key>, Digest)> {
    let mut keys = Vec::new();
    for entry in fs::read_dir(path)? {
        let name = entry?.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("signed-") {
            continue;
        }
        let parts: Vec<_> = name
            .strip_prefix("signed-")
            .unwrap()
            .strip_suffix(".bin")
            .ok_or_else(|| invalid("legacy signing filename"))?
            .split('-')
            .collect();
        if parts.len() != 3 {
            return Err(invalid("legacy signing filename"));
        }
        let k = (
            parts[0].parse().map_err(invalid)?,
            parts[1].parse().map_err(invalid)?,
            parts[2].parse().map_err(invalid)?,
        );
        if legacy_name(k) != name {
            return Err(invalid("noncanonical legacy signing filename"));
        }
        keys.push(k);
        if keys.len() as u64 > MAX_SIGNING_RESERVATIONS {
            return Err(invalid("signing index work limit"));
        }
    }
    keys.sort_unstable();
    let mut root = Digest::default();
    for k in &keys {
        root = chain(
            root,
            &frame(
                *k,
                &read_bounded(&path.join(legacy_name(*k)), MAX_RECORD_BYTES)?,
            )?,
        );
    }
    Ok((keys, root))
}
fn migration_bytes(index: u8, count: u64, root: Digest) -> Vec<u8> {
    let mut w = Writer::default();
    w.raw(b"YC-SIGN-MIG1");
    w.u8(index);
    w.u64(count);
    w.raw(&root.0);
    w.0
}
fn parse_migration(bytes: &[u8]) -> io::Result<(u8, u64, Digest)> {
    let mut r = Reader::new(bytes, 64).map_err(invalid)?;
    if r.take(12).map_err(invalid)? != b"YC-SIGN-MIG1" {
        return Err(invalid("signing migration domain"));
    }
    let out = (
        r.u8().map_err(invalid)?,
        r.u64().map_err(invalid)?,
        Digest(r.array().map_err(invalid)?),
    );
    r.end().map_err(invalid)?;
    if out.1 > MAX_SIGNING_RESERVATIONS {
        return Err(invalid("signing index work limit"));
    }
    Ok(out)
}
fn active_bytes(migration: &[u8]) -> Vec<u8> {
    let mut w = Writer::default();
    w.raw(b"YC-SIGN-ACTIVE-1");
    w.raw(&yami_chain::hash(b"YC-SIGN-MIGRATION-1", migration).0);
    w.0
}
impl Journal {
    // Caller holds write_lock. Never overwrite an immutable migration artifact.
    fn signing_exact(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let path = self.path.join(name);
        match read_bounded(&path, bytes.len()) {
            Ok(old) if old == bytes => {
                File::open(path)?.sync_all()?;
                self.sync_dir()
            }
            Ok(_) => Err(invalid("conflicting signing migration artifact")),
            Err(e) if e.kind() == io::ErrorKind::NotFound => self.signing_replace(name, bytes),
            Err(e) => Err(e),
        }
    }
    fn signing_replace(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        self.signing_replace_inner(name, bytes, |_| Ok(()))
    }
    fn signing_replace_inner(
        &self,
        name: &str,
        bytes: &[u8],
        mut at: impl FnMut(WriteStage) -> io::Result<()>,
    ) -> io::Result<()> {
        self.reserve_capacity(bytes.len() as u64, 1, 1)?;
        let pending = self.path.join(format!(".pending-{name}"));
        let result = (|| {
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&pending)?;
            f.write_all(bytes)?;
            at(WriteStage::FileSync)?;
            f.sync_all()?;
            fs::rename(&pending, self.path.join(name))?;
            at(WriteStage::Renamed)?;
            at(WriteStage::DirectorySync)?;
            self.sync_dir()
        })();
        if result.is_err() {
            let _ = fs::remove_file(pending);
        }
        result
    }
    pub fn bind_validator_index(&self, index: u8) -> io::Result<()> {
        if index >= yami_chain::committee::MAX_VALIDATORS as u8 {
            return Err(invalid("validator index bound"));
        }
        let path = self.path.join("validator-index.bin");
        if fs::symlink_metadata(&path).is_ok() {
            if validator_index_at(&self.path)? != Some(index) {
                return Err(invalid("validator identity binding conflict"));
            }
            File::open(path)?.sync_all()?;
            return self.sync_dir();
        }
        self.write_once("validator-index.bin", &[index])
    }
    pub fn has_signing_reservations(&self) -> io::Result<bool> {
        if self.path.join(INTENT).exists() {
            return Ok(true);
        }
        if let Some(state) = self
            .signing
            .lock()
            .map_err(|_| invalid("poisoned signing index"))?
            .as_ref()
        {
            return Ok(!state.index.is_empty());
        }
        if self.path.join(ACTIVE).exists() {
            return Ok(true);
        }
        for e in fs::read_dir(&self.path)? {
            if e?.file_name().to_string_lossy().starts_with("signed-") {
                return Ok(true);
            }
        }
        Ok(false)
    }
    /// Retains EVERY legacy file. New reservations subsequently use segments.
    pub fn migrate_signatures(&self) -> io::Result<SigningAnchor> {
        self.migrate_signatures_observed(|_| {})
    }
    pub fn migrate_signatures_observed(
        &self,
        mut at: impl FnMut(SigningStage),
    ) -> io::Result<SigningAnchor> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| invalid("poisoned store lock"))?;
        let mut state = self
            .signing
            .lock()
            .map_err(|_| invalid("poisoned signing index"))?;
        if let Some(current) = state.as_ref() {
            current.check_head(self)?;
            return Ok(current.anchor());
        }
        let result = self.migrate_locked(&mut at);
        match result {
            Ok(s) => {
                let a = s.anchor();
                *state = Some(s);
                Ok(a)
            }
            Err(e) => {
                *state = None;
                Err(e)
            }
        }
    }
    fn migrate_locked(&self, at: &mut impl FnMut(SigningStage)) -> io::Result<SigningState> {
        if self.path.join(ACTIVE).exists() {
            return self.load_signing_locked();
        }
        let (keys, root) = legacy(&self.path)?;
        let index = validator_index_at(&self.path)?.unwrap_or(255);
        let migration = migration_bytes(index, keys.len() as u64, root);
        self.signing_exact(MIGRATION, &migration)?;
        at(SigningStage::MigrationIntent);
        let index_path = self.path.join("validator-index.bin");
        if let Ok(old) = read_bounded(&index_path, 64) {
            if old.len() != 1 && old != fence(index, false, &migration) {
                return Err(invalid("missing/rolled-back signing activation"));
            }
        }
        self.signing_replace("validator-index.bin", &fence(index, false, &migration))?;
        at(SigningStage::Fence);
        let mut commits = Vec::from(HEADER);
        commits.extend_from_slice(&yami_chain::hash(b"YC-SIGN-MIGRATION-1", &migration).0);
        let mut segment = 0;
        let mut data = Vec::new();
        let mut running = Digest::default();
        for (i, key) in keys.iter().enumerate() {
            let bytes = frame(
                *key,
                &read_bounded(&self.path.join(legacy_name(*key)), MAX_RECORD_BYTES)?,
            )?;
            if data.len() + bytes.len() > SIGNING_SEGMENT_BYTES {
                self.signing_exact(&segment_name(segment), &data)?;
                at(SigningStage::MigrationSegment);
                segment += 1;
                data.clear();
            }
            data.extend_from_slice(&bytes);
            running = chain(running, &bytes);
            commits.extend_from_slice(&commit(i as u64 + 1, segment, data.len() as u64, running));
        }
        if !data.is_empty() {
            self.signing_exact(&segment_name(segment), &data)?;
            at(SigningStage::MigrationSegment);
        }
        self.signing_exact(COMMITS, &commits)?;
        // Re-read both representations before publication; no partial inventory
        // or unchecked staged file can authorize activation.
        let checked = self.scan_signing(&migration, false)?;
        if checked.anchor()
            != (SigningAnchor {
                reservations: keys.len() as u64,
                root,
            })
        {
            return Err(invalid("incomplete signing migration"));
        }
        at(SigningStage::MigrationVerified);
        self.signing_exact(
            TIP,
            &commit(
                keys.len() as u64,
                checked.segment,
                checked.end,
                checked.root,
            ),
        )?;
        self.signing_exact(ACTIVE, &active_bytes(&migration))?;
        at(SigningStage::Active);
        self.signing_replace("validator-index.bin", &fence(index, true, &migration))?;
        self.scan_signing(&migration, true)
    }
    pub(super) fn recover_signing(&self) -> io::Result<()> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| invalid("poisoned store lock"))?;
        let mut state = self
            .signing
            .lock()
            .map_err(|_| invalid("poisoned signing index"))?;
        if self.path.join(ACTIVE).exists() {
            *state = Some(self.load_signing_locked()?);
        } else if self.path.join(MIGRATION).exists() {
            let migration = read_bounded(&self.path.join(MIGRATION), 64)?;
            let (index, count, root) = parse_migration(&migration)?;
            let (keys, actual) = legacy(&self.path)?;
            if keys.len() as u64 != count || actual != root {
                return Err(invalid("incomplete legacy signing inventory"));
            }
            let binding = read_bounded(&self.path.join("validator-index.bin"), 64);
            if binding
                .as_ref()
                .is_ok_and(|b| *b == fence(index, true, &migration))
            {
                return Err(invalid("missing signing activation witness"));
            }
            // A pending migration cannot sign; explicit migration/reservation will
            // re-read all inputs and verify staged completeness before activation.
        } else {
            for e in fs::read_dir(&self.path)? {
                if e?.file_name().to_string_lossy().starts_with("signing-") {
                    return Err(invalid("orphan signing storage; refusing legacy fallback"));
                }
            }
            if read_bounded(&self.path.join("validator-index.bin"), 64)
                .is_ok_and(|b| b.starts_with(FENCE))
            {
                return Err(invalid("missing signing migration witness"));
            }
        }
        Ok(())
    }
    fn load_signing_locked(&self) -> io::Result<SigningState> {
        let migration = read_bounded(&self.path.join(MIGRATION), 64)?;
        let (index, _, _) = parse_migration(&migration)?;
        if read_bounded(&self.path.join(ACTIVE), 64)? != active_bytes(&migration) {
            return Err(invalid("signing activation binding"));
        }
        let binding = read_bounded(&self.path.join("validator-index.bin"), 64)?;
        if binding == fence(index, false, &migration) {
            let state = self.scan_signing(&migration, false)?;
            if state.index.len() as u64 != state.legacy_count {
                return Err(invalid("rolled-back signing format fence"));
            }
            self.signing_replace("validator-index.bin", &fence(index, true, &migration))?;
        } else if binding != fence(index, true, &migration) {
            return Err(invalid("signing format fence mismatch"));
        }
        // A durable intent is itself an authorization. Finish only its exact
        // bytes, including partial append prefixes; never truncate unknown data.
        self.recover_intent(&migration)?;
        self.scan_signing(&migration, true)
    }
    fn scan_signing(&self, migration: &[u8], active: bool) -> io::Result<SigningState> {
        let (_, legacy_count, legacy_root) = parse_migration(migration)?;
        let (keys, actual) = legacy(&self.path)?;
        if keys.len() as u64 != legacy_count || actual != legacy_root {
            return Err(invalid("legacy signing completeness mismatch"));
        }
        let mut header = Vec::from(HEADER);
        header.extend_from_slice(&yami_chain::hash(b"YC-SIGN-MIGRATION-1", migration).0);
        let path = self.path.join(COMMITS);
        let size = regular(&path)?.len();
        if size < header.len() as u64
            || !(size - header.len() as u64).is_multiple_of(COMMIT_BYTES as u64)
        {
            return Err(invalid("truncated signing commit stream"));
        }
        let count = (size - header.len() as u64) / COMMIT_BYTES as u64;
        if count > MAX_SIGNING_RESERVATIONS
            || count < legacy_count
            || (!active && count != legacy_count)
        {
            return Err(invalid("signing commit count/work limit"));
        }
        let mut f = std::io::BufReader::new(File::open(path)?);
        let mut h = vec![0; header.len()];
        f.read_exact(&mut h)?;
        if h != header {
            return Err(invalid("signing commit migration binding"));
        }
        let mut state = SigningState {
            index: BTreeMap::new(),
            root: Digest::default(),
            segment: 0,
            end: 0,
            legacy_count,
            header,
            preimage_bytes: 0,
        };
        let mut segment_reader: Option<std::io::BufReader<File>> = None;
        for sequence in 1..=count {
            let mut bytes = [0; COMMIT_BYTES];
            f.read_exact(&mut bytes)?;
            let (n, seg, end, root) = decode_commit(&bytes)?;
            let start = if seg == state.segment {
                state.end
            } else if seg == state.segment + 1 && state.end > 0 {
                0
            } else {
                return Err(invalid("signing segment gap"));
            };
            if n != sequence
                || end <= start
                || end > SIGNING_SEGMENT_BYTES as u64
                || end - start > (MAX_RECORD_BYTES + FRAME_HEADER) as u64
            {
                return Err(invalid("signing position/count"));
            }
            if seg != state.segment && state.end + (end - start) <= SIGNING_SEGMENT_BYTES as u64 {
                return Err(invalid("noncanonical signing segment rollover"));
            }
            if seg != state.segment
                && regular(&self.path.join(segment_name(state.segment)))?.len() != state.end
            {
                return Err(invalid("sealed signing segment length"));
            }
            if segment_reader.is_none() || seg != state.segment {
                let path = self.path.join(segment_name(seg));
                regular(&path)?;
                segment_reader = Some(std::io::BufReader::new(File::open(path)?));
            }
            let mut raw = vec![0; (end - start) as usize];
            segment_reader
                .as_mut()
                .ok_or_else(|| invalid("missing segment reader"))?
                .read_exact(&mut raw)?;
            let (key, _) = decode_frame(&raw)?;
            if sequence <= legacy_count && keys[(sequence - 1) as usize] != key {
                return Err(invalid("legacy signing migration key mismatch"));
            }
            if chain(state.root, &raw) != root
                || state
                    .index
                    .insert(
                        key,
                        Position {
                            segment: seg,
                            start,
                            len: raw.len(),
                            sequence,
                            root,
                        },
                    )
                    .is_some()
            {
                return Err(invalid("signing chain/duplicate authorization"));
            }
            state.preimage_bytes += (raw.len() - FRAME_HEADER) as u64;
            state.root = root;
            state.segment = seg;
            state.end = end;
            if sequence == legacy_count && root != legacy_root {
                return Err(invalid("signing migration prefix root"));
            }
        }
        let expected = if count == 0 { 0 } else { state.segment + 1 };
        for e in fs::read_dir(&self.path)? {
            let e = e?;
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("signing-")
                && ![MIGRATION, ACTIVE, COMMITS, TIP, INTENT].contains(&name.as_ref())
                && !(name.starts_with("signing-segment-") && name.ends_with(".bin"))
            {
                return Err(invalid("unknown signing storage entry"));
            }
            if let Some(n) = name
                .strip_prefix("signing-segment-")
                .and_then(|s| s.strip_suffix(".bin"))
            {
                let n: u32 = n.parse().map_err(invalid)?;
                if name != segment_name(n) || n >= expected {
                    return Err(invalid("extra/noncanonical signing segment"));
                }
            }
        }
        if count > 0 && regular(&self.path.join(segment_name(state.segment)))?.len() != state.end {
            return Err(invalid("uncommitted signing segment tail"));
        }
        if active
            && read_bounded(&self.path.join(TIP), COMMIT_BYTES)?
                != commit(count, state.segment, state.end, state.root)
        {
            return Err(invalid("signing tip rollback/mismatch"));
        }
        Ok(state)
    }
    fn recover_intent(&self, migration: &[u8]) -> io::Result<()> {
        let bytes = match read_bounded(&self.path.join(INTENT), MAX_RECORD_BYTES + 256) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        let mut r = Reader::new(&bytes, MAX_RECORD_BYTES + 256).map_err(invalid)?;
        if r.take(4).map_err(invalid)? != b"SI01" {
            return Err(invalid("signing intent domain"));
        }
        let previous = SigningAnchor {
            reservations: r.u64().map_err(invalid)?,
            root: Digest(r.array().map_err(invalid)?),
        };
        let commit_bytes = r.take(COMMIT_BYTES).map_err(invalid)?.to_vec();
        let raw = r
            .bytes(MAX_RECORD_BYTES + FRAME_HEADER)
            .map_err(invalid)?
            .to_vec();
        let sum = r.array::<32>().map_err(invalid)?;
        r.end().map_err(invalid)?;
        if sum != yami_chain::hash(b"YC-SIGN-INTENT-1", &bytes[..bytes.len() - 32]).0 {
            return Err(invalid("signing intent checksum"));
        }
        let (n, seg, end, root) = decode_commit(&commit_bytes)?;
        decode_frame(&raw)?;
        if previous.reservations >= MAX_SIGNING_RESERVATIONS
            || n != previous.reservations.saturating_add(1)
            || n > MAX_SIGNING_RESERVATIONS
            || chain(previous.root, &raw) != root
            || end < raw.len() as u64
            || end > SIGNING_SEGMENT_BYTES as u64
        {
            return Err(invalid("signing intent binding"));
        }
        let header_len = HEADER.len() + 32;
        let commit_start = header_len as u64 + previous.reservations * COMMIT_BYTES as u64;
        let cpath = self.path.join(COMMITS);
        let csize = regular(&cpath)?.len();
        let header = read_at(&cpath, 0, header_len)?;
        if !header.starts_with(HEADER)
            || header[HEADER.len()..] != yami_chain::hash(b"YC-SIGN-MIGRATION-1", migration).0
        {
            return Err(invalid("intent migration binding"));
        }
        if previous.reservations > 0 {
            let (seq, prevseg, prevend, prevroot) = decode_commit(&read_at(
                &cpath,
                commit_start - COMMIT_BYTES as u64,
                COMMIT_BYTES,
            )?)?;
            if seq != previous.reservations
                || prevroot != previous.root
                || !(seg == prevseg && end - raw.len() as u64 == prevend
                    || Some(seg) == prevseg.checked_add(1) && end == raw.len() as u64)
            {
                return Err(invalid("intent previous commit mismatch"));
            }
        } else if previous.root != Digest::default() || seg != 0 || end != raw.len() as u64 {
            return Err(invalid("intent initial position"));
        }
        let tip = read_bounded(&self.path.join(TIP), COMMIT_BYTES)?;
        let tip_state = decode_commit(&tip)?;
        if !(tip_state.0 == previous.reservations && tip_state.3 == previous.root
            || tip == commit_bytes)
        {
            return Err(invalid("intent/tip witness mismatch"));
        }
        if csize < commit_start || csize > commit_start + COMMIT_BYTES as u64 {
            return Err(invalid("signing intent commit position"));
        }
        let segment_path = self.path.join(segment_name(seg));
        let segment_start = end - raw.len() as u64;
        let slen = self.validate_append(&segment_path, segment_start, &raw)?;
        self.validate_append(&cpath, commit_start, &commit_bytes)?;
        if slen < end || csize < commit_start + COMMIT_BYTES as u64 {
            self.reserve_capacity((raw.len() + COMMIT_BYTES) as u64, 2, 1)?;
        }
        self.append_exact(&segment_path, segment_start, &raw)?;
        self.append_exact(&cpath, commit_start, &commit_bytes)?;
        if tip != commit_bytes {
            self.signing_replace(TIP, &commit_bytes)?;
        } else {
            File::open(self.path.join(TIP))?.sync_all()?;
            self.sync_dir()?;
        }
        fs::remove_file(self.path.join(INTENT))?;
        self.sync_dir()
    }
    fn validate_append(&self, path: &Path, start: u64, bytes: &[u8]) -> io::Result<u64> {
        let len = match regular(path) {
            Ok(m) => m.len(),
            Err(e) if e.kind() == io::ErrorKind::NotFound && start == 0 => 0,
            Err(e) => return Err(e),
        };
        if len < start || len > start + bytes.len() as u64 {
            return Err(invalid("signing append length mismatch"));
        }
        if len > start
            && read_at(path, start, (len - start) as usize)? != bytes[..(len - start) as usize]
        {
            return Err(invalid("signing append prefix mismatch"));
        }
        Ok(len)
    }
    fn append_exact(&self, path: &Path, start: u64, bytes: &[u8]) -> io::Result<()> {
        let len = self.validate_append(path, start, bytes)?;
        let mut f = OpenOptions::new()
            .create(true)
            .truncate(false)
            .append(true)
            .open(path)?;
        f.write_all(&bytes[(len - start) as usize..])?;
        f.sync_all()?;
        self.sync_dir()
    }
    pub fn signing_stats(&self) -> io::Result<Option<SigningStats>> {
        let state = self
            .signing
            .lock()
            .map_err(|_| invalid("poisoned signing index"))?;
        if state.is_none() && self.path.join(MIGRATION).exists() {
            return Err(invalid("signing index needs recovery"));
        }
        Ok(state.as_ref().map(|s| SigningStats {
            reservations: s.index.len() as u64,
            preimage_bytes: s.preimage_bytes,
            segments: if s.index.is_empty() { 0 } else { s.segment + 1 },
            legacy_files: s.legacy_count,
            root: s.root,
        }))
    }
    pub fn signing_anchor(&self) -> io::Result<Option<SigningAnchor>> {
        let state = self
            .signing
            .lock()
            .map_err(|_| invalid("poisoned signing index"))?;
        if state.is_none() && self.path.join(MIGRATION).exists() {
            return Err(invalid("signing index needs recovery"));
        }
        Ok(state.as_ref().map(SigningState::anchor))
    }
    /// The caller must obtain this minimum anchor independently of this store.
    pub fn verify_signing_anchor(&self, anchor: SigningAnchor) -> io::Result<()> {
        let state = self
            .signing
            .lock()
            .map_err(|_| invalid("poisoned signing index"))?;
        let state = state
            .as_ref()
            .ok_or_else(|| invalid("segmented signing log required"))?;
        if anchor.reservations > state.index.len() as u64 {
            return Err(invalid("signing anchor rollback"));
        }
        let root = if anchor.reservations == 0 {
            Digest::default()
        } else {
            decode_commit(&read_at(
                &self.path.join(COMMITS),
                state.header.len() as u64 + (anchor.reservations - 1) * COMMIT_BYTES as u64,
                COMMIT_BYTES,
            )?)?
            .3
        };
        if root != anchor.root {
            return Err(invalid("signing anchor mismatch"));
        }
        Ok(())
    }
    pub fn signature_reservation(
        &self,
        height: u64,
        round: u32,
        step: u8,
    ) -> io::Result<Option<Vec<u8>>> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| invalid("poisoned store lock"))?;
        let state = self
            .signing
            .lock()
            .map_err(|_| invalid("poisoned signing index"))?;
        let key = (height, round, step);
        if let Some(state) = state.as_ref() {
            state.check_head(self)?;
            return state
                .read(self, key)
                .map(|o| o.map(|b| b[FRAME_HEADER..].to_vec()));
        }
        if self.path.join(MIGRATION).exists() {
            return Err(invalid("signing index needs recovery"));
        }
        match read_bounded(&self.path.join(legacy_name(key)), MAX_RECORD_BYTES) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    pub(super) fn reserve_segmented(
        &self,
        key: Key,
        preimage: &[u8],
        mut at: impl FnMut(SigningStage),
    ) -> io::Result<()> {
        if preimage.len() > MAX_RECORD_BYTES {
            return Err(invalid("signing preimage cap"));
        }
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| invalid("poisoned store lock"))?;
        let mut state = self
            .signing
            .lock()
            .map_err(|_| invalid("poisoned signing index"))?;
        if state.is_none() {
            *state = Some(self.migrate_locked(&mut at)?);
        }
        let current = state
            .as_ref()
            .ok_or_else(|| invalid("missing signing index"))?;
        current.check_head(self)?;
        if let Some(old) = current.read(self, key)? {
            if decode_frame(&old)?.1 != preimage {
                return Err(invalid("conflicting durable signing reservation"));
            }
            File::open(self.path.join(segment_name(current.index[&key].segment)))?.sync_all()?;
            File::open(self.path.join(COMMITS))?.sync_all()?;
            return self.sync_dir();
        }
        if current.index.len() as u64 >= MAX_SIGNING_RESERVATIONS {
            return Err(invalid("signing index work limit"));
        }
        let raw = frame(key, preimage)?;
        let seg = if current.end + raw.len() as u64 > SIGNING_SEGMENT_BYTES as u64 {
            current.segment + 1
        } else {
            current.segment
        };
        let start = if seg == current.segment {
            current.end
        } else {
            0
        };
        let root = chain(current.root, &raw);
        let c = commit(
            current.index.len() as u64 + 1,
            seg,
            start + raw.len() as u64,
            root,
        );
        let mut w = Writer::default();
        w.raw(b"SI01");
        w.u64(current.index.len() as u64);
        w.raw(&current.root.0);
        w.raw(&c);
        w.bytes(&raw).map_err(invalid)?;
        w.raw(&yami_chain::hash(b"YC-SIGN-INTENT-1", &w.0).0);
        // Pure preflight refusal preserves the already verified in-memory index.
        // Any error after this point requires recovery before further signing.
        self.reserve_capacity((w.0.len() + raw.len() + 2 * c.len()) as u64, 4, 3)?;
        let result = (|| {
            self.signing_exact(INTENT, &w.0)?;
            at(SigningStage::Intent);
            self.append_exact(&self.path.join(segment_name(seg)), start, &raw)?;
            at(SigningStage::Segment);
            self.append_exact(
                &self.path.join(COMMITS),
                current.header.len() as u64 + current.index.len() as u64 * COMMIT_BYTES as u64,
                &c,
            )?;
            at(SigningStage::Commit);
            self.signing_replace(TIP, &c)?;
            at(SigningStage::Tip);
            fs::remove_file(self.path.join(INTENT))?;
            self.sync_dir()?;
            at(SigningStage::Cleared);
            Ok(())
        })();
        if let Err(e) = result {
            *state = None;
            return Err(e);
        }
        let current = state
            .as_mut()
            .ok_or_else(|| invalid("missing signing index"))?;
        current.index.insert(
            key,
            Position {
                segment: seg,
                start,
                len: raw.len(),
                sequence: current.index.len() as u64 + 1,
                root,
            },
        );
        current.segment = seg;
        current.end = start + raw.len() as u64;
        current.root = root;
        current.preimage_bytes += preimage.len() as u64;
        Ok(())
    }
}
impl SigningState {
    fn anchor(&self) -> SigningAnchor {
        SigningAnchor {
            reservations: self.index.len() as u64,
            root: self.root,
        }
    }
    fn check_head(&self, journal: &Journal) -> io::Result<()> {
        let migration = read_bounded(&journal.path.join(MIGRATION), 64)?;
        let (index, _, _) = parse_migration(&migration)?;
        if read_bounded(&journal.path.join(ACTIVE), 64)? != active_bytes(&migration)
            || read_bounded(&journal.path.join("validator-index.bin"), 64)?
                != fence(index, true, &migration)
            || self.header[HEADER.len()..] != yami_chain::hash(b"YC-SIGN-MIGRATION-1", &migration).0
        {
            return Err(invalid("signing control metadata changed"));
        }
        if read_bounded(&journal.path.join(TIP), COMMIT_BYTES)?
            != commit(self.index.len() as u64, self.segment, self.end, self.root)
        {
            return Err(invalid("signing tip witness changed"));
        }
        let path = journal.path.join(COMMITS);
        if regular(&path)?.len()
            != self.header.len() as u64 + self.index.len() as u64 * COMMIT_BYTES as u64
            || read_at(&path, 0, self.header.len())? != self.header
        {
            return Err(invalid("signing commit stream changed"));
        }
        if !self.index.is_empty() {
            let c = decode_commit(&read_at(
                &path,
                self.header.len() as u64 + (self.index.len() as u64 - 1) * COMMIT_BYTES as u64,
                COMMIT_BYTES,
            )?)?;
            if c != (self.index.len() as u64, self.segment, self.end, self.root)
                || regular(&journal.path.join(segment_name(self.segment)))?.len() != self.end
            {
                return Err(invalid("signing committed head changed"));
            }
        }
        Ok(())
    }
    fn read(&self, journal: &Journal, key: Key) -> io::Result<Option<Vec<u8>>> {
        let Some(p) = self.index.get(&key) else {
            return Ok(None);
        };
        let raw = read_at(&journal.path.join(segment_name(p.segment)), p.start, p.len)?;
        if decode_frame(&raw)?.0 != key {
            return Err(invalid("signing index key mismatch"));
        }
        let c = read_at(
            &journal.path.join(COMMITS),
            self.header.len() as u64 + (p.sequence - 1) * COMMIT_BYTES as u64,
            COMMIT_BYTES,
        )?;
        let (seq, seg, end, root) = decode_commit(&c)?;
        let previous = if p.sequence == 1 {
            Digest::default()
        } else {
            decode_commit(&read_at(
                &journal.path.join(COMMITS),
                self.header.len() as u64 + (p.sequence - 2) * COMMIT_BYTES as u64,
                COMMIT_BYTES,
            )?)?
            .3
        };
        if seq != p.sequence
            || seg != p.segment
            || end != p.start + p.len as u64
            || root != p.root
            || chain(previous, &raw) != root
        {
            return Err(invalid("signing index/commit binding"));
        }
        Ok(Some(raw))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        process::Command,
        sync::atomic::{AtomicU64, Ordering},
    };
    static ID: AtomicU64 = AtomicU64::new(0);
    fn directory() -> PathBuf {
        std::env::temp_dir().join(format!(
            "yc-signing-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ))
    }
    fn open(path: &Path) -> io::Result<Journal> {
        for _ in 0..1000 {
            match Journal::open(path) {
                Err(e) if e.to_string().starts_with("store already in use:") => {
                    std::thread::sleep(std::time::Duration::from_millis(1))
                }
                result => return result,
            }
        }
        panic!("unexpected persistent lock owner in signing test");
    }
    fn seeded() -> PathBuf {
        let p = directory();
        let j = open(&p).unwrap();
        j.bind_validator_index(0).unwrap();
        j.reserve_signature(1, 0, 1, b"one").unwrap();
        drop(j);
        p
    }
    fn files(p: &Path) -> BTreeMap<String, Vec<u8>> {
        fs::read_dir(p)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                (
                    e.file_name().to_string_lossy().into_owned(),
                    fs::read(e.path()).unwrap(),
                )
            })
            .collect()
    }
    fn pending(p: &Path) -> (Vec<u8>, Vec<u8>) {
        let bytes = fs::read(p.join(INTENT)).unwrap();
        let mut r = Reader::new(&bytes, MAX_RECORD_BYTES + 256).unwrap();
        r.take(4 + 8 + 32).unwrap();
        let c = r.take(COMMIT_BYTES).unwrap().to_vec();
        let raw = r.bytes(MAX_RECORD_BYTES + FRAME_HEADER).unwrap().to_vec();
        (c, raw)
    }
    fn child(p: &Path, mode: &str, stage: &str) {
        let s = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "signing::tests::crash_child", "--nocapture"])
            .env("YAMI_SIGN_TEST_DIR", p)
            .env("YAMI_SIGN_TEST_MODE", mode)
            .env("YAMI_SIGN_TEST_STAGE", stage)
            .status()
            .unwrap();
        assert_eq!(s.code(), Some(86), "{mode} {stage}");
    }
    #[test]
    fn crash_child() {
        let Some(p) = std::env::var_os("YAMI_SIGN_TEST_DIR") else {
            return;
        };
        let p = PathBuf::from(p);
        let mode = std::env::var("YAMI_SIGN_TEST_MODE").unwrap();
        let stage = std::env::var("YAMI_SIGN_TEST_STAGE").unwrap();
        let j = open(&p).unwrap();
        let mut at = |s: SigningStage| {
            if format!("{s:?}") == stage {
                std::process::exit(86)
            }
        };
        if mode == "migrate" {
            j.migrate_signatures_observed(&mut at).unwrap();
        } else if mode == "append" {
            j.reserve_segmented((2, 0, 1), b"two", &mut at).unwrap();
        } else {
            // Durable intent, then exact data/commit bytes, then uncertain atomic
            // tip publication. Parent tests every real I/O publication boundary.
            let old = j.signing_anchor().unwrap().unwrap();
            let raw = frame((2, 0, 1), b"two").unwrap();
            let state = j.signing.lock().unwrap();
            let state = state.as_ref().unwrap();
            let c = commit(
                2,
                state.segment,
                state.end + raw.len() as u64,
                chain(old.root, &raw),
            );
            let mut w = Writer::default();
            w.raw(b"SI01");
            w.u64(old.reservations);
            w.raw(&old.root.0);
            w.raw(&c);
            w.bytes(&raw).unwrap();
            w.raw(&yami_chain::hash(b"YC-SIGN-INTENT-1", &w.0).0);
            let _guard = j.write_lock.lock().unwrap();
            j.signing_exact(INTENT, &w.0).unwrap();
            j.append_exact(&p.join(segment_name(state.segment)), state.end, &raw)
                .unwrap();
            j.append_exact(
                &p.join(COMMITS),
                state.header.len() as u64 + COMMIT_BYTES as u64,
                &c,
            )
            .unwrap();
            j.signing_replace_inner(TIP, &c, |s| {
                if format!("{s:?}") == stage {
                    std::process::exit(86)
                }
                Ok(())
            })
            .unwrap();
        }
        panic!("unreached signing crash stage");
    }
    #[test]
    fn append_publication_process_interruptions_never_forget_authorizations() {
        for stage in ["Intent", "Segment", "Commit", "Tip", "Cleared"] {
            let p = seeded();
            child(&p, "append", stage);
            let j = open(&p).unwrap();
            assert_eq!(j.signing_anchor().unwrap().unwrap().reservations, 2);
            assert!(j.reserve_signature(2, 0, 1, b"fork").is_err());
            j.reserve_signature(1, 0, 1, b"one").unwrap();
            j.reserve_signature(2, 0, 1, b"two").unwrap();
            assert_eq!(j.signature_reservation(2, 0, 1).unwrap().unwrap(), b"two");
            assert!(!p.join(INTENT).exists());
            drop(j);
            fs::remove_dir_all(p).unwrap();
        }
    }
    #[test]
    fn atomic_tip_failures_before_sync_after_rename_and_directory_sync_recover() {
        for stage in ["FileSync", "Renamed", "DirectorySync"] {
            let p = seeded();
            child(&p, "tip", stage);
            let j = open(&p).unwrap();
            j.reserve_signature(2, 0, 1, b"two").unwrap();
            assert!(j.reserve_signature(2, 0, 1, b"fork").is_err());
            assert_eq!(j.signing_anchor().unwrap().unwrap().reservations, 2);
            drop(j);
            fs::remove_dir_all(p).unwrap();
        }
    }
    #[test]
    fn partial_data_and_commit_appends_recover_only_exact_intent_prefixes() {
        for commit_partial in [false, true] {
            let p = seeded();
            child(&p, "append", "Intent");
            let (c, raw) = pending(&p);
            let (_, seg, _, _) = decode_commit(&c).unwrap();
            let n = if commit_partial {
                raw.len()
            } else {
                raw.len() / 2
            };
            OpenOptions::new()
                .append(true)
                .open(p.join(segment_name(seg)))
                .unwrap()
                .write_all(&raw[..n])
                .unwrap();
            if commit_partial {
                OpenOptions::new()
                    .append(true)
                    .open(p.join(COMMITS))
                    .unwrap()
                    .write_all(&c[..COMMIT_BYTES / 2])
                    .unwrap();
            }
            let j = open(&p).unwrap();
            j.reserve_signature(2, 0, 1, b"two").unwrap();
            assert!(j.reserve_signature(2, 0, 1, b"fork").is_err());
            drop(j);
            fs::remove_dir_all(p).unwrap();
        }
        let p = seeded();
        child(&p, "append", "Intent");
        OpenOptions::new()
            .append(true)
            .open(p.join(segment_name(0)))
            .unwrap()
            .write_all(b"wrong prefix")
            .unwrap();
        let before = files(&p);
        assert!(open(&p).is_err());
        assert_eq!(files(&p), before);
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn migration_interruptions_keep_complete_legacy_source_and_resume_verified() {
        for stage in [
            "MigrationIntent",
            "Fence",
            "MigrationSegment",
            "MigrationVerified",
            "Active",
        ] {
            let p = directory();
            let j = open(&p).unwrap();
            j.bind_validator_index(0).unwrap();
            // Large bounded preimages force multiple segments without expensive
            // consensus fixtures. These are durable legacy authorizations.
            let count = SIGNING_SEGMENT_BYTES / (MAX_RECORD_BYTES + FRAME_HEADER) + 2;
            for i in 0..count {
                let path = p.join(legacy_name((i as u64, 7, 2)));
                fs::write(&path, vec![i as u8; MAX_RECORD_BYTES]).unwrap();
                File::open(path).unwrap().sync_all().unwrap();
            }
            j.sync_dir().unwrap();
            let before = files(&p);
            drop(j);
            child(&p, "migrate", stage);
            let j = open(&p).unwrap();
            let a = j.migrate_signatures().unwrap();
            assert_eq!(a.reservations, count as u64);
            for (name, bytes) in &before {
                if name == "validator-index.bin" {
                    continue;
                }
                assert_eq!(fs::read(p.join(name)).unwrap(), *bytes);
            }
            assert_eq!(validator_index_at(&p).unwrap(), Some(0));
            assert!(j.write_once("validator-index.bin", &[0]).is_err()); // M10 downgrade fence.
            for i in 0..count {
                j.reserve_signature(i as u64, 7, 2, &vec![i as u8; MAX_RECORD_BYTES])
                    .unwrap();
                assert!(j.reserve_signature(i as u64, 7, 2, b"fork").is_err());
            }
            j.reserve_signature(100_000, 0, 1, b"future").unwrap();
            assert!(!p.join(legacy_name((100_000, 0, 1))).exists());
            drop(j);
            fs::remove_dir_all(p).unwrap();
        }
    }
    #[test]
    fn incomplete_migration_missing_or_changed_legacy_inputs_never_activates() {
        for missing in [false, true] {
            let p = directory();
            let j = open(&p).unwrap();
            j.bind_validator_index(0).unwrap();
            j.write_once(&legacy_name((1, 0, 1)), b"one").unwrap();
            j.write_once(&legacy_name((2, 0, 1)), b"two").unwrap();
            drop(j);
            child(&p, "migrate", "MigrationIntent");
            if missing {
                fs::remove_file(p.join(legacy_name((2, 0, 1)))).unwrap();
            } else {
                fs::write(p.join(legacy_name((2, 0, 1))), b"different").unwrap();
            }
            assert!(open(&p).is_err());
            assert!(!p.join(ACTIVE).exists());
            assert_eq!(fs::read(p.join(legacy_name((1, 0, 1)))).unwrap(), b"one");
            fs::remove_dir_all(p).unwrap();
        }
    }
    #[test]
    fn corruption_missing_files_and_selective_rollbacks_fail_closed() {
        for target in [
            MIGRATION,
            ACTIVE,
            COMMITS,
            TIP,
            "validator-index.bin",
            "signing-segment-00000000.bin",
        ] {
            for remove in [false, true] {
                let p = seeded();
                let j = open(&p).unwrap();
                j.reserve_signature(2, 0, 1, b"two").unwrap();
                drop(j);
                let path = p.join(target);
                if remove {
                    fs::remove_file(&path).unwrap();
                } else {
                    let mut b = fs::read(&path).unwrap();
                    let n = b.len() / 2;
                    b[n] ^= 0x80;
                    fs::write(&path, b).unwrap();
                }
                assert!(open(&p).is_err(), "{target} remove={remove}");
                fs::remove_dir_all(p).unwrap();
            }
        }
        let p = seeded();
        let first = files(&p);
        let j = open(&p).unwrap();
        j.reserve_signature(2, 0, 1, b"two").unwrap();
        let anchor = j.signing_anchor().unwrap().unwrap();
        drop(j);
        // Joint segment+commit rollback is caught by the separate tip witness.
        for name in [COMMITS, "signing-segment-00000000.bin"] {
            fs::write(p.join(name), &first[name]).unwrap();
        }
        assert!(open(&p).is_err());
        // Complete rollback of all signing data AND its trusted tip is locally
        // indistinguishable from the old state. An independently pinned minimum
        // anchor rejects it. Do not claim hash chains solve full local rollback.
        fs::write(p.join(TIP), &first[TIP]).unwrap();
        let j = open(&p).unwrap();
        assert!(j.verify_signing_anchor(anchor).is_err());
        drop(j);
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn forged_old_frame_and_commit_cannot_override_loaded_index_binding() {
        let p = seeded();
        let j = open(&p).unwrap();
        j.reserve_signature(2, 0, 1, b"two").unwrap();
        let raw = frame((1, 0, 1), b"bad").unwrap();
        let fake = commit(1, 0, raw.len() as u64, chain(Digest::default(), &raw));
        let mut f = OpenOptions::new()
            .write(true)
            .open(p.join(segment_name(0)))
            .unwrap();
        f.write_all(&raw).unwrap();
        let mut c = OpenOptions::new()
            .write(true)
            .open(p.join(COMMITS))
            .unwrap();
        let header = HEADER.len() + 32;
        c.seek(SeekFrom::Start(header as u64)).unwrap();
        c.write_all(&fake).unwrap();
        assert!(j.reserve_signature(1, 0, 1, b"bad").is_err());
        drop(j);
        assert!(open(&p).is_err());
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn quota_refusal_and_pressured_intent_recovery_retain_every_authorization() {
        let p = seeded();
        let j = open(&p).unwrap();
        let used = j.storage_footprint(100).unwrap().allocated_bytes;
        drop(j);
        let j = Journal::open_with_limits(
            &p,
            100,
            StorageLimits {
                max_bytes: used + 4096,
                reserve_bytes: 0,
            },
        )
        .unwrap();
        j.reserve_signature(1, 0, 1, b"one").unwrap();
        let before = files(&p);
        assert!(j.reserve_signature(2, 0, 1, b"two").is_err());
        assert_eq!(files(&p), before);
        drop(j);
        child(&p, "append", "Intent");
        let before = files(&p);
        assert!(Journal::open_with_limits(
            &p,
            100,
            StorageLimits {
                max_bytes: 1,
                reserve_bytes: 0
            }
        )
        .is_err());
        assert_eq!(files(&p), before);
        let j = open(&p).unwrap();
        j.reserve_signature(2, 0, 1, b"two").unwrap();
        assert!(j.reserve_signature(2, 0, 1, b"fork").is_err());
        drop(j);
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn concurrent_conflicting_requests_authorize_exactly_one_preimage() {
        let p = directory();
        let j = std::sync::Arc::new(open(&p).unwrap());
        j.bind_validator_index(0).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(17));
        let threads: Vec<_> = (0..16)
            .map(|i| {
                let j = j.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let value = if i % 2 == 0 {
                        b"left".as_slice()
                    } else {
                        b"right".as_slice()
                    };
                    barrier.wait();
                    (value, j.reserve_signature(1, 0, 1, value).is_ok())
                })
            })
            .collect();
        barrier.wait();
        let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        let winner = j.signature_reservation(1, 0, 1).unwrap().unwrap();
        assert_eq!(j.signing_anchor().unwrap().unwrap().reservations, 1);
        for (value, accepted) in results {
            assert_eq!(accepted, value == winner);
        }
        drop(j);
        let j = open(&p).unwrap();
        assert_eq!(j.signature_reservation(1, 0, 1).unwrap().unwrap(), winner);
        drop(j);
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn missing_intent_and_rolled_back_migration_fence_never_fall_back() {
        for stage in ["Segment", "Commit"] {
            let p = seeded();
            child(&p, "append", stage);
            fs::remove_file(p.join(INTENT)).unwrap();
            assert!(open(&p).is_err());
            fs::remove_dir_all(p).unwrap();
        }
        let p = seeded();
        let j = open(&p).unwrap();
        j.reserve_signature(2, 0, 1, b"two").unwrap();
        drop(j);
        let mut binding = fs::read(p.join("validator-index.bin")).unwrap();
        binding[8] = 0;
        fs::write(p.join("validator-index.bin"), binding).unwrap();
        assert!(open(&p).is_err());
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn durable_counts_and_bounded_credit_audits_rebuild_on_restart() {
        let p = directory();
        let j = open(&p).unwrap();
        j.bind_validator_index(0).unwrap();
        for h in 1..=513 {
            j.reserve_signature(h, 0, 1, &[7; 158]).unwrap();
        }
        let stats = j.signing_stats().unwrap().unwrap();
        assert_eq!(stats.reservations, 513);
        assert_eq!(stats.preimage_bytes, 513 * 158);
        assert!(j.accounting_audits().unwrap() < 30);
        let actual = j.storage_footprint(100).unwrap();
        {
            let cache = j.accounting.lock().unwrap();
            let c = cache.footprint.as_ref().unwrap();
            assert!(c.bytes >= actual.bytes);
            assert!(c.allocated_bytes >= actual.allocated_bytes);
        }
        drop(j);
        let j = open(&p).unwrap();
        assert_eq!(
            j.signing_stats().unwrap().unwrap().preimage_bytes,
            513 * 158
        );
        j.ensure_capacity(0).unwrap();
        assert_eq!(
            j.accounting
                .lock()
                .unwrap()
                .footprint
                .as_ref()
                .unwrap()
                .bytes,
            j.storage_footprint(100).unwrap().bytes
        );
        drop(j);
        fs::remove_dir_all(p).unwrap();
    }
}
