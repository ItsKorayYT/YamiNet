//! Private content-defined encrypted snapshots. The manifest is authenticated by
//! the existing local anchor/witness receipt, not by an untrusted chunk database.
//! No account identifiers, balances, amounts or plaintext content hashes are stored.
use crate::*;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{BTreeMap, BTreeSet};

const FULL: &[u8; 8] = b"YCFULL01";
const MANIFEST: &[u8; 8] = b"YCMANF01";
const DOMAIN: &[u8] = b"YAMI-financial-encrypted-chunk-v1";
pub(crate) const MAX_SNAPSHOT: usize = 4 * 1024 * 1024;
const MAX_CHUNKS: usize = 2048;
const MIN_CHUNK: usize = 2048;
const MAX_CHUNK: usize = 32768;

#[derive(Clone)]
pub(crate) struct Snapshot {
    pub(crate) chunks: Vec<Vec<u8>>,
}
fn u32_at(raw: &[u8], at: &mut usize) -> Result<usize> {
    let end = at.checked_add(4).ok_or("snapshot offset")?;
    let bytes: [u8; 4] = raw.get(*at..end).ok_or("truncated snapshot")?.try_into()?;
    *at = end;
    Ok(u32::from_le_bytes(bytes) as usize)
}
impl Snapshot {
    pub(crate) fn is_full(raw: &[u8]) -> bool {
        raw.starts_with(FULL)
    }
    pub(crate) fn is_manifest(raw: &[u8]) -> bool {
        raw.starts_with(MANIFEST)
    }
    pub(crate) fn parse(raw: &[u8]) -> Result<Self> {
        check(Self::is_full(raw) && raw.len() <= MAX_SNAPSHOT)?;
        let mut at = 8;
        let count = u32_at(raw, &mut at)?;
        check((1..=MAX_CHUNKS).contains(&count))?;
        let mut chunks = Vec::with_capacity(count);
        for _ in 0..count {
            let size = u32_at(raw, &mut at)?;
            check((28..=MAX_CHUNK + 28).contains(&size))?;
            let end = at.checked_add(size).ok_or("chunk offset")?;
            chunks.push(raw.get(at..end).ok_or("truncated chunk")?.to_vec());
            at = end;
        }
        check(at == raw.len())?;
        Ok(Self { chunks })
    }
    pub(crate) fn pack(&self) -> Result<Vec<u8>> {
        check((1..=MAX_CHUNKS).contains(&self.chunks.len()))?;
        let mut out = FULL.to_vec();
        out.extend_from_slice(&(self.chunks.len() as u32).to_le_bytes());
        for chunk in &self.chunks {
            out.extend_from_slice(&(u32::try_from(chunk.len())?).to_le_bytes());
            out.extend_from_slice(chunk);
        }
        check(out.len() <= MAX_SNAPSHOT)?;
        Ok(out)
    }
    pub(crate) fn manifest(&self) -> Vec<u8> {
        let mut out = MANIFEST.to_vec();
        out.extend_from_slice(&(self.chunks.len() as u32).to_le_bytes());
        for chunk in &self.chunks {
            out.extend_from_slice(&hash(chunk));
        }
        out
    }
    fn ids(raw: &[u8]) -> Result<Vec<Id>> {
        check(Self::is_manifest(raw))?;
        let mut at = 8;
        let count = u32_at(raw, &mut at)?;
        check((1..=MAX_CHUNKS).contains(&count) && raw.len() == 12 + count * 32)?;
        raw[12..]
            .chunks_exact(32)
            .map(|id| Ok(id.try_into()?))
            .collect()
    }
    fn decrypt_chunk(raw: &[u8], key: Id) -> Result<Vec<u8>> {
        check((28..=MAX_CHUNK + 28).contains(&raw.len()))?;
        open(
            key,
            &Envelope {
                ephemeral: [0; 32],
                nonce: raw[..12].try_into()?,
                ciphertext: raw[12..].to_vec(),
            },
            DOMAIN,
        )
    }
    pub(crate) fn plaintext(&self, key: Id) -> Result<Vec<u8>> {
        let mut raw = Vec::new();
        for chunk in &self.chunks {
            raw.extend_from_slice(&Self::decrypt_chunk(chunk, key)?);
            check(raw.len() <= MAX_SNAPSHOT)?;
        }
        Ok(raw)
    }
    pub(crate) fn build(raw: &[u8], key: Id, previous: Option<&Self>) -> Result<Self> {
        check(!raw.is_empty() && raw.len() <= MAX_SNAPSHOT)?;
        let mut reused = BTreeMap::new();
        if let Some(previous) = previous {
            for chunk in &previous.chunks {
                let plain = Self::decrypt_chunk(chunk, key)?;
                reused.insert(hash(&plain), chunk.clone());
            }
        }
        // Secret-dependent boundaries avoid publishing a dictionary of plaintext
        // boundary decisions. The rolling hash forgets insertions after 64 bytes.
        let mut table = [0u8; 2048];
        Hkdf::<Sha256>::new(Some(b"YAMI-private-chunk-boundaries-v1"), &key)
            .expand(b"gear", &mut table)
            .map_err(|_| "chunk boundary key")?;
        let mut chunks = Vec::new();
        let mut start = 0;
        let mut rolling = 0u64;
        for (at, byte) in raw.iter().enumerate() {
            let offset = usize::from(*byte) * 8;
            let gear = u64::from_le_bytes(table[offset..offset + 8].try_into()?);
            rolling = rolling.wrapping_shl(1).wrapping_add(gear);
            let size = at + 1 - start;
            if size >= MAX_CHUNK
                || (size >= MIN_CHUNK && rolling & 8191 == 0)
                || at + 1 == raw.len()
            {
                let plain = &raw[start..=at];
                let digest = hash(plain);
                let encrypted = if let Some(old) = reused.get(&digest) {
                    old.clone()
                } else {
                    let envelope = seal(key, plain, DOMAIN)?;
                    let mut raw = envelope.nonce.to_vec();
                    raw.extend_from_slice(&envelope.ciphertext);
                    raw
                };
                reused.insert(digest, encrypted.clone());
                chunks.push(encrypted);
                start = at + 1;
                rolling = 0;
            }
        }
        let snapshot = Self { chunks };
        snapshot.pack()?;
        Ok(snapshot)
    }
    pub(crate) fn create_table(connection: &Connection) -> Result<()> {
        connection.execute_batch("CREATE TABLE IF NOT EXISTS financial_chunks(id BLOB PRIMARY KEY, payload BLOB NOT NULL); CREATE TABLE IF NOT EXISTS financial_manifest(id INTEGER PRIMARY KEY CHECK(id=1), payload BLOB NOT NULL); CREATE TABLE IF NOT EXISTS financial_stats(id INTEGER PRIMARY KEY CHECK(id=1), chunk_bytes INTEGER NOT NULL, manifest_bytes INTEGER NOT NULL);")?;
        Ok(())
    }
    pub(crate) fn load(connection: &Connection, manifest: &[u8]) -> Result<Self> {
        let ids = Self::ids(manifest)?;
        let mut chunks = Vec::with_capacity(ids.len());
        let mut total = 12usize;
        for id in ids {
            let chunk: Vec<u8> = connection.query_row(
                "SELECT payload FROM financial_chunks WHERE id=?1",
                params![id.as_slice()],
                |r| r.get(0),
            )?;
            check(hash(&chunk) == id && (28..=MAX_CHUNK + 28).contains(&chunk.len()))?;
            total = total
                .checked_add(chunk.len() + 4)
                .ok_or("snapshot size overflow")?;
            check(total <= MAX_SNAPSHOT)?;
            chunks.push(chunk);
        }
        Ok(Self { chunks })
    }
    /// Caller owns the SQLite transaction. Snapshot chunks and the authority's
    /// revision, nonce, vote lock and small state record commit together.
    pub(crate) fn store(
        &self,
        connection: &Connection,
        old_manifest: Option<&[u8]>,
    ) -> Result<u64> {
        let keep: BTreeSet<_> = self.chunks.iter().map(|c| hash(c)).collect();
        let mut written = 0;
        for chunk in &self.chunks {
            let changed = connection.execute(
                "INSERT OR IGNORE INTO financial_chunks VALUES(?1,?2)",
                params![hash(chunk).as_slice(), chunk],
            )?;
            if changed == 0 {
                let stored: Vec<u8> = connection.query_row(
                    "SELECT payload FROM financial_chunks WHERE id=?1",
                    params![hash(chunk).as_slice()],
                    |r| r.get(0),
                )?;
                check(stored == *chunk)?;
            }
            written += (changed * chunk.len()) as u64;
        }
        if let Some(old) = old_manifest {
            for id in Self::ids(old)?
                .into_iter()
                .collect::<BTreeSet<_>>()
                .difference(&keep)
            {
                connection.execute(
                    "DELETE FROM financial_chunks WHERE id=?1",
                    params![id.as_slice()],
                )?;
            }
        }
        connection.execute(
            "INSERT OR REPLACE INTO financial_manifest VALUES(1,?1)",
            params![self.manifest()],
        )?;
        connection.execute("INSERT INTO financial_stats VALUES(1,?1,?2) ON CONFLICT(id) DO UPDATE SET chunk_bytes=chunk_bytes+excluded.chunk_bytes, manifest_bytes=manifest_bytes+excluded.manifest_bytes",params![written,self.manifest().len() as u64])?;
        Ok(written)
    }
    pub(crate) fn retain(&self, connection: &Connection) -> Result<()> {
        let keep: BTreeSet<_> = self.chunks.iter().map(|c| hash(c)).collect();
        let ids: Vec<Vec<u8>> = connection
            .prepare("SELECT id FROM financial_chunks LIMIT 4097")?
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        check(ids.len() <= MAX_CHUNKS * 2)?;
        for id in ids {
            let key: Id = id.as_slice().try_into()?;
            if !keep.contains(&key) {
                connection.execute("DELETE FROM financial_chunks WHERE id=?1", params![id])?;
            }
        }
        Ok(())
    }
    pub(crate) fn stored_manifest(connection: &Connection) -> Result<Option<Vec<u8>>> {
        Ok(connection
            .query_row(
                "SELECT payload FROM financial_manifest WHERE id=1",
                [],
                |r| r.get(0),
            )
            .optional()?)
    }
}

