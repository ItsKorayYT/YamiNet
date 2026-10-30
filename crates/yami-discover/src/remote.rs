//! Authenticated remote directory retrieval.
//!
//! Remote sources are fetched only on an explicit refresh. Resolution reuses
//! `yami-dht` signed records and bootstrap contacts; transport reuses
//! `ynp-tls` with exact node-ID pinning and `ynp` document framing. This module
//! does not implement a TLS client, DHT resolver or YNP codec, and it never
//! falls back to plaintext.
//!
//! TLS authentication proves which node delivered a document. It does **not**
//! make the directory content or listed websites trustworthy.
use crate::{Directory, WebsiteAddress};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Read},
    net::SocketAddr,
    path::Path,
};
use yami_dht::{Contact, Network, PeerRecord};
use yami_identity::NodeId;
use ynp::Protection;
use ynp_tls::{IdentityMismatch, TlsTransport};

const PEER_HEADER: &str = "YAMIPEERS\t1\n";
const MAX_PEER_FILE: usize = 64 * 1024;
const MAX_PEER_LINE: usize = 160;
const MAX_PEERS: usize = 256;

/// Failure category for an explicit remote directory refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteFetchErrorKind {
    /// Invalid local fetcher or bootstrap configuration.
    Configuration,
    /// Signed DHT lookup failed or returned no usable record.
    Lookup,
    /// The node's advertised endpoints were unreachable.
    Unreachable,
    /// The endpoint did not prove the exact expected node identity.
    Identity,
    /// Authenticated transport or document framing failed.
    Transport,
    /// The delivered document is not a valid version-1 directory.
    InvalidDirectory,
}

impl RemoteFetchErrorKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Configuration => "configuration error",
            Self::Lookup => "DHT lookup failed",
            Self::Unreachable => "node unreachable",
            Self::Identity => "identity mismatch",
            Self::Transport => "transport error",
            Self::InvalidDirectory => "invalid directory",
        }
    }
}

/// A bounded, privacy-conscious remote refresh error.
///
/// Messages must not contain private keys, local search text or directory
/// content. They may identify the expected node, endpoint or a validation
/// category so the user can diagnose the failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFetchError {
    pub kind: RemoteFetchErrorKind,
    pub message: String,
}

impl RemoteFetchError {
    pub fn new(kind: RemoteFetchErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn configuration(message: impl Into<String>) -> Self {
        Self::new(RemoteFetchErrorKind::Configuration, message)
    }

    pub fn lookup(message: impl Into<String>) -> Self {
        Self::new(RemoteFetchErrorKind::Lookup, message)
    }

    pub fn unreachable(message: impl Into<String>) -> Self {
        Self::new(RemoteFetchErrorKind::Unreachable, message)
    }

    pub fn identity(message: impl Into<String>) -> Self {
        Self::new(RemoteFetchErrorKind::Identity, message)
    }

    pub fn transport(message: impl Into<String>) -> Self {
        Self::new(RemoteFetchErrorKind::Transport, message)
    }

    pub fn invalid_directory(message: impl Into<String>) -> Self {
        Self::new(RemoteFetchErrorKind::InvalidDirectory, message)
    }
}

impl std::fmt::Display for RemoteFetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind.label(), self.message)
    }
}

impl std::error::Error for RemoteFetchError {}

/// Transport adapter for remote directory retrieval.
///
/// Implementations must authenticate the exact node identity in the address
/// before returning a parsed directory. A mock may be used to test cache and
/// provenance behavior, but it must never stand in for authentication in a
/// production path.
pub trait RemoteDirectoryFetcher: Send + Sync {
    fn fetch(&self, address: &WebsiteAddress) -> std::result::Result<Directory, RemoteFetchError>;
}

/// Authenticated fetcher using signed DHT lookup and pinned TLS 1.3 YNP.
pub struct AuthenticatedFetcher {
    service: yami_dht::Service,
    bootstrap: Vec<Contact>,
    network: Network,
}

