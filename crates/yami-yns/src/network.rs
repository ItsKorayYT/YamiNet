//! Bounded YNS v1 publication/query over YNP wire v4 and destination-pinned TLS.
//! Referrals are location hints, never ownership or freshness attestations.
use crate::{Bundle, Error, Name, Result, Store};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashSet, VecDeque},
    net::SocketAddrV4,
    path::Path,
    sync::Mutex,
    time::{Duration, Instant},
};
use yami_dht::{Contact, Network};
use yami_identity::NodeId;
pub const MAX_PEERS: usize = 4;
pub const MAX_CONTACTS: usize = 8;
pub const MAX_PUBLICATIONS_PER_MINUTE: usize = 32;
pub const RPC_BUDGET: Duration = Duration::from_secs(2);
pub const DISCOVERY_BUDGET: Duration = Duration::from_secs(8);
pub const REFRESH_SECONDS: u64 = 60;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub id: String,
    pub endpoint: SocketAddrV4,
}
impl Provider {
    pub fn parse(text: &str, network: Network) -> Result<Self> {
        let (id, endpoint) = text
            .split_once('@')
            .ok_or(Error::Invalid("provider requires full-ID@IPv4:port"))?;
        let provider = Self {
            id: id.into(),
            endpoint: endpoint
                .parse()
                .map_err(|_| Error::Invalid("numeric provider IPv4 endpoint required"))?,
        };
        provider.contact(network)?;
        Ok(provider)
    }
    pub fn contact(&self, network: Network) -> Result<Contact> {
        Contact::new(
            self.id
                .parse()
                .map_err(|_| Error::Invalid("full provider identity required"))?,
            self.endpoint,
            network,
        )
        .map_err(Error::Io)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    pub bundle: Option<Bundle>,
    pub peers: Vec<Provider>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase", deny_unknown_fields)]
enum Operation {
    Query { name: String },
    Publish { bundle: Bundle },
    Answer { answer: Answer },
    Stored,
    Rejected,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u8,
    message: Operation,
}
fn encode(message: Operation) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(&Envelope {
        version: 1,
        message,
    })
    .map_err(|_| Error::Invalid("naming encoding"))?;
    if bytes.len() > ynp::MAX_NAMING_BYTES {
        return Err(Error::Invalid("naming envelope exceeds budget"));
    }
    Ok(bytes)
}
fn decode(bytes: &[u8]) -> Result<Operation> {
    if bytes.len() > ynp::MAX_NAMING_BYTES {
        return Err(Error::Invalid("naming envelope exceeds budget"));
    }
    let e: Envelope =
        serde_json::from_slice(bytes).map_err(|_| Error::Invalid("malformed naming envelope"))?;
    if e.version != 1 {
        return Err(Error::Invalid("unsupported naming protocol version"));
    }
    match &e.message {
        Operation::Query { name } => {
            if matches!(Name::parse(name)?, Name::Alias(_)) {
                return Err(Error::Invalid("aliases cannot leave the profile"));
            }
        }
        Operation::Publish { bundle } => bounded_bundle(bundle)?,
        Operation::Answer { answer } => {
            if answer.peers.len() > MAX_PEERS {
                return Err(Error::Invalid("too many referrals"));
            }
            if let Some(bundle) = &answer.bundle {
                bounded_bundle(bundle)?;
            }
        }
        _ => {}
    }
    Ok(e.message)
}
fn bounded_bundle(bundle: &Bundle) -> Result<()> {
    if bundle.records.is_empty() || bundle.records.len() > 8 || bundle.name.len() > 330 {
        return Err(Error::Invalid("chain resource budget"));
    }
    for r in &bundle.records {
        if r.to_bytes().len() > crate::MAX_RECORD_BYTES {
            return Err(Error::Invalid("record resource budget"));
        }
    }
    Ok(())
}
fn exchange(
    provider: &Provider,
    network: Network,
    op: Operation,
    deadline: Instant,
) -> Result<Operation> {
    let contact = provider.contact(network)?;
    let remaining = deadline
        .saturating_duration_since(Instant::now())
        .min(RPC_BUDGET);
    let mut connection = ynp_tls::TlsTransport::new(contact.id)?
        .connect_with_timeout(contact.endpoint.into(), remaining)?;
    let bytes = ynp::request_naming(&mut *connection, encode(op)?)?;
    decode(&bytes)
}
pub fn query(
    provider: &Provider,
    network: Network,
    name: &Name,
    deadline: Instant,
) -> Result<Answer> {
    if matches!(name, Name::Alias(_)) {
        return Err(Error::Invalid("aliases are never sent to peers"));
    }
    match exchange(
        provider,
        network,
        Operation::Query {
            name: name.to_string(),
        },
        deadline,
    )? {
        Operation::Answer { answer } => {
            if answer
                .bundle
                .as_ref()
                .is_some_and(|b| b.name != name.to_string())
            {
                return Err(Error::Invalid("provider returned another name"));
            }
            Ok(answer)
        }
        _ => Err(Error::Invalid("unexpected query response")),
    }
}
pub fn publish(
    provider: &Provider,
    network: Network,
    bundle: Bundle,
    deadline: Instant,
) -> Result<()> {
    bounded_bundle(&bundle)?;
    match exchange(provider, network, Operation::Publish { bundle }, deadline)? {
        Operation::Stored => Ok(()),
        _ => Err(Error::Invalid("publisher rejected the signed chain")),
    }
}
/// A bounded, unstructured peer graph. No forwarding, broadcast, recursive RPC,
/// default server, DNS, or assertion that the searched subset is globally complete.
#[derive(Default, Debug)]
pub struct Discovery {
    pub bundles: Vec<Bundle>,
    pub contacted: usize,
    pub failures: usize,
}
pub fn discover(
    seeds: &[Provider],
    network: Network,
    name: &Name,
    registry: &[NodeId],
    now: u64,
    deadline: Instant,
) -> Result<Discovery> {
    if seeds.is_empty() {
        return Err(Error::NoProviders);
    }
    if seeds.len() > MAX_CONTACTS || matches!(name, Name::Alias(_)) {
        return Err(Error::Invalid("discovery seed/name bounds"));
    }
    let deadline = deadline.min(Instant::now() + DISCOVERY_BUDGET);
    let mut seen = HashSet::new();
    let mut queue = VecDeque::new();
    let mut result = Discovery::default();
    for seed in seeds {
        let c = seed.contact(network)?;
        if seen.insert(c.id) {
            queue.push_back(seed.clone());
        }
    }
    while let Some(provider) = queue.pop_front() {
        if result.contacted >= MAX_CONTACTS || Instant::now() >= deadline {
            break;
        }
        result.contacted += 1;
        match query(&provider, network, name, deadline) {
            Ok(answer) => {
                if let Some(bundle) = answer.bundle {
                    if bundle.verify(registry, now).is_ok() {
                        result.bundles.push(bundle);
                    } else {
                        result.failures += 1;
                    }
                }
                for peer in answer.peers {
                    // Hints cannot cross the selected network policy or cause
                    // repeated/cyclic probes; identities are pinned before RPC.
                    if seen.len() >= MAX_CONTACTS {
                        break;
                    }
                    if let Ok(contact) = peer.contact(network) {
                        if seen.insert(contact.id) {
                            queue.push_back(peer);
                        }
                    }
                }
            }
            Err(_) => result.failures += 1,
        }
    }
    Ok(result)
}
/// An opt-in mirror on the existing node TLS listener. Remote clients cannot
/// configure local aliases, registry keys, peers, or destination binding approvals.
pub struct Service {
    store: Mutex<Store>,
    peers: Vec<Provider>,
    publication_budget: Mutex<(Instant, usize)>,
}
impl Service {
    pub fn open(dir: impl AsRef<Path>, network: Network, peers: Vec<Provider>) -> Result<Self> {
        if peers.len() > MAX_PEERS {
            return Err(Error::Invalid("too many mirror peers"));
        }
        for peer in &peers {
            peer.contact(network)?;
        }
        Ok(Self {
            store: Mutex::new(Store::open(dir)?),
            peers,
            publication_budget: Mutex::new((Instant::now(), 0)),
        })
    }
    pub fn configure_registry(&self, keys: &[NodeId]) -> Result<()> {
        self.store
            .lock()
            .map_err(|_| Error::Invalid("mirror lock poisoned"))?
            .configure_registry(keys)
    }
    pub fn handle(&self, bytes: &[u8], now: u64) -> Result<Vec<u8>> {
        let message = match decode(bytes)? {
            Operation::Query { name } => {
                let name = Name::parse(&name)?;
                let store = self
                    .store
                    .lock()
                    .map_err(|_| Error::Invalid("mirror lock poisoned"))?;
                Operation::Answer {
                    answer: Answer {
                        bundle: store.export_bundle(&name, now)?,
                        peers: self.peers.clone(),
                    },
                }
            }
            Operation::Publish { bundle } => {
                let mut budget = self
                    .publication_budget
                    .lock()
                    .map_err(|_| Error::Invalid("publication budget unavailable"))?;
                if budget.0.elapsed() >= Duration::from_secs(60) {
                    *budget = (Instant::now(), 0);
                }
                if budget.1 >= MAX_PUBLICATIONS_PER_MINUTE {
                    return encode(Operation::Rejected);
                }
                budget.1 += 1;
                drop(budget);
                let mut store = self
                    .store
                    .lock()
                    .map_err(|_| Error::Invalid("mirror lock poisoned"))?;
                if store.import_bundle(&bundle, now).is_ok() {
                    Operation::Stored
                } else {
                    Operation::Rejected
                }
            }
            _ => {
                return Err(Error::Invalid(
                    "mirror accepts query/publication requests only",
                ))
            }
        };
        encode(message)
    }
}
