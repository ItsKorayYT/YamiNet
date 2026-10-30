//! Native identity addresses. Discovery supplies locations, never TLS trust.
use std::{net::SocketAddr, path::PathBuf, sync::OnceLock};
use yami_dht::{Contact, Network, PeerRecord, Service};
use yami_identity::NodeId;
use yami_node::peers::PeerStore;
use ynp::{DocumentData, Protection};
use ynp_tls::{IdentityMismatch, TlsTransport};

pub const SCHEME: &str = "yami://";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address {
    pub id: NodeId,
    pub key: String,
}
impl Address {
    pub fn parse(text: &str) -> Result<Self, String> {
        let rest = text
            .strip_prefix(SCHEME)
            .ok_or("Expected yami://node-id/key.")?;
        let (id, key) = rest.split_once('/').unwrap_or((rest, "home"));
        let id = id.parse().map_err(|_| "Invalid Yami node ID: use the complete lowercase yn1- identity, including its checksum.")?;
        let key = if key.is_empty() { "home" } else { key };
        ynp::validate_key(key).map_err(|_| {
            "Invalid Yami document key: use 1–128 ASCII letters, digits, underscores or hyphens."
        })?;
        Ok(Self {
            id,
            key: key.into(),
        })
    }
    pub fn url(&self) -> String {
        format!("{SCHEME}{}/{}", self.id, self.key)
    }
}

/// Files and sockets are opened lazily, on a loader worker. The two browser
/// workers share Service's synchronized persistent validation/high-water cache.
pub struct Resolver {
    directory: PathBuf,
    network: Network,
    service: OnceLock<Result<Service, String>>,
}
impl Resolver {
    pub fn new(directory: PathBuf, network: Network) -> Self {
        Self {
            directory,
            network,
            service: OnceLock::new(),
        }
    }
    pub fn from_environment() -> Result<Self, String> {
        let network = match std::env::var("YAMINET_BROWSER_DHT_NETWORK") {
            Err(std::env::VarError::NotPresent) => Network::Public,
            Ok(value) if value == "public" => Network::Public,
            Ok(value) if value == "local" => Network::Loopback,
            _ => return Err("Invalid browser DHT network setting; select public or local.".into()),
        };
        let directory = yami_node::peers::default_dir()
            .map_err(|_| "Cannot locate browser peer configuration; set YAMINET_PEER_DIR.")?;
        Ok(Self::new(directory, network))
    }
    pub fn state_path(&self) -> PathBuf {
        let mode = if self.network == Network::Public {
            "public"
        } else {
            "local"
        };
        self.directory.join(format!("dht-{mode}-browser.cache"))
    }
    fn service(&self) -> Result<&Service, String> {
        self.service.get_or_init(|| {
            Service::open_lookup(self.state_path(), self.network)
                .map_err(|_| "Browser DHT state or peer records are invalid, locked or unavailable. Check the selected network and state directory.".into())
        }).as_ref().map_err(Clone::clone)
    }
    fn seeds(&self) -> Result<Vec<Contact>, String> {
        let peers = PeerStore::open(&self.directory)
            .map_err(|_| "Browser bootstrap configuration is invalid, locked or unavailable.")?;
        Ok(peers
            .entries()
            .iter()
            .filter_map(|peer| {
                let SocketAddr::V4(endpoint) = peer.address else {
                    return None;
                };
                Contact::new(peer.id, endpoint, self.network).ok()
            })
            .collect())
    }
    pub fn fetch(&self, address: &Address) -> Result<(DocumentData, SocketAddr), String> {
        // Also validate programmatically constructed addresses at the boundary.
        ynp::validate_key(&address.key).map_err(|_| "Invalid Yami document key.")?;
        let service = self.service()?;
        let at =
            yami_dht::now().map_err(|_| "Cannot read the clock for peer record validation.")?;
        let state = service
            .snapshot()
            .map_err(|_| "Browser DHT cache is unavailable.")?;
        let cached = state.get(address.id, at);
        let mut previous_failure = None;
        if let Some(record) = &cached {
            match self.fetch_record(address, record) {
                Ok(data) => return Ok(data),
                Err(FetchError::Document(message)) => return Err(message),
                Err(error) => previous_failure = Some(error.message()),
            }
        }
        let expired = state
            .records()
            .iter()
            .any(|r| r.id() == address.id && r.verify(self.network, at).is_err());
        let seeds = self.seeds()?;
        let has_contacts = state
            .records()
            .iter()
            .any(|r| r.id() != state.local && state.get(r.id(), at).is_some());
        if seeds.is_empty() && !has_contacts {
            return Err(previous_failure.unwrap_or_else(|| if expired {
                "Expired or invalid peer record; no live location is available.".into()
            } else {
                "No bootstrap peers configured for this DHT network. Configure an ordinary peer and retry.".into()
            }));
        }
        let found = if let Some(record) = &cached {
            yami_dht::lookup_newer(service, address.id, &seeds, record.sequence())
        } else {
            yami_dht::lookup(service, address.id, &seeds)
        }
        .map_err(|_| "DHT lookup failed: peer state is invalid or unavailable.")?;
        if let Some(record) = found.record {
            // A failed cached endpoint is retried only after an accepted newer
            // signed record. Equal-sequence changes are rejected by Service.
            if cached
                .as_ref()
                .is_none_or(|old| record.sequence() > old.sequence())
            {
                return self
                    .fetch_record(address, &record)
                    .map_err(FetchError::message);
            }
        }
        if let Some(message) = previous_failure {
            return Err(message);
        }
        Err(if found.invalid_records > 0 || expired {
            "Expired or invalid peer record or DHT response; signature, expiration, sequence or protocol validation failed."
        } else if found.timeouts > 0 {
            "DHT lookup timeout. Retry when a configured peer is reachable."
        } else if found.queries > 0 && found.failures == found.queries {
            "Node temporarily unreachable: no DHT peer answered the lookup."
        } else {
            "Unknown node: no signed endpoint record was found."
        }
        .into())
    }
    fn fetch_record(
        &self,
        address: &Address,
        record: &PeerRecord,
    ) -> Result<(DocumentData, SocketAddr), FetchError> {
        let at = yami_dht::now().map_err(|_| FetchError::Record)?;
        if record.id() != address.id || record.verify(self.network, at).is_err() {
            return Err(FetchError::Record);
        }
        let transport = TlsTransport::new(address.id).map_err(|_| FetchError::Unreachable)?;
        let mut failure = FetchError::Unreachable;
        for endpoint in record.endpoints() {
            record
                .verify(
                    self.network,
                    yami_dht::now().map_err(|_| FetchError::Record)?,
                )
                .map_err(|_| FetchError::Record)?;
            // `connect_with_timeout` bounds handshake AND subsequent YNP I/O.
            match transport.connect_with_timeout((*endpoint).into(), yami_dht::RPC_TIMEOUT) {
                Ok(mut connection) => {
                    if connection.protection() != Protection::Tls13PinnedServer {
                        return Err(FetchError::Identity);
                    }
                    record
                        .verify(
                            self.network,
                            yami_dht::now().map_err(|_| FetchError::Record)?,
                        )
                        .map_err(|_| FetchError::Record)?;
                    let data = ynp::request(&mut *connection, &address.key).map_err(|error| {
                        FetchError::Document(format!(
                            "Verified node could not return this document: {error}"
                        ))
                    })?;
                    return Ok((data, (*endpoint).into()));
                }
                Err(error) => {
                    if error
                        .get_ref()
                        .and_then(|e| e.downcast_ref::<IdentityMismatch>())
                        .is_some()
                    {
                        failure = FetchError::Identity;
                    }
                }
            }
        }
        Err(failure)
    }
}
enum FetchError {
    Record,
    Unreachable,
    Identity,
    Document(String),
}
impl FetchError {
    fn message(self) -> String {
        match self {
            Self::Record => "Expired or invalid peer record.".into(),
            Self::Unreachable => "Node temporarily unreachable: an authenticated TLS 1.3 connection could not be established. YNP never falls back to plaintext.".into(),
            Self::Identity => "TLS identity mismatch: the remote node did not prove the exact requested node ID. Nothing was requested; YNP never falls back to plaintext.".into(),
            Self::Document(message) => message,
        }
    }
}

