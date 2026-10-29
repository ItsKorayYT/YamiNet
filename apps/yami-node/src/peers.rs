//! Manual bootstrap pins, never populated by discovery or remote metadata.
//! Status is the last attempt's history, not a live connection or trust bypass.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use yami_identity::NodeId;
use ynp::{Protection, Transport};
use ynp_tls::{Scope, TlsTransport};

pub const MAX_PEERS: usize = 256;
pub const MAX_FILE: usize = 64 * 1024;
const HEADER: &str = "YAMIPEERS\t1\n";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Unverified,
    Authenticated,
    Failed,
}
impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unverified => "unverified",
            Self::Authenticated => "authenticated",
            Self::Failed => "failed",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    pub id: NodeId,
    pub address: SocketAddr,
    /// Unix seconds; updated only after a fresh pinned TLS handshake succeeds.
    pub last_success: Option<u64>,
    pub status: Status,
}
pub struct PeerStore {
    dir: PathBuf,
    entries: Vec<Peer>,
    _lock: File,
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn endpoint(address: SocketAddr) -> io::Result<()> {
    Scope::Internet.check(address)?;
    if address.port() == 0 {
        return Err(invalid("peer port must be nonzero"));
    }
    Ok(())
}
pub fn default_dir() -> io::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("YAMINET_PEER_DIR") {
        return Ok(PathBuf::from(dir));
    }
    yami_identity::default_dir()
        .and_then(|dir| dir.parent().map(|p| p.join("peers")))
        .ok_or_else(|| invalid("no per-user peer directory; pass --dir <directory>"))
}
impl PeerStore {
    /// Exclusive nonblocking OS lock, held through a single command/connection.
    /// Concurrent commands fail clearly rather than lose updates. No stale lock
    /// after process exit: locking follows the file handle, not file existence.
    pub fn open(dir: impl AsRef<Path>) -> io::Result<Self> {
        let dir = dir.as_ref();
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(dir)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let lock = options.open(dir.join("peers.lock"))?;
        lock.try_lock().map_err(|e| {
            io::Error::other(format!("peer configuration is locked or unavailable: {e}"))
        })?;
        let path = dir.join("peers.tsv");
        let entries = match File::open(&path) {
            Ok(file) => {
                if !file.metadata()?.is_file() {
                    return Err(invalid("peers.tsv must be a regular file"));
                }
                let mut text = String::new();
                file.take((MAX_FILE + 1) as u64).read_to_string(&mut text)?;
                parse(&text)?
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        Ok(Self {
            dir: dir.into(),
            entries,
            _lock: lock,
        })
    }
    pub fn entries(&self) -> &[Peer] {
        &self.entries
    }
    pub fn add(&mut self, id: NodeId, address: SocketAddr) -> io::Result<()> {
        endpoint(address)?;
        if self.entries.iter().any(|peer| peer.id == id) {
            return Err(invalid(
                "peer ID already configured; remove it explicitly before changing its endpoint",
            ));
        }
        if self.entries.len() >= MAX_PEERS {
            return Err(invalid("peer limit is 256"));
        }
        self.entries.push(Peer {
            id,
            address,
            last_success: None,
            status: Status::Unverified,
        });
        let result = self.save();
        if result.is_err() {
            self.entries.pop();
        }
        result
    }
    pub fn remove(&mut self, id: NodeId) -> io::Result<()> {
        let index = self.index(id)?;
        let peer = self.entries.remove(index);
        let result = self.save();
        if result.is_err() {
            self.entries.insert(index, peer);
        }
        result
    }
    fn index(&self, id: NodeId) -> io::Result<usize> {
        self.entries
            .iter()
            .position(|peer| peer.id == id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "peer ID is not configured"))
    }
    /// Authenticate only; never downloads a page, starts a retry loop, follows
    /// redirects, learns another peer, or replaces a configured identity.
    pub fn connect(&mut self, id: NodeId) -> io::Result<()> {
        let index = self.index(id)?;
        let previous = self.entries[index].clone();
        let peer = &self.entries[index];
        let result = TlsTransport::new(peer.id)?
            .connect(peer.address)
            .and_then(|connection| {
                if connection.protection() != Protection::Tls13PinnedServer {
                    return Err(invalid("connection did not authenticate TLS 1.3"));
                }
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|time| time.as_secs())
                    .map_err(io::Error::other)
            });
        let outcome = match result {
            Ok(time) => {
                self.entries[index].last_success = Some(time);
                self.entries[index].status = Status::Authenticated;
                Ok(())
            }
            Err(error) => {
                self.entries[index].status = Status::Failed;
                Err(error)
            }
        };
        // Persisted status is historical and must remain consistent if an
        // atomic replacement fails. The next command always authenticates anew.
        if let Err(error) = self.save() {
            self.entries[index] = previous;
            return Err(error);
        }
        outcome
    }
    fn save(&self) -> io::Result<()> {
        let mut text = String::from(HEADER);
        for peer in &self.entries {
            text.push_str(&format!(
                "{}\t{}\t{}\t{}\n",
                peer.id,
                peer.address,
                peer.last_success
                    .map_or_else(|| "-".into(), |s| s.to_string()),
                peer.status.label()
            ));
        }
        if text.len() > MAX_FILE {
            return Err(invalid("peer configuration exceeds file budget"));
        }
        let temp = self.dir.join(format!("peers-{}.tmp", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&temp)?;
        let result = file
            .write_all(text.as_bytes())
            .and_then(|()| file.sync_all());
        drop(file);
        let result = result.and_then(|()| fs::rename(&temp, self.dir.join("peers.tsv")));
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
}
fn parse(text: &str) -> io::Result<Vec<Peer>> {
    if text.len() > MAX_FILE || !text.starts_with(HEADER) || !text.ends_with('\n') {
        return Err(invalid(
            "invalid or oversized peer configuration (expected YAMIPEERS version 1)",
        ));
    }
    let mut peers: Vec<Peer> = Vec::new();
    for line in text[HEADER.len()..].lines() {
        if peers.len() >= MAX_PEERS || line.len() > 160 {
            return Err(invalid("peer configuration exceeds limits"));
        }
        let fields: Vec<_> = line.split('\t').collect();
        if fields.len() != 4 {
            return Err(invalid("peer record requires exactly four fields"));
        }
        let id: NodeId = fields[0].parse().map_err(io::Error::other)?;
        let address: SocketAddr = fields[1].parse().map_err(io::Error::other)?;
        endpoint(address)?;
        if address.to_string() != fields[1] {
            return Err(invalid("peer endpoint must be canonical numeric IP:port"));
        }
        let last_success = if fields[2] == "-" {
            None
        } else {
            Some(fields[2].parse::<u64>().map_err(io::Error::other)?)
        };
        let status = match fields[3] {
            "unverified" => Status::Unverified,
            "authenticated" => Status::Authenticated,
            "failed" => Status::Failed,
            _ => return Err(invalid("unknown peer status")),
        };
        if (status == Status::Unverified && last_success.is_some())
            || (status == Status::Authenticated && last_success.is_none())
            || peers.iter().any(|p| p.id == id)
        {
            return Err(invalid("inconsistent or duplicate peer record"));
        }
        peers.push(Peer {
            id,
            address,
            last_success,
            status,
        });
    }
    Ok(peers)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id() -> NodeId {
        *yami_identity::NodeIdentity::generate().unwrap().id()
    }
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("ynp-peers-{name}-{}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn persistent_manual_peers_lock_and_atomic_updates() {
        let dir = Scratch::new("roundtrip");
        let node = id();
        let mut peers = PeerStore::open(&dir.0).unwrap();
        peers.add(node, "8.8.8.8:7443".parse().unwrap()).unwrap();
        assert_eq!(peers.entries()[0].status, Status::Unverified);
        assert_eq!(peers.entries()[0].last_success, None);
        assert!(
            PeerStore::open(&dir.0).is_err(),
            "exclusive nonblocking lock"
        );
        assert!(
            peers.add(node, "1.1.1.1:7443".parse().unwrap()).is_err(),
            "never silently replace a pin/endpoint"
        );
        drop(peers);
        let mut peers = PeerStore::open(&dir.0).unwrap();
        assert_eq!(peers.entries()[0].id, node);
        assert_eq!(peers.entries()[0].address, "8.8.8.8:7443".parse().unwrap());
        let prior = fs::read(dir.0.join("peers.tsv")).unwrap();
        let temp = dir.0.join(format!("peers-{}.tmp", std::process::id()));
        fs::write(&temp, b"do not overwrite").unwrap();
        assert!(
            peers.remove(node).is_err(),
            "staging collision fails closed"
        );
        assert_eq!(peers.entries().len(), 1, "failed saves roll back memory");
        assert_eq!(fs::read(dir.0.join("peers.tsv")).unwrap(), prior);
        fs::remove_file(temp).unwrap();
        peers.remove(node).unwrap();
        drop(peers);
        assert!(PeerStore::open(&dir.0).unwrap().entries().is_empty());
    }
    #[test]
    fn malformed_peer_files_and_addresses_fail_closed() {
        let node = id();
        let good = format!("{HEADER}{node}\t127.0.0.1:7443\t-\tunverified\n");
        assert_eq!(parse(&good).unwrap().len(), 1);
        for bad in [
            String::new(),
            good.replace("YAMIPEERS\t1", "YAMIPEERS\t2"),
            good.trim_end().into(),
            good.replace("unverified", "trusted"),
            good.replace("unverified", "authenticated"),
            good.replace("\t-\t", "\t1\t"),
            format!("{good}{node}\t127.0.0.1:7443\t-\tunverified\n"),
            good.replace("127.0.0.1:7443", "host:7443"),
            good.replace("127.0.0.1:7443", "0.0.0.0:7443"),
            good.replace("127.0.0.1:7443", "8.8.8.8:0"),
            "x".repeat(MAX_FILE + 1),
        ] {
            assert!(parse(&bad).is_err());
        }
        let dir = Scratch::new("malformed");
        fs::write(dir.0.join("peers.tsv"), "bad configuration").unwrap();
        assert!(PeerStore::open(&dir.0).is_err());
        assert_eq!(
            fs::read_to_string(dir.0.join("peers.tsv")).unwrap(),
            "bad configuration"
        );
        let too_many = format!(
            "{HEADER}{}",
            format!("{node}\t127.0.0.1:7443\t-\tunverified\n").repeat(MAX_PEERS + 1)
        );
        assert!(parse(&too_many).is_err());
    }
    #[test]
    fn full_valid_peer_configuration_refuses_additional_entries() {
        let dir = Scratch::new("limit");
        let mut text = String::from(HEADER);
        for _ in 0..MAX_PEERS {
            text.push_str(&format!("{}\t127.0.0.1:7443\t-\tunverified\n", id()));
        }
        fs::write(dir.0.join("peers.tsv"), &text).unwrap();
        let mut peers = PeerStore::open(&dir.0).unwrap();
        assert_eq!(peers.entries().len(), MAX_PEERS);
        assert!(peers.add(id(), "127.0.0.1:7443".parse().unwrap()).is_err());
        assert_eq!(fs::read_to_string(dir.0.join("peers.tsv")).unwrap(), text);
    }
}