impl AuthenticatedFetcher {
    /// Opens the existing bounded DHT lookup cache under `state_directory`.
    ///
    /// `bootstrap` contacts are ordinary manually trusted peers. They are not
    /// discovered or modified here. No connection is opened until [`Self::fetch`].
    pub fn open(
        state_directory: impl AsRef<Path>,
        network: Network,
        bootstrap: Vec<Contact>,
    ) -> std::result::Result<Self, RemoteFetchError> {
        let bootstrap = merge_bootstrap_peers(Vec::new(), bootstrap)?;
        let mode = if network == Network::Public {
            "public"
        } else {
            "local"
        };
        let cache = state_directory
            .as_ref()
            .join(format!("dht-{mode}-discover.cache"));
        let service = yami_dht::Service::open_lookup(&cache, network).map_err(|error| {
            RemoteFetchError::configuration(format!("cannot open local DHT lookup state: {error}"))
        })?;
        let reserved: Vec<NodeId> = bootstrap
            .iter()
            .map(|contact| contact.id)
            .take(yami_dht::RESERVED_OWNERS.saturating_sub(1))
            .collect();
        if !reserved.is_empty() {
            service.protect(&reserved).map_err(|error| {
                RemoteFetchError::configuration(format!(
                    "cannot reserve DHT cache capacity for bootstrap peers: {error}"
                ))
            })?;
        }
        Ok(Self {
            service,
            bootstrap,
            network,
        })
    }

    /// The network policy used for signed records and endpoints.
    pub fn network(&self) -> Network {
        self.network
    }

    /// Configured manual bootstrap contacts.
    pub fn bootstrap(&self) -> &[Contact] {
        &self.bootstrap
    }

    fn fetch_inner(
        &self,
        address: &WebsiteAddress,
    ) -> std::result::Result<Directory, RemoteFetchError> {
        let target = address.node_id();
        let lookup = yami_dht::lookup_fresh(&self.service, target, &self.bootstrap)
            .map_err(|error| RemoteFetchError::lookup(format!("signed lookup failed: {error}")))?;
        let record = lookup
            .record
            .clone()
            .ok_or_else(|| RemoteFetchError::lookup(lookup_failure_message(&lookup)))?;
        if record.id() != target {
            return Err(RemoteFetchError::identity(
                "DHT returned a signed record for a different node identity",
            ));
        }
        let now = yami_dht::now()
            .map_err(|error| RemoteFetchError::lookup(format!("cannot read clock: {error}")))?;
        record
            .verify(self.network, now)
            .map_err(|error| RemoteFetchError::lookup(format!("invalid signed record: {error}")))?;
        self.fetch_record(address, &record)
    }

    fn fetch_record(
        &self,
        address: &WebsiteAddress,
        record: &PeerRecord,
    ) -> std::result::Result<Directory, RemoteFetchError> {
        if record.endpoints().is_empty() {
            return Err(if record.relay_routes().is_empty() {
                RemoteFetchError::unreachable("signed record has no direct endpoint")
            } else {
                RemoteFetchError::transport(
                    "node advertises only relay routes; this build does not fetch directories through relays",
                )
            });
        }
        let mut best: Option<RemoteFetchError> = None;
        for endpoint in record.endpoints() {
            let now = yami_dht::now()
                .map_err(|error| RemoteFetchError::lookup(format!("cannot read clock: {error}")))?;
            record.verify(self.network, now).map_err(|error| {
                RemoteFetchError::lookup(format!("invalid signed record: {error}"))
            })?;
            match fetch_from_endpoint(address, (*endpoint).into()) {
                Ok(directory) => return Ok(directory),
                Err(error) => {
                    if best
                        .as_ref()
                        .is_none_or(|current| error_priority(&error) >= error_priority(current))
                    {
                        best = Some(error);
                    }
                }
            }
        }
        Err(best.unwrap_or_else(|| RemoteFetchError::unreachable("no usable endpoint")))
    }
}

impl RemoteDirectoryFetcher for AuthenticatedFetcher {
    fn fetch(&self, address: &WebsiteAddress) -> std::result::Result<Directory, RemoteFetchError> {
        self.fetch_inner(address)
    }
}