/// Browser address adapter for native links in the existing inert link nodes.
/// The YLang parser/grammar stays unchanged. Substitution is byte-length
/// preserving, so parser limits, diagnostics and spans still refer to source.
pub fn parse_document(source: &str) -> Result<ylang::Document, ylang::SyntaxError> {
    if source.len() > ylang::MAX_SOURCE || !source.contains("> yami://") {
        return ylang::parse(source);
    }
    let mut adapted = String::new();
    let mut native_lines = Vec::new();
    for (index, line) in source.split_inclusive('\n').enumerate() {
        let content = line.trim_start_matches(' ');
        if content.starts_with("> yami://") {
            native_lines.push(index + 1);
            adapted.push_str(&line.replacen("> yami://", "> ynp:///", 1));
        } else {
            adapted.push_str(line);
        }
    }
    let mut document = ylang::parse(&adapted)?;
    fn restore(nodes: &mut [ylang::Node], lines: &[usize]) {
        for node in nodes {
            match &mut node.kind {
                ylang::NodeKind::Link { target, .. }
                    if lines.binary_search(&node.span.line).is_ok() =>
                {
                    *target = target.replacen("ynp:///", SCHEME, 1);
                }
                ylang::NodeKind::Section { children, .. } => restore(children, lines),
                _ => {}
            }
        }
    }
    restore(&mut document.nodes, &native_lines);
    Ok(document)
}