pub(crate) fn sidecar(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".chunks.sqlite");
    PathBuf::from(name)
}
pub(crate) fn connect(path: &Path) -> Result<Connection> {
    let connection = Connection::open(sidecar(path))?;
    connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA wal_autocheckpoint=256; PRAGMA journal_size_limit=2097152;")?;
    let page_size: u64 = connection.query_row("PRAGMA page_size", [], |r| r.get(0))?;
    let pages = 12 * 1024 * 1024 / page_size;
    let maximum: u64 =
        connection.query_row(&format!("PRAGMA max_page_count={pages}"), [], |r| r.get(0))?;
    check(maximum <= pages)?;
    Snapshot::create_table(&connection)?;
    Ok(connection)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn large() -> serde_json::Value {
        serde_json::json!((0u32..10000)
            .map(|i| (
                hash(&i.to_le_bytes()),
                Account {
                    balance: 123456789,
                    nonce: 0
                }
            ))
            .collect::<Vec<_>>())
    }
    #[test]
    fn changing_two_accounts_reuses_authenticated_chunks_and_bounds_live_storage() {
        let mut value = large();
        let original = bytes(&value);
        let first = Snapshot::build(&original, [7; 32], None).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut connection = connect(&dir.path().join("financial-cache")).unwrap();
        let tx = connection.transaction().unwrap();
        let initial = first.store(&tx, None).unwrap();
        tx.commit().unwrap();
        value[1024][1]["balance"] = serde_json::json!(123456788);
        value[8192][1]["balance"] = serde_json::json!(123456790);
        let next = Snapshot::build(&bytes(&value), [7; 32], Some(&first)).unwrap();
        let tx = connection.transaction().unwrap();
        let changed = next.store(&tx, Some(&first.manifest())).unwrap();
        tx.commit().unwrap();
        assert!(
            changed < initial / 4,
            "changed={changed}, initial={initial}"
        );
        assert_eq!(
            Snapshot::load(&connection, &next.manifest())
                .unwrap()
                .plaintext([7; 32])
                .unwrap(),
            bytes(&value)
        );
        assert!(next.plaintext([8; 32]).is_err());
        let mut corrupt = next.pack().unwrap();
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(Snapshot::parse(&corrupt)
            .unwrap()
            .plaintext([7; 32])
            .is_err());
        let size: u64 = connection
            .query_row(
                "SELECT sum(length(payload)) FROM financial_chunks",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(size < original.len() as u64 + 64 * 1024);
        let manifest = next.manifest();
        drop(connection);
        let connection = connect(&dir.path().join("financial-cache")).unwrap();
        assert_eq!(
            Snapshot::load(&connection, &manifest)
                .unwrap()
                .pack()
                .unwrap(),
            next.pack().unwrap()
        );
        let removed = hash(&next.chunks[0]);
        connection
            .execute(
                "DELETE FROM financial_chunks WHERE id=?1",
                params![removed.as_slice()],
            )
            .unwrap();
        assert!(Snapshot::load(&connection, &manifest).is_err());
        println!("large private snapshot bytes={}, initial encrypted bytes={initial}, bytes changed for two accounts={changed}",original.len());
    }
    #[test]
    fn malformed_truncated_and_oversized_checkpoints_fail_closed() {
        let snapshot = Snapshot::build(b"private ledger", [1; 32], None).unwrap();
        let raw = snapshot.pack().unwrap();
        for end in 0..raw.len() {
            assert!(Snapshot::parse(&raw[..end]).is_err());
        }
        let mut extra = raw.clone();
        extra.push(0);
        assert!(Snapshot::parse(&extra).is_err());
        let mut count = raw;
        count[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Snapshot::parse(&count).is_err());
    }
}