/// Reads the existing `peers.tsv` manual bootstrap store without modifying it.
///
/// This is a read-only compatibility adapter for the peer file maintained by
/// existing YamiNet node/browser commands. It never discovers peers, writes the
/// store or treats historical status as authentication. A missing file is an
/// empty bootstrap list.
pub fn load_bootstrap_peers(
    peer_directory: &Path,
    network: Network,
) -> std::result::Result<Vec<Contact>, RemoteFetchError> {
    let path = peer_directory.join("peers.tsv");
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(RemoteFetchError::configuration(format!(
                "cannot inspect bootstrap peer store: {error}"
            )))
        }
    };
    if !metadata.file_type().is_file() {
        return Err(RemoteFetchError::configuration(
            "bootstrap peer store must be a regular, non-symlink file",
        ));
    }
    if metadata.len() > MAX_PEER_FILE as u64 {
        return Err(RemoteFetchError::configuration(
            "bootstrap peer store exceeds its byte limit",
        ));
    }
    let mut source = String::new();
    File::open(&path)
        .and_then(|file| {
            file.take((MAX_PEER_FILE + 1) as u64)
                .read_to_string(&mut source)
        })
        .map_err(|error| {
            RemoteFetchError::configuration(format!("cannot read bootstrap peer store: {error}"))
        })?;
    if source.len() > MAX_PEER_FILE || !source.starts_with(PEER_HEADER) || !source.ends_with('\n') {
        return Err(RemoteFetchError::configuration(
            "invalid or oversized bootstrap peer store (expected YAMIPEERS version 1)",
        ));
    }
    let mut ids = HashSet::new();
    let mut contacts = Vec::new();
    let mut entries = 0usize;
    for line in source[PEER_HEADER.len()..].lines() {
        entries += 1;
        if line.len() > MAX_PEER_LINE || entries > MAX_PEERS {
            return Err(RemoteFetchError::configuration(
                "bootstrap peer store exceeds its entry or line limit",
            ));
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 4 {
            return Err(RemoteFetchError::configuration(
                "bootstrap peer store entry requires four fields",
            ));
        }
        let id: NodeId = fields[0].parse().map_err(|_| {
            RemoteFetchError::configuration("bootstrap peer store contains an invalid node ID")
        })?;
        let address: SocketAddr = fields[1].parse().map_err(|_| {
            RemoteFetchError::configuration(
                "bootstrap peer store contains an invalid numeric endpoint",
            )
        })?;
        if fields[2] != "-" && fields[2].parse::<u64>().is_err() {
            return Err(RemoteFetchError::configuration(
                "bootstrap peer store contains an invalid success timestamp",
            ));
        }
        if !matches!(fields[3], "unverified" | "authenticated" | "failed") {
            return Err(RemoteFetchError::configuration(
                "bootstrap peer store contains an unknown status",
            ));
        }
        if !ids.insert(id) {
            return Err(RemoteFetchError::configuration(
                "bootstrap peer store contains a duplicate identity",
            ));
        }
        let SocketAddr::V4(endpoint) = address else {
            continue;
        };
        if let Ok(contact) = Contact::new(id, endpoint, network) {
            contacts.push(contact);
        }
    }
    Ok(contacts)
}

/// Merges bootstrap lists by node identity and rejects conflicting endpoints.
pub fn merge_bootstrap_peers(
    mut existing: Vec<Contact>,
    additional: Vec<Contact>,
) -> std::result::Result<Vec<Contact>, RemoteFetchError> {
    for contact in additional {
        if let Some(previous) = existing.iter().find(|current| current.id == contact.id) {
            if previous.endpoint != contact.endpoint {
                return Err(RemoteFetchError::configuration(
                    "conflicting bootstrap endpoints for the same node identity",
                ));
            }
        } else {
            existing.push(contact);
        }
    }
    if existing.len() > yami_dht::MAX_CACHE {
        return Err(RemoteFetchError::configuration(
            "too many bootstrap peers for the bounded DHT lookup",
        ));
    }
    Ok(existing)
}

