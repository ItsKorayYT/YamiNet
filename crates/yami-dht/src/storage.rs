use crate::{
    invalid,
    wire::{self, Message},
    Clock, Contact, Network, PeerRecord, State, SystemClock, MAX_CACHE, MAX_RECORD_TTL,
};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    net::SocketAddrV4,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};
use yami_identity::{NodeId, NodeIdentity};
use ynp::{Connection, Protection};
const MAX_SNAPSHOT: usize = 64 * 1024;
struct Disk {
    state: State,
    path: PathBuf,
    _lock: File,
    faulted: bool,
}
#[derive(Clone)]
pub struct Service {
    disk: Arc<Mutex<Disk>>,
    clock: Arc<dyn Clock>,
}
impl Service {
    /// Open a lookup-only cache without a client signing key. `local` is only
    /// a routing origin/cache owner, never sent as an authenticated identity.
    /// Keep its public ID across restarts; discard a new origin's private key.
    pub fn open_lookup(path: impl AsRef<Path>, network: Network) -> io::Result<Self> {
        let path = path.as_ref();
        let local = match read_snapshot(path, network) {
            Ok(state) => state.local,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                *NodeIdentity::generate().map_err(io::Error::other)?.id()
            }
            Err(error) => return Err(error),
        };
        Self::open(path, local, network)
    }
    pub fn open(path: impl AsRef<Path>, local: NodeId, network: Network) -> io::Result<Self> {
        Self::open_with_clock(path, local, network, Arc::new(SystemClock))
    }
    pub fn open_with_clock(
        path: impl AsRef<Path>,
        local: NodeId,
        network: Network,
        clock: Arc<dyn Clock>,
    ) -> io::Result<Self> {
        let path = path.as_ref();
        let parent = path
            .parent()
            .ok_or_else(|| invalid("DHT snapshot requires a directory"))?;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(parent)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let lock = options.open(path.with_extension("lock"))?;
        lock.try_lock()
            .map_err(|e| invalid(format!("DHT state is locked: {e}")))?;
        // The exclusive writer lock proves no live writer owns this staging
        // file. A crash before rename must not block all later renewals.
        match fs::remove_file(path.with_extension("tmp")) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let state = match read_snapshot(path, network) {
            Ok(state) if state.local == local => state,
            Ok(_) => return Err(invalid("DHT cache belongs to another local identity")),
            Err(e) if e.kind() == io::ErrorKind::NotFound => State::new(local, network),
            Err(e) => return Err(e),
        };
        Ok(Self {
            clock,
            disk: Arc::new(Mutex::new(Disk {
                state,
                path: path.into(),
                _lock: lock,
                faulted: false,
            })),
        })
    }
    fn guard(&self) -> io::Result<MutexGuard<'_, Disk>> {
        let disk = self
            .disk
            .lock()
            .map_err(|_| invalid("DHT state poisoned"))?;
        if disk.faulted {
            return Err(invalid(
                "DHT persistence failed; restart after repairing local storage",
            ));
        }
        Ok(disk)
    }
    pub fn now(&self) -> io::Result<u64> {
        self.clock.now()
    }
    /// Bounded signed candidates plus recently authenticated routes. Unverified
    /// manual pins are supplied separately and must still pass pinned TLS.
    pub fn contacts(&self, at: u64) -> io::Result<Vec<Contact>> {
        self.candidates(at)
    }
    pub fn snapshot(&self) -> io::Result<State> {
        Ok(self.guard()?.state.clone())
    }
    /// Read an atomically replaced file without taking a writer's lifetime lock.
    pub fn read_snapshot(path: &Path, network: Network) -> io::Result<State> {
        read_snapshot(path, network)
    }
    pub fn accept(&self, record: PeerRecord, now: u64) -> io::Result<bool> {
        self.accept_many(vec![record], now)
    }
    pub fn accept_records(&self, records: Vec<PeerRecord>, now: u64) -> io::Result<bool> {
        if records.len() > MAX_CACHE {
            return Err(invalid("too many records to import"));
        }
        self.accept_many(records, now)
    }
    pub(crate) fn accept_many(&self, records: Vec<PeerRecord>, now: u64) -> io::Result<bool> {
        let mut disk = self.guard()?;
        let mut next = disk.state.clone();
        let mut changed = false;
        for record in records {
            changed |= next.accept(record, now)?;
        }
        if changed {
            disk.commit(next)?;
        }
        Ok(changed)
    }
    /// The next sequence is persisted atomically BEFORE this record is returned
    /// for publication. Expired self records retain the high-water mark.
    pub fn prepare_self(
        &self,
        identity: &NodeIdentity,
        endpoints: Vec<SocketAddrV4>,
        now: u64,
    ) -> io::Result<PeerRecord> {
        self.prepare_self_routes(identity, endpoints, Vec::new(), now)
    }
    pub fn prepare_self_routes(
        &self,
        identity: &NodeIdentity,
        endpoints: Vec<SocketAddrV4>,
        routes: Vec<crate::RelayRoute>,
        now: u64,
    ) -> io::Result<PeerRecord> {
        let mut disk = self.guard()?;
        if disk.state.local != *identity.id() {
            return Err(invalid("DHT signer is not this node"));
        }
        let previous = disk
            .state
            .records
            .iter()
            .find(|r| r.id() == *identity.id())
            .map_or(0, PeerRecord::sequence);
        let sequence = previous
            .checked_add(1)
            .ok_or_else(|| invalid("peer sequence exhausted"))?;
        let record = PeerRecord::sign_routes(
            identity,
            endpoints,
            routes,
            now.checked_add(MAX_RECORD_TTL)
                .ok_or_else(|| invalid("clock overflow"))?,
            sequence,
            disk.state.network,
            now,
        )?;
        let mut next = disk.state.clone();
        next.accept(record.clone(), now)?;
        disk.commit(next)?;
        Ok(record)
    }
    pub(crate) fn observed(&self, contact: Contact, now: u64) -> io::Result<()> {
        let mut disk = self.guard()?;
        let _ = disk.state.routing.observed(contact, now);
        Ok(())
    }
    pub(crate) fn failed(&self, contact: Contact) -> io::Result<()> {
        self.guard()?.state.routing.failed(contact);
        Ok(())
    }
    pub(crate) fn candidates(&self, now: u64) -> io::Result<Vec<Contact>> {
        Ok(self.guard()?.state.candidates(now))
    }
    /// One RPC per existing authenticated TLS connection. Unauthenticated clients
    /// may relay signed records, but cannot forge their owners' signatures.
    pub fn handle(&self, connection: &mut dyn Connection, prefix: [u8; 4]) -> io::Result<()> {
        if connection.protection() != Protection::Tls13PinnedServer {
            return Err(invalid(
                "DHT requires the authenticated encrypted TLS transport",
            ));
        }
        let (nonce, request) = wire::read_prefixed(connection, prefix)?;
        let now = self.now()?;
        let response = match request {
            Message::Find(target) => Message::Peers(self.snapshot()?.closest(target, now)),
            Message::Store(record) => {
                self.accept(record, now)?;
                Message::Stored
            }
            _ => return Err(invalid("DHT server accepts only FIND or STORE requests")),
        };
        wire::write(connection, nonce, &response)
    }
}
impl Disk {
    fn commit(&mut self, next: State) -> io::Result<()> {
        let mut installed = false;
        if let Err(error) = save(&self.path, &next, &mut installed) {
            // An error after rename may mean disk is ahead of memory. Fail
            // closed: never serve or reserve a conflicting sequence in-process.
            self.faulted = installed;
            return Err(error);
        }
        self.state = next;
        Ok(())
    }
}
fn save(path: &Path, state: &State, installed: &mut bool) -> io::Result<()> {
    let mut bytes = Vec::with_capacity(MAX_SNAPSHOT);
    bytes.extend_from_slice(b"YDC\x01");
    bytes.push(state.network.byte());
    bytes.extend_from_slice(&state.local.public_key());
    bytes.extend_from_slice(&(state.records.len() as u16).to_be_bytes());
    for record in &state.records {
        let record = record.encode();
        bytes.extend_from_slice(&(record.len() as u16).to_be_bytes());
        bytes.extend_from_slice(&record);
    }
    if bytes.len() > MAX_SNAPSHOT {
        return Err(invalid("DHT snapshot exceeds budget"));
    }
    let temp = path.with_extension("tmp");
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&temp)?;
    let result = file.write_all(&bytes).and_then(|()| file.sync_all());
    drop(file);
    let result = result
        .and_then(|()| fs::rename(&temp, path))
        .and_then(|()| {
            *installed = true;
            // Persist the directory entry before any signed record is advertised.
            #[cfg(unix)]
            File::open(
                path.parent()
                    .ok_or_else(|| invalid("missing snapshot parent"))?,
            )?
            .sync_all()?;
            Ok(())
        });
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
fn read_snapshot(path: &Path, network: Network) -> io::Result<State> {
    let file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid("DHT snapshot is not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take((MAX_SNAPSHOT + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() < 39
        || bytes.len() > MAX_SNAPSHOT
        || &bytes[..4] != b"YDC\x01"
        || bytes[4] != network.byte()
    {
        return Err(invalid("invalid DHT cache version, size or network policy"));
    }
    let local = NodeId::from_public_key(
        bytes[5..37]
            .try_into()
            .map_err(|_| invalid("cache owner"))?,
    )
    .map_err(io::Error::other)?;
    let count = u16::from_be_bytes([bytes[37], bytes[38]]) as usize;
    if count > MAX_CACHE {
        return Err(invalid("DHT cache count exceeds budget"));
    }
    let mut state = State::new(local, network);
    let mut offset = 39;
    for _ in 0..count {
        let length = bytes
            .get(offset..offset + 2)
            .ok_or_else(|| invalid("truncated cached record length"))?;
        let length = u16::from_be_bytes([length[0], length[1]]) as usize;
        offset += 2;
        let record = PeerRecord::decode(
            bytes
                .get(offset..offset + length)
                .ok_or_else(|| invalid("truncated cached record"))?,
        )?;
        record.verify_signature(network)?;
        if state.records.iter().any(|r| r.id() == record.id()) {
            return Err(invalid("duplicate cache owner"));
        }
        // Expired signed records restore ONLY anti-rollback history, never live
        // routing/location information. Every live use checks its expiry again.
        state.records.push(record);
        offset += length;
    }
    if offset != bytes.len() {
        return Err(invalid("trailing DHT cache data"));
    }
    Ok(state)
}
