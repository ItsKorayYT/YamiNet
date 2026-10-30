//! Opt-in bounded maintenance over the existing DHT RPCs. Scheduling can be
//! driven without sleeping; production owns exactly one cancellable worker.
mod status;
use crate::{peers::PeerStore, shutdown::Shutdown};
pub use status::{read_status, status_path, Status};
use std::{
    collections::HashSet,
    fs::File,
    io::{self, Read},
    net::{SocketAddr, SocketAddrV4},
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use yami_dht::{Contact, Network, PeerRecord, RelayRoute, RpcConnector, Service};
use yami_identity::NodeIdentity;

pub const CONTACTS_PER_PASS: usize = 4;
pub const MAX_MAINTENANCE_QUERIES: usize = CONTACTS_PER_PASS * 2;
pub const MAX_BACKOFF: u64 = 900;

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub renew_secs: u64,
    pub health_secs: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            renew_secs: 6 * 3600,
            health_secs: 300,
        }
    }
}
impl Config {
    pub fn validate(self) -> io::Result<Self> {
        if !(30..=yami_dht::MAX_RECORD_TTL / 2).contains(&self.renew_secs)
            || !(5..=3600).contains(&self.health_secs)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "DHT renewal must be 30..43200 seconds and health checks 5..3600 seconds",
            ));
        }
        Ok(self)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    NoPeers,
    Unreachable,
    State,
    Configuration,
    Clock,
}
impl Failure {
    pub fn message(self) -> &'static str {
        match self {
            Self::NoPeers => "no compatible bootstrap or learned peers available",
            Self::Unreachable => {
                "peer maintenance failed: connection, identity or signed-record validation refused"
            }
            Self::State => "local DHT state could not be safely updated",
            Self::Configuration => {
                "maintenance peer or endpoint configuration is invalid or unavailable"
            }
            Self::Clock => "DHT clock unavailable or moved backwards",
        }
    }
}
#[derive(Default, Debug)]
pub struct Pass {
    pub queries: usize,
    pub verified: usize,
    pub published: usize,
    pub failures: usize,
}