fn lookup_failure_message(lookup: &yami_dht::Lookup) -> String {
    if lookup.invalid_records > 0 {
        "signed DHT lookup rejected malformed or invalid records".into()
    } else if lookup.timeouts > 0 {
        "signed DHT lookup timed out; check reachable bootstrap peers".into()
    } else if lookup.queries > 0 && lookup.failures == lookup.queries {
        "no configured DHT peer answered the signed lookup".into()
    } else {
        "no signed endpoint record was found for this node identity".into()
    }
}

fn fetch_from_endpoint(
    address: &WebsiteAddress,
    endpoint: SocketAddr,
) -> std::result::Result<Directory, RemoteFetchError> {
    let transport = TlsTransport::new(address.node_id()).map_err(|error| {
        RemoteFetchError::configuration(format!("cannot initialize pinned TLS: {error}"))
    })?;
    let mut connection = transport
        .connect_with_timeout(endpoint, yami_dht::RPC_TIMEOUT)
        .map_err(classify_connect_error)?;
    if connection.protection() != Protection::Tls13PinnedServer {
        return Err(RemoteFetchError::identity(
            "connection did not authenticate TLS 1.3 with the expected node identity",
        ));
    }
    let document =
        ynp::request(&mut *connection, address.key()).map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => RemoteFetchError::transport("document was not found"),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock | io::ErrorKind::UnexpectedEof => {
                RemoteFetchError::unreachable(
                    "node stopped responding while returning the document",
                )
            }
            _ => RemoteFetchError::transport(format!("document request failed: {error}")),
        })?;
    Directory::parse(&document.source).map_err(|error| {
        RemoteFetchError::invalid_directory(format!(
            "delivered document is not a valid version-1 directory: {error}"
        ))
    })
}

fn classify_connect_error(error: io::Error) -> RemoteFetchError {
    if error
        .get_ref()
        .and_then(|source| source.downcast_ref::<IdentityMismatch>())
        .is_some()
    {
        RemoteFetchError::identity(
            "endpoint did not prove the exact node identity named by the directory source",
        )
    } else if matches!(
        error.kind(),
        io::ErrorKind::TimedOut
            | io::ErrorKind::WouldBlock
            | io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::NotConnected
    ) {
        RemoteFetchError::unreachable(format!("pinned TLS endpoint was unreachable: {error}"))
    } else {
        RemoteFetchError::transport(format!("pinned TLS connection failed: {error}"))
    }
}

