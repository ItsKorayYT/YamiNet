//! Native identity addresses. Discovery supplies locations, never TLS trust.
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
    time::{Duration, Instant},
};
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConnectionInfo {
    pub endpoint: SocketAddr,
    /// Set only after BOTH relay and destination completed their pinned TLS handshakes.
    pub relay: Option<NodeId>,
}
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
struct DeadlineConnector(Instant);
impl yami_dht::RpcConnector for DeadlineConnector {
    fn connect(
        &self,
        contact: Contact,
        budget: Duration,
    ) -> std::io::Result<Box<dyn ynp::Connection>> {
        let budget = budget.min(self.0.saturating_duration_since(Instant::now()));
        if budget.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "browser total deadline exceeded",
            ));
        }
        yami_dht::RpcConnector::connect(&yami_dht::PinnedTls, contact, budget)
    }
}
pub struct Resolver {
    directory: PathBuf,
    network: Network,
    service: OnceLock<Result<Service, String>>,
    pub(crate) relay_enabled: Arc<AtomicBool>,
}
impl Resolver {
    pub fn new(directory: PathBuf, network: Network) -> Self {
        Self {
            directory,
            network,
            service: OnceLock::new(),
            relay_enabled: Arc::new(AtomicBool::new(false)),
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
        let resolver = Self::new(directory, network);
        let enabled = match std::env::var("YAMINET_BROWSER_RELAYS").as_deref() {
            Ok("1") => true,
            Ok("0") | Err(std::env::VarError::NotPresent) => false,
            _ => return Err("YAMINET_BROWSER_RELAYS must be 0 or 1".into()),
        };
        resolver.set_relay_enabled(enabled);
        Ok(resolver)
    }
    pub fn set_relay_enabled(&self, enabled: bool) {
        self.relay_enabled.store(enabled, Ordering::SeqCst);
    }
    pub fn relay_enabled(&self) -> bool {
        self.relay_enabled.load(Ordering::SeqCst)
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
        self.fetch_verified(address)
            .map(|(data, info)| (data, info.endpoint))
    }
    pub fn fetch_verified(
        &self,
        address: &Address,
    ) -> Result<(DocumentData, ConnectionInfo), String> {
        let deadline = Instant::now() + FETCH_TIMEOUT;
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
            match self.fetch_record(address, record, deadline) {
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
            yami_dht::lookup_newer_with_connector(
                service,
                address.id,
                &seeds,
                record.sequence(),
                &DeadlineConnector(deadline),
            )
        } else {
            yami_dht::lookup_with_connector(
                service,
                address.id,
                &seeds,
                &DeadlineConnector(deadline),
            )
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
                    .fetch_record(address, &record, deadline)
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
        deadline: Instant,
    ) -> Result<(DocumentData, ConnectionInfo), FetchError> {
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
            match transport.connect_with_timeout(
                (*endpoint).into(),
                yami_dht::RPC_TIMEOUT.min(deadline.saturating_duration_since(Instant::now())),
            ) {
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
                    return Ok((
                        data,
                        ConnectionInfo {
                            endpoint: (*endpoint).into(),
                            relay: None,
                        },
                    ));
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
        if self.relay_enabled() {
            for advertised in record.relay_routes() {
                record
                    .verify(
                        self.network,
                        yami_dht::now().map_err(|_| FetchError::Record)?,
                    )
                    .map_err(|_| FetchError::Record)?;
                let budget =
                    yami_dht::RPC_TIMEOUT.min(deadline.saturating_duration_since(Instant::now()));
                if budget.is_zero() {
                    break;
                }
                let route = yami_relay::Route {
                    relay: advertised.relay,
                    endpoint: advertised.endpoint.into(),
                    destination: address.id,
                };
                match route.connect_with_timeout(self.network == Network::Public, budget) {
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
                        let data = ynp::request(&mut *connection, &address.key).map_err(|e| {
                            FetchError::Document(format!(
                                "Verified node could not return this document: {e}"
                            ))
                        })?;
                        return Ok((
                            data,
                            ConnectionInfo {
                                endpoint: advertised.endpoint.into(),
                                relay: Some(advertised.relay),
                            },
                        ));
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
        } else if record.endpoints().is_empty() && !record.relay_routes().is_empty() {
            return Err(FetchError::Document("This node advertises relay routes. Enable relay fallback in the browser settings and reload.".into()));
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

/// Explicit settings action: persist a pin, without making a connection or fetching content.
pub fn add_bootstrap_from_environment(pin: &str) -> Result<(), String> {
    let resolver = Resolver::from_environment()?;
    let (id, endpoint) = pin.split_once('@').ok_or("Use full-node-ID@IPv4:port")?;
    let id = id.parse().map_err(|_| "Invalid full bootstrap node ID")?;
    let endpoint = endpoint
        .parse()
        .map_err(|_| "Invalid bootstrap IPv4 endpoint")?;
    Contact::new(id, endpoint, resolver.network).map_err(|e| e.to_string())?;
    PeerStore::open(&resolver.directory)
        .and_then(|mut peers| peers.add(id, endpoint.into()))
        .map_err(|e| format!("Cannot save bootstrap pin: {e}"))
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