pub struct Maintainer {
    service: Service,
    network: Network,
    identity: Arc<NodeIdentity>,
    endpoints: Vec<SocketAddrV4>,
    routes: Vec<RelayRoute>,
    config: Config,
    record: PeerRecord,
    initial: bool,
    next_publication: u64,
    next_health: u64,
    not_before: u64,
    publication_failures: u32,
    health_failures: u32,
    cursor: usize,
    retries: Vec<(Contact, u32, u64)>,
    last_tick: u64,
    last_publication: Option<u64>,
    last_error: Option<Failure>,
}
impl Maintainer {
    pub fn new(
        service: Service,
        identity: Arc<NodeIdentity>,
        endpoints: Vec<SocketAddrV4>,
        config: Config,
    ) -> io::Result<Self> {
        let config = config.validate()?;
        let at = service.now()?;
        let network = service.snapshot()?.network;
        let record = service.prepare_self(&identity, endpoints, at)?;
        let endpoints = record.endpoints().to_vec();
        Ok(Self {
            service,
            network,
            identity,
            endpoints,
            routes: Vec::new(),
            config,
            record,
            initial: true,
            next_publication: at,
            next_health: at,
            not_before: at,
            publication_failures: 0,
            health_failures: 0,
            cursor: 0,
            retries: Vec::new(),
            last_tick: at,
            last_publication: None,
            last_error: None,
        })
    }
    pub fn record(&self) -> &PeerRecord {
        &self.record
    }
    pub fn next_publication(&self) -> u64 {
        self.next_publication
    }
    pub fn next_due(&self) -> u64 {
        self.next_publication
            .min(self.next_health)
            .max(self.not_before)
    }
    /// Explicit operator update only. Validate the entire replacement first;
    /// no automatic external-IP discovery or probing is performed.
    pub fn set_endpoints(&mut self, endpoints: Vec<SocketAddrV4>) -> io::Result<()> {
        let network = self.service.snapshot()?.network;
        if endpoints.is_empty() || endpoints.len() > 4 {
            return Err(io::Error::other("expected one to four endpoints"));
        }
        for endpoint in &endpoints {
            network.check(*endpoint)?;
        }
        let mut endpoints = endpoints;
        endpoints.sort();
        endpoints.dedup();
        if endpoints != self.endpoints {
            self.endpoints = endpoints;
            self.initial = false;
            self.next_publication = self.service.now()?;
        }
        Ok(())
    }
    /// Live outbound registrations feed the existing publication scheduler.
    pub fn set_routes(&mut self, mut routes: Vec<RelayRoute>) -> io::Result<()> {
        routes.sort();
        routes.dedup();
        // Validate before replacing configuration, including the empty withdrawal.
        PeerRecord::sign_routes(
            &self.identity,
            self.endpoints.clone(),
            routes.clone(),
            self.service.now()?.saturating_add(yami_dht::MAX_RECORD_TTL),
            1,
            self.network,
            self.service.now()?,
        )?;
        if routes != self.routes {
            self.routes = routes;
            self.initial = false;
            self.next_publication = self.service.now()?;
        }
        Ok(())
    }
    /// At most four peers/eight RPCs/15 seconds per pass; stop is checked
    /// between every RPC. Socket connect/handshake/I/O share a 3-second budget.
    pub fn tick(
        &mut self,
        seeds: &[Contact],
        shutdown: &Shutdown,
        entropy: u64,
        connector: &dyn RpcConnector,
    ) -> io::Result<Pass> {
        let at = self.service.now()?;
        if shutdown.is_stopped() {
            return Ok(Pass::default());
        }
        if at < self.last_tick {
            self.next_publication = at;
            self.next_health = at;
            self.not_before = at;
            self.initial = false;
            self.last_error = Some(Failure::Clock);
        }
        self.last_tick = at;
        if at < self.not_before {
            return Ok(Pass::default());
        }
        let publish = at >= self.next_publication
            || self
                .record
                .verify(self.service.snapshot()?.network, at)
                .is_err();
        if !publish && at < self.next_health {
            return Ok(Pass::default());
        }
        if seeds.len() > yami_dht::MAX_CACHE {
            return Err(io::Error::other("too many maintenance seeds"));
        }
        let network = self.service.snapshot()?.network;
        for seed in seeds {
            seed.validate(network)?;
        }
        if publish {
            if self.initial && self.record.verify(network, at).is_ok() {
                self.initial = false;
            } else {
                self.initial = false;
                match self.service.prepare_self_routes(
                    &self.identity,
                    self.endpoints.clone(),
                    self.routes.clone(),
                    at,
                ) {
                    Ok(record) => self.record = record,
                    Err(error) => {
                        self.last_error = Some(Failure::State);
                        self.publication_failures = self.publication_failures.saturating_add(1);
                        self.next_publication = at.saturating_add(backoff(
                            self.publication_failures,
                            self.config.health_secs,
                            entropy,
                        ));
                        self.next_health = self.next_publication;
                        return Err(error);
                    }
                }
            }
        }
        let mut candidates = self.service.contacts(at)?;
        candidates.extend_from_slice(seeds);
        candidates.retain(|c| c.id != *self.identity.id());
        candidates.sort_by_key(|c| (c.id.public_key(), c.endpoint));
        candidates.dedup();
        self.retries
            .retain(|(contact, _, _)| candidates.contains(contact));
        let mut pass = Pass::default();
        let deadline = Instant::now() + yami_dht::LOOKUP_TIMEOUT;
        let count = candidates.len().min(CONTACTS_PER_PASS);
        for offset in 0..count {
            let contact = candidates[(self.cursor + offset) % candidates.len()];
            if shutdown.is_stopped() || Instant::now() >= deadline {
                break;
            }
            if self
                .retries
                .iter()
                .any(|(peer, _, until)| *peer == contact && *until > at)
            {
                continue;
            }
            if publish {
                pass.queries += 1;
                let budget =
                    yami_dht::RPC_TIMEOUT.min(deadline.saturating_duration_since(Instant::now()));
                if yami_dht::exchange(
                    &self.service,
                    contact,
                    Some(&self.record),
                    contact.id,
                    budget,
                    connector,
                )
                .is_ok()
                {
                    pass.published += 1;
                } else {
                    pass.failures += 1;
                    self.peer_failed(contact, at, entropy);
                    continue;
                }
            }
            if shutdown.is_stopped() || Instant::now() >= deadline {
                break;
            }
            pass.queries += 1;
            let budget =
                yami_dht::RPC_TIMEOUT.min(deadline.saturating_duration_since(Instant::now()));
            // Query near alternating local/peer IDs to refresh learned routes.
            let target = if self.cursor.wrapping_add(offset) % 2 == 0 {
                *self.identity.id()
            } else {
                contact.id
            };
            if yami_dht::exchange(&self.service, contact, None, target, budget, connector).is_ok() {
                pass.verified += 1;
                self.retries.retain(|(peer, _, _)| *peer != contact);
            } else {
                pass.failures += 1;
                self.peer_failed(contact, at, entropy);
            }
        }
        if !candidates.is_empty() {
            self.cursor = (self.cursor + count) % candidates.len();
        }
        let finished = self.service.now()?;
        self.not_before = finished.saturating_add(5);
        if publish {
            if pass.published > 0 {
                self.last_publication = Some(finished);
                self.publication_failures = 0;
                self.next_publication =
                    finished.saturating_add(jitter(self.config.renew_secs, entropy));
            } else {
                self.publication_failures = self.publication_failures.saturating_add(1);
                self.next_publication = finished.saturating_add(backoff(
                    self.publication_failures,
                    self.config.health_secs,
                    entropy,
                ));
            }
        }
        if pass.verified > 0 || pass.published > 0 {
            self.health_failures = 0;
            self.next_health =
                finished.saturating_add(jitter(self.config.health_secs, entropy.rotate_left(17)));
        } else {
            self.health_failures = self.health_failures.saturating_add(1);
            self.next_health = finished.saturating_add(backoff(
                self.health_failures,
                self.config.health_secs,
                entropy.rotate_left(17),
            ));
        }
        self.last_error = if candidates.is_empty() {
            Some(Failure::NoPeers)
        } else if pass.failures > 0 || (pass.verified == 0 && pass.published == 0) {
            Some(Failure::Unreachable)
        } else {
            None
        };
        Ok(pass)
    }
    fn peer_failed(&mut self, contact: Contact, at: u64, entropy: u64) {
        if let Some((_, failures, until)) = self
            .retries
            .iter_mut()
            .find(|(peer, _, _)| *peer == contact)
        {
            *failures = failures.saturating_add(1);
            *until = at.saturating_add(backoff(*failures, self.config.health_secs, entropy));
        } else {
            // Cooldown memory is independently bounded under endpoint churn.
            if self.retries.len() == yami_dht::MAX_CANDIDATES {
                self.retries.remove(0);
            }
            self.retries.push((
                contact,
                1,
                at.saturating_add(backoff(1, self.config.health_secs, entropy)),
            ));
        }
    }
    pub fn status(&self, running: bool) -> io::Result<Status> {
        let time = self.service.now();
        let at = time.as_ref().copied().unwrap_or(self.last_tick);
        let mut status = Status {
            id: *self.identity.id(),
            network: self.network,
            observed: at,
            running,
            known_peers: 0,
            recently_verified: 0,
            last_publication: self.last_publication,
            expires: self.record.expires(),
            next_attempt: self.next_due(),
            last_error: if time.is_err() {
                Some(Failure::Clock)
            } else {
                self.last_error
            },
        };
        // Diagnostics must remain writable even after a fail-closed cache
        // persistence error. Do not reuse unconfirmed counts in that case.
        let state = match self
            .service
            .contacts(at)
            .and_then(|_| self.service.snapshot())
        {
            Ok(state) => state,
            Err(_) => {
                status.last_error = Some(Failure::State);
                return Ok(status);
            }
        };
        let mut peers: HashSet<_> = state
            .records()
            .iter()
            .filter(|r| r.id() != state.local && state.get(r.id(), at).is_some())
            .map(PeerRecord::id)
            .collect();
        peers.extend(
            state
                .routing
                .contacts()
                .into_iter()
                .filter(|c| c.id != state.local)
                .map(|c| c.id),
        );
        status.known_peers = peers.len();
        status.recently_verified = state.routing.recently_verified(at);
        Ok(status)
    }
}
fn jitter(base: u64, entropy: u64) -> u64 {
    let spread = base / 10;
    base - spread + entropy % (spread * 2 + 1)
}
fn backoff(failures: u32, health: u64, entropy: u64) -> u64 {
    let base = health
        .min(30)
        .saturating_mul(1u64 << failures.saturating_sub(1).min(10))
        .min(MAX_BACKOFF * 5 / 6);
    base.saturating_add(entropy % (base / 5 + 1))
        .min(MAX_BACKOFF)
}