fn error_priority(error: &RemoteFetchError) -> u8 {
    match error.kind {
        RemoteFetchErrorKind::Configuration => 0,
        RemoteFetchErrorKind::Lookup => 1,
        RemoteFetchErrorKind::Unreachable => 2,
        RemoteFetchErrorKind::Transport => 3,
        RemoteFetchErrorKind::InvalidDirectory => 4,
        RemoteFetchErrorKind::Identity => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        net::SocketAddr,
        sync::Arc,
        thread::{self, JoinHandle},
    };
    use yami_identity::NodeIdentity;
    use ynp::{Frame, Incoming, Message};
    use ynp_tls::TlsListener;

    const ID: &str = "yn1-tiqub4vxm4zslzlrhlfhc5k6b7vvkealot25nodtcixooe6xfzmxfsm6re";

    fn directory_source() -> String {
        format!(
            "@page 0.2\n  title Remote Directory\n:: directory 1\n  :: card Remote Site\n    > yami://{ID}/home | Open\n    A remote listing delivered over pinned TLS.\n"
        )
    }

    fn serve_once(identity: Arc<NodeIdentity>, source: String) -> (SocketAddr, JoinHandle<()>) {
        let listener = TlsListener::bind("127.0.0.1:0".parse().unwrap(), identity).unwrap();
        let endpoint = listener.local_addr().unwrap();
        let task = thread::spawn(move || {
            let Ok(mut connection) = listener.accept() else {
                return;
            };
            let Ok(request) = ynp::read_frame(&mut *connection) else {
                return;
            };
            let Message::Get { key } = request.message else {
                return;
            };
            let _ = ynp::write_frame(
                &mut *connection,
                &Frame {
                    request_id: request.request_id,
                    message: Message::Document(ynp::DocumentData {
                        key,
                        source,
                        style: String::new(),
                    }),
                },
            );
        });
        (endpoint, task)
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "yami-discover-remote-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn loopback_fetch_authenticates_identity_and_parses_directory() {
        let identity = Arc::new(NodeIdentity::generate().unwrap());
        let address = WebsiteAddress::parse(&format!("yami://{}/home", identity.id())).unwrap();
        let (endpoint, server) = serve_once(Arc::clone(&identity), directory_source());
        let directory = fetch_from_endpoint(&address, endpoint).unwrap();
        assert_eq!(directory.name(), "Remote Directory");
        assert_eq!(directory.entries().len(), 1);
        server.join().unwrap();

        let wrong = NodeIdentity::generate().unwrap();
        let wrong_address = WebsiteAddress::parse(&format!("yami://{}/home", wrong.id())).unwrap();
        let (endpoint, server) = serve_once(
            Arc::new(NodeIdentity::generate().unwrap()),
            directory_source(),
        );
        let error = fetch_from_endpoint(&wrong_address, endpoint).unwrap_err();
        assert_eq!(error.kind, RemoteFetchErrorKind::Identity);
        server.join().unwrap();
    }

    #[test]
    fn loopback_invalid_document_is_rejected_after_authentication() {
        let identity = Arc::new(NodeIdentity::generate().unwrap());
        let address = WebsiteAddress::parse(&format!("yami://{}/home", identity.id())).unwrap();
        let (endpoint, server) = serve_once(Arc::clone(&identity), "not a directory\n".into());
        let error = fetch_from_endpoint(&address, endpoint).unwrap_err();
        assert_eq!(error.kind, RemoteFetchErrorKind::InvalidDirectory);
        server.join().unwrap();
    }

    #[test]
    fn peer_store_adapter_reads_manual_pins_without_writing() {
        let root = scratch("peers");
        let peers = root.join("peers.tsv");
        let id = *NodeIdentity::generate().unwrap().id();
        let text = format!("YAMIPEERS\t1\n{id}\t127.0.0.1:7443\t-\tunverified\n");
        fs::write(&peers, &text).unwrap();
        let contacts = load_bootstrap_peers(&root, Network::Loopback).unwrap();
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].id, id);
        assert_eq!(fs::read_to_string(&peers).unwrap(), text);
        assert!(load_bootstrap_peers(&root, Network::Public)
            .unwrap()
            .is_empty());

        fs::write(&peers, "not a peer store").unwrap();
        assert!(load_bootstrap_peers(&root, Network::Loopback).is_err());
        fs::write(&peers, "x".repeat(MAX_PEER_FILE + 1)).unwrap();
        assert!(load_bootstrap_peers(&root, Network::Loopback).is_err());

        let a = Contact::new(id, "127.0.0.1:7443".parse().unwrap(), Network::Loopback).unwrap();
        let b = Contact::new(id, "127.0.0.1:7444".parse().unwrap(), Network::Loopback).unwrap();
        assert!(merge_bootstrap_peers(vec![a], vec![b]).is_err());
        assert_eq!(merge_bootstrap_peers(vec![a], vec![a]).unwrap().len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fetcher_without_bootstrap_peers_reports_lookup_failure() {
        let root = scratch("no-peers");
        let fetcher = AuthenticatedFetcher::open(&root, Network::Loopback, Vec::new()).unwrap();
        let address = WebsiteAddress::parse(&format!("yami://{ID}/home")).unwrap();
        let error = fetcher.fetch(&address).unwrap_err();
        assert_eq!(error.kind, RemoteFetchErrorKind::Lookup);
        drop(fetcher);
        let _ = fs::remove_dir_all(root);
    }
}
