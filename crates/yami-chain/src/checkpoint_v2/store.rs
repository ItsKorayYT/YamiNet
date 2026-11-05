//! Immutable dedicated inactive directories only. Never open a Journal/safety store.
//! Exact anchors are mandatory; local metadata is not rollback authority.
use super::*;
use std::{
    fs::{self, File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
const GUARD: &str = "inactive-checkpoint-v2.bin";
const GUARD_BYTES: &[u8] = b"YC-INACTIVE-CHECKPOINT2-STORE-1-NOT-CONSENSUS";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Publication {
    Synced,
    Renamed,
    DirectorySynced,
}
fn chunk_name(index: u32) -> String {
    format!("chunk-{index:06}.bin")
}
fn file_cap(name: &str) -> Option<usize> {
    let name = name.strip_prefix(".pending-").unwrap_or(name);
    match name {
        GUARD => Some(GUARD_BYTES.len()),
        // Retain the old-reader refusal fence as well as the new format guard.
        "lab.lock" => Some(0),
        "anchor.bin" | "state.bin" => Some(256),
        "manifest.bin" => Some(MAX_MANIFEST_BYTES),
        "complete.bin" => Some(32),
        _ => {
            let n = name
                .strip_prefix("chunk-")?
                .strip_suffix(".bin")?
                .parse::<u32>()
                .ok()?;
            ((n as usize) < MAX_CHUNKS && name == chunk_name(n)).then_some(MAX_CHUNK_BYTES)
        }
    }
}
fn metadata(path: &Path) -> io::Result<fs::Metadata> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_file() {
        return Err(invalid("inactive entry must be a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if m.nlink() != 1 {
            return Err(invalid("inactive hardlink refused"));
        }
    }
    Ok(m)
}
fn read(path: &Path, cap: usize) -> io::Result<Vec<u8>> {
    if metadata(path)?.len() > cap as u64 {
        return Err(invalid("inactive file cap"));
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(cap as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > cap {
        return Err(invalid("inactive file grew"));
    }
    Ok(bytes)
}
struct Store {
    path: PathBuf,
    _lock: File,
}
impl Store {
    fn inspect(path: &Path) -> io::Result<Vec<String>> {
        let mut names = Vec::new();
        for e in fs::read_dir(path)? {
            if names.len() >= MAX_CHUNKS * 2 + 12 {
                return Err(invalid("inactive directory work bound"));
            }
            let e = e?;
            let name = e
                .file_name()
                .into_string()
                .map_err(|_| invalid("inactive filename"))?;
            let cap = file_cap(&name).ok_or_else(|| invalid("not a dedicated inactive store"))?;
            if metadata(&e.path())?.len() > cap as u64 {
                return Err(invalid("inactive file bounds"));
            }
            names.push(name);
        }
        Ok(names)
    }
    fn open(path: &Path) -> io::Result<Self> {
        fs::create_dir_all(path)?;
        if !fs::symlink_metadata(path)?.is_dir() {
            return Err(invalid("inactive directory type"));
        }
        let entries = Self::inspect(path)?;
        if !entries.iter().any(|n| n == GUARD) && entries.iter().any(|n| n != "lab.lock") {
            return Err(invalid("inactive guard missing"));
        }
        if path.join(GUARD).exists() {
            let bytes = read(&path.join(GUARD), GUARD_BYTES.len())?;
            if !GUARD_BYTES.starts_with(&bytes)
                || (bytes != GUARD_BYTES && entries.iter().any(|n| n != GUARD && n != "lab.lock"))
            {
                return Err(invalid("inactive guard conflict"));
            }
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.join("lab.lock"))?;
        lock.try_lock()
            .map_err(|e| io::Error::other(format!("inactive store in use: {e}")))?;
        Self::inspect(path)?;
        let store = Self {
            path: path.to_owned(),
            _lock: lock,
        };
        store.prefix(GUARD, GUARD_BYTES)?;
        store.sync_dir()?;
        #[cfg(unix)]
        for p in fs::canonicalize(path)?.ancestors().skip(1) {
            File::open(p)?.sync_all()?;
        }
        Ok(store)
    }
    fn open_read(path: &Path) -> io::Result<Self> {
        if !fs::symlink_metadata(path)?.is_dir() {
            return Err(invalid("inactive directory type"));
        }
        Self::inspect(path)?;
        if read(&path.join(GUARD), GUARD_BYTES.len())? != GUARD_BYTES {
            return Err(invalid("inactive guard"));
        }
        let lock = File::open(path.join("lab.lock"))?;
        lock.try_lock_shared()
            .map_err(|e| io::Error::other(format!("inactive store writer active: {e}")))?;
        Ok(Self {
            path: path.to_owned(),
            _lock: lock,
        })
    }
    fn sync_dir(&self) -> io::Result<()> {
        #[cfg(unix)]
        File::open(&self.path)?.sync_all()?;
        Ok(())
    }
    fn prefix(&self, name: &str, expected: &[u8]) -> io::Result<()> {
        if expected.len() > file_cap(name).ok_or_else(|| invalid("inactive namespace"))? {
            return Err(invalid("inactive publication cap"));
        }
        let path = self.path.join(name);
        if path.try_exists()? {
            metadata(&path)?;
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let len = file.metadata()?.len();
        if len > expected.len() as u64 {
            return Err(invalid("inactive unknown tail"));
        }
        let mut prefix = vec![0; len as usize];
        file.read_exact(&mut prefix)?;
        if prefix != expected[..prefix.len()] {
            return Err(invalid("inactive conflicting prefix"));
        }
        file.seek(SeekFrom::Start(len))?;
        file.write_all(&expected[prefix.len()..])?;
        file.sync_all()
    }
    fn put(
        &self,
        name: &str,
        bytes: &[u8],
        observe: &mut impl FnMut(&str, Publication),
    ) -> io::Result<()> {
        let dest = self.path.join(name);
        let pending = format!(".pending-{name}");
        if dest.try_exists()? {
            if self.path.join(&pending).try_exists()? || read(&dest, bytes.len())? != bytes {
                return Err(invalid("inactive immutable destination conflict"));
            }
            File::open(dest)?.sync_all()?;
            return self.sync_dir();
        }
        self.prefix(&pending, bytes)?;
        observe(name, Publication::Synced);
        fs::rename(self.path.join(pending), dest)?;
        observe(name, Publication::Renamed);
        self.sync_dir()?;
        observe(name, Publication::DirectorySynced);
        Ok(())
    }
    fn namespace(&self, m: &Manifest) -> io::Result<()> {
        for name in Self::inspect(&self.path)? {
            let base = name.strip_prefix(".pending-").unwrap_or(&name);
            if let Some(n) = base
                .strip_prefix("chunk-")
                .and_then(|s| s.strip_suffix(".bin"))
            {
                let i = n.parse::<usize>().map_err(invalid)?;
                let c = m
                    .chunks
                    .get(i)
                    .ok_or_else(|| invalid("unreferenced inactive chunk"))?;
                let size = metadata(&self.path.join(name))?.len();
                if size > u64::from(c.bytes) {
                    return Err(invalid("inactive chunk tail"));
                }
            }
        }
        Ok(())
    }
    fn recover(
        &self,
        genesis: Genesis,
        expected: Anchor,
        require_complete: bool,
    ) -> io::Result<InactiveState> {
        if read(&self.path.join("state.bin"), 256)? != expected.state.canonical()
            || read(&self.path.join("anchor.bin"), 256)? != expected.canonical()
        {
            return Err(invalid("inactive exact pinned transport context"));
        }
        let manifest = Manifest::decode(
            &read(&self.path.join("manifest.bin"), MAX_MANIFEST_BYTES)?,
            expected,
        )?;
        self.namespace(&manifest)?;
        if require_complete
            && Self::inspect(&self.path)?
                .iter()
                .any(|name| name.starts_with(".pending-"))
        {
            return Err(invalid("uncommitted inactive tail"));
        }
        if require_complete && read(&self.path.join("complete.bin"), 32)? != expected.manifest.0 {
            return Err(invalid("inactive completion witness"));
        }
        import(genesis, expected, &manifest, |i| {
            let p = self.path.join(chunk_name(i));
            metadata(&p)?;
            File::open(p)
        })
    }
}
pub fn save(state: &InactiveState, path: &Path) -> io::Result<Anchor> {
    save_observed(state, path, |_, _| {})
}
pub fn save_observed(
    state: &InactiveState,
    path: &Path,
    mut observe: impl FnMut(&str, Publication),
) -> io::Result<Anchor> {
    let store = Store::open(path)?;
    store.put("state.bin", &state.anchor().canonical(), &mut observe)?;
    // Source state is immutable. An interrupted export has no published manifest;
    // exact chunk prefixes can be resumed from the same verified source state.
    let manifest = export(state, |i, b| store.put(&chunk_name(i), b, &mut observe))?;
    let expected = manifest.anchor();
    store.namespace(&manifest)?;
    store.put("anchor.bin", &expected.canonical(), &mut observe)?;
    store.put("manifest.bin", &manifest.canonical(), &mut observe)?;
    store.recover((*state.state.genesis).clone(), expected, false)?;
    store.put("complete.bin", &expected.manifest.0, &mut observe)?;
    Ok(expected)
}
pub fn recover(path: &Path, genesis: Genesis, expected: Anchor) -> io::Result<InactiveState> {
    let store = Store::open_read(path)?;
    store.recover(genesis, expected, true)
}
pub fn transfer(
    source: &Path,
    destination: &Path,
    genesis: Genesis,
    expected: Anchor,
) -> io::Result<InactiveState> {
    transfer_observed(source, destination, genesis, expected, |_, _| {})
}
pub fn transfer_observed(
    source: &Path,
    destination: &Path,
    genesis: Genesis,
    expected: Anchor,
    mut observe: impl FnMut(&str, Publication),
) -> io::Result<InactiveState> {
    // The source is read-only and may be an interrupted/hostile transfer. Do not
    // repair it, derive authority from its metadata, or return a partial state.
    if !fs::symlink_metadata(source)?.is_dir()
        || read(&source.join(GUARD), GUARD_BYTES.len())? != GUARD_BYTES
    {
        return Err(invalid("source inactive guard"));
    }
    let manifest = Manifest::decode(
        &read(&source.join("manifest.bin"), MAX_MANIFEST_BYTES)?,
        expected,
    )?;
    let store = Store::open(destination)?;
    store.namespace(&manifest)?;
    store.put("state.bin", &expected.state.canonical(), &mut observe)?;
    store.put("anchor.bin", &expected.canonical(), &mut observe)?;
    for i in 0..manifest.chunks.len() {
        let p = source.join(chunk_name(i as u32));
        metadata(&p)?;
        let bytes = manifest.read_chunk(i, File::open(p)?)?;
        store.put(&chunk_name(i as u32), &bytes, &mut observe)?;
    }
    store.put("manifest.bin", &manifest.canonical(), &mut observe)?;
    let state = store.recover(genesis, expected, false)?;
    store.put("complete.bin", &expected.manifest.0, &mut observe)?;
    Ok(state)
}