pub fn read_seeds(directory: &Path, network: Network) -> io::Result<Vec<Contact>> {
    let store = PeerStore::open(directory)?;
    Ok(store
        .entries()
        .iter()
        .filter_map(|peer| {
            let SocketAddr::V4(endpoint) = peer.address else {
                return None;
            };
            Contact::new(peer.id, endpoint, network).ok()
        })
        .collect())
}
pub fn read_endpoints(path: &Path, network: Network) -> io::Result<Vec<SocketAddrV4>> {
    let mut text = String::new();
    File::open(path)?.take(257).read_to_string(&mut text)?;
    if text.len() > 256 {
        return Err(io::Error::other("endpoint file exceeds limit"));
    }
    let endpoints: Vec<SocketAddrV4> = text
        .split_whitespace()
        .map(|s| s.parse().map_err(io::Error::other))
        .collect::<io::Result<_>>()?;
    if endpoints.is_empty() || endpoints.len() > 4 {
        return Err(io::Error::other("expected one to four endpoints"));
    }
    for endpoint in &endpoints {
        network.check(*endpoint)?;
    }
    Ok(endpoints)
}

pub struct Worker {
    shutdown: Shutdown,
    task: Option<thread::JoinHandle<()>>,
}
impl Worker {
    pub fn start(
        engine: Maintainer,
        directory: PathBuf,
        endpoints_file: Option<PathBuf>,
        shutdown: Shutdown,
    ) -> io::Result<Self> {
        Self::start_with_routes(engine, directory, endpoints_file, shutdown, None)
    }
    pub fn start_with_routes(
        mut engine: Maintainer,
        directory: PathBuf,
        endpoints_file: Option<PathBuf>,
        shutdown: Shutdown,
        routes: Option<crate::relay_host::ActiveRoutes>,
    ) -> io::Result<Self> {
        let signal = shutdown.clone();
        let network = engine.service.snapshot()?.network;
        let path = status_path(&directory, network);
        if let Ok(previous) = read_status(&path, network) {
            if previous.id == *engine.identity.id() {
                engine.last_publication = previous
                    .last_publication
                    .filter(|time| *time <= engine.last_tick);
            }
        }
        let task = thread::Builder::new()
            .name("dht-maintenance".into())
            .spawn(move || {
                let mut startup = true;
                while !signal.is_stopped() {
                    let mut configuration_error = false;
                    if let Some(routes) = &routes {
                        if routes.current().and_then(|r| engine.set_routes(r)).is_err() {
                            configuration_error = true;
                        }
                    }
                    let seeds = read_seeds(&directory, network).unwrap_or_else(|_| {
                        configuration_error = true;
                        Vec::new()
                    });
                    if let Some(file) = &endpoints_file {
                        if read_endpoints(file, network)
                            .and_then(|endpoints| engine.set_endpoints(endpoints))
                            .is_err()
                        {
                            configuration_error = true;
                        }
                    }
                    let mut bytes = [0; 8];
                    // Entropy failure delays maintenance instead of substituting a
                    // synchronized fixed schedule on every process.
                    if getrandom::fill(&mut bytes).is_err() {
                        engine.last_error = Some(Failure::State);
                        if let Ok(status) = engine.status(true) {
                            let _ = status::write_status(&path, &status);
                        }
                        signal.wait(Duration::from_secs(30));
                        continue;
                    }
                    match engine.tick(
                        &seeds,
                        &signal,
                        u64::from_ne_bytes(bytes),
                        &yami_dht::PinnedTls,
                    ) {
                        Ok(pass) if startup => {
                            eprintln!(
                                "DHT startup publication complete: {} authenticated bootstrap(s)",
                                pass.published
                            );
                            startup = false;
                        }
                        Ok(_) => {}
                        Err(_) => {
                            engine.last_error = Some(Failure::State);
                            engine.health_failures = engine.health_failures.saturating_add(1);
                            if let Ok(at) = engine.service.now() {
                                let delay = backoff(
                                    engine.health_failures,
                                    engine.config.health_secs,
                                    u64::from_ne_bytes(bytes),
                                );
                                engine.next_health = at.saturating_add(delay);
                                engine.next_publication =
                                    engine.next_publication.max(engine.next_health);
                            }
                        }
                    }
                    if configuration_error {
                        engine.last_error = Some(Failure::Configuration);
                    }
                    if let Ok(status) = engine.status(true) {
                        if status::write_status(&path, &status).is_err() {
                            eprintln!("DHT maintenance diagnostics could not be saved");
                        }
                    }
                    let delay = engine
                        .service
                        .now()
                        .map(|at| engine.next_due().saturating_sub(at).clamp(5, 60))
                        .unwrap_or(30);
                    signal.wait(Duration::from_secs(if routes.is_some() {
                        delay.min(1)
                    } else {
                        delay
                    }));
                }
                if let Ok(status) = engine.status(false) {
                    let _ = status::write_status(&path, &status);
                }
            })?;
        Ok(Self {
            shutdown,
            task: Some(task),
        })
    }
    pub fn stop(&mut self) {
        self.shutdown.stop();
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}
