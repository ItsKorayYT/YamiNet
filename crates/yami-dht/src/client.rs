use crate::{
    distance, invalid, now,
    wire::{self, Message},
    Contact, PeerRecord, Service, LOOKUP_TIMEOUT, MAX_CANDIDATES, MAX_QUERIES, RPC_TIMEOUT,
};
use std::{
    collections::HashSet,
    io,
    time::{Duration, Instant},
};
use yami_identity::NodeId;
use ynp::{Connection, Protection};
use ynp_tls::TlsTransport;

/// Extension point for future transports. Adapters MUST pin `contact.id`, keep
/// TLS 1.3 encryption, and honor the full connect/handshake/I/O budget. Network
/// policy still runs before this hook. No anonymous transport is implemented.
pub trait RpcConnector: Send + Sync {
    fn connect(&self, contact: Contact, budget: Duration) -> io::Result<Box<dyn Connection>>;
}
pub struct PinnedTls;
impl RpcConnector for PinnedTls {
    fn connect(&self, contact: Contact, budget: Duration) -> io::Result<Box<dyn Connection>> {
        TlsTransport::new(contact.id)?.connect_with_timeout(contact.endpoint.into(), budget)
    }
}
#[cfg(test)]
fn query(contact: Contact, request: Message, timeout: Duration) -> io::Result<Message> {
    query_with_connector(&PinnedTls, contact, request, timeout)
}
fn query_with_connector(
    connector: &dyn RpcConnector,
    contact: Contact,
    request: Message,
    timeout: Duration,
) -> io::Result<Message> {
    let mut connection = connector.connect(contact, timeout)?;
    if connection.protection() != Protection::Tls13PinnedServer {
        return Err(invalid("DHT query did not authenticate TLS 1.3"));
    }
    let mut nonce = [0; 16];
    getrandom::fill(&mut nonce).map_err(io::Error::other)?;
    wire::write(&mut *connection, nonce, &request)?;
    let (found, response) = wire::read(&mut *connection)?;
    if found != nonce {
        return Err(invalid("unsolicited/mismatched DHT response"));
    }
    match (&request, &response) {
        (Message::Find(_), Message::Peers(_)) | (Message::Store(_), Message::Stored) => {
            Ok(response)
        }
        _ => Err(invalid("DHT response type does not match query")),
    }
}
#[derive(Debug)]
pub struct Lookup {
    pub record: Option<PeerRecord>,
    pub queries: usize,
    pub failures: usize,
}
/// Sequential iterative lookup (alpha=1), nearest first. Each endpoint attempt
/// consumes one query, including failures. No recursion, automatic fetch or DNS.
pub fn lookup(service: &Service, target: NodeId, seeds: &[Contact]) -> io::Result<Lookup> {
    lookup_with_connector(service, target, seeds, &PinnedTls)
}
pub fn lookup_with_connector(
    service: &Service,
    target: NodeId,
    seeds: &[Contact],
    connector: &dyn RpcConnector,
) -> io::Result<Lookup> {
    let initial = service.snapshot()?;
    let network = initial.network;
    if seeds.len() > crate::MAX_CACHE {
        return Err(invalid("too many bootstrap peers"));
    }
    for seed in seeds {
        seed.validate(network)?;
    }
    let mut result = Lookup {
        record: None,
        queries: 0,
        failures: 0,
    };
    let deadline = Instant::now() + LOOKUP_TIMEOUT;
    let mut candidates = service.candidates(now()?)?;
    candidates.extend_from_slice(seeds);
    let mut visited = HashSet::new();
    loop {
        candidates.sort_by_key(|c| (distance(c.id, target), c.id.public_key(), c.endpoint));
        candidates.dedup();
        candidates.retain(|c| !visited.contains(c));
        candidates.truncate(MAX_CANDIDATES);
        if result.queries >= MAX_QUERIES || Instant::now() >= deadline || candidates.is_empty() {
            break;
        }
        let contact = candidates.remove(0);
        visited.insert(contact);
        result.queries += 1;
        let timeout = RPC_TIMEOUT.min(deadline.saturating_duration_since(Instant::now()));
        let mut returned_target = false;
        let attempt = query_with_connector(connector, contact, Message::Find(target), timeout)
            .and_then(|response| {
                let Message::Peers(records) = response else {
                    return Err(invalid("expected DHT records"));
                };
                returned_target = records.iter().any(|r| r.id() == target);
                service.accept_many(records, now()?)?;
                service.observed(contact, now()?)
            });
        if attempt.is_err() {
            result.failures += 1;
            service.failed(contact)?;
        }
        let state = service.snapshot()?;
        if attempt.is_ok() && returned_target {
            result.record = state.get(target, now()?);
        }
        if result.record.is_some() {
            break;
        }
        candidates.extend(service.candidates(now()?)?);
    }
    if result.record.is_none() {
        result.record = service.snapshot()?.get(target, now()?);
    }
    Ok(result)
}
/// One bounded initial STORE + FIND pass to up to eight configured ordinary
/// peers (at most 16 RPCs). No periodic republishing or background crawling.
pub fn publish(service: &Service, record: &PeerRecord, seeds: &[Contact]) -> io::Result<usize> {
    publish_with_connector(service, record, seeds, &PinnedTls)
}
pub fn publish_with_connector(
    service: &Service,
    record: &PeerRecord,
    seeds: &[Contact],
    connector: &dyn RpcConnector,
) -> io::Result<usize> {
    let network = service.snapshot()?.network;
    record.verify(network, now()?)?;
    let deadline = Instant::now() + LOOKUP_TIMEOUT;
    let mut successes = 0;
    for seed in seeds.iter().take(crate::MAX_REPLY_RECORDS) {
        seed.validate(network)?;
        if Instant::now() >= deadline {
            break;
        }
        if query_with_connector(
            connector,
            *seed,
            Message::Store(record.clone()),
            RPC_TIMEOUT.min(deadline.saturating_duration_since(Instant::now())),
        )
        .is_ok()
        {
            service.observed(*seed, now()?)?;
            successes += 1;
            if Instant::now() < deadline {
                if let Ok(Message::Peers(records)) = query_with_connector(
                    connector,
                    *seed,
                    Message::Find(record.id()),
                    RPC_TIMEOUT.min(deadline.saturating_duration_since(Instant::now())),
                ) {
                    // Failure to learn neighbors does not undo an acknowledged
                    // publication. All learned records still pass full checks.
                    let _ = service.accept_many(records, now()?);
                }
            }
        } else {
            service.failed(*seed)?;
        }
    }
    Ok(successes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, sync::Arc, thread};
    use yami_identity::NodeIdentity;
    use ynp::Incoming;
    use ynp_tls::TlsListener;
    fn malicious(mode: u8) -> (Contact, thread::JoinHandle<()>) {
        let identity = Arc::new(NodeIdentity::generate().unwrap());
        let id = *identity.id();
        let listener = TlsListener::bind("127.0.0.1:0".parse().unwrap(), identity).unwrap();
        let std::net::SocketAddr::V4(endpoint) = listener.local_addr().unwrap() else {
            unreachable!()
        };
        let task = thread::spawn(move || {
            let mut connection = listener.accept().unwrap();
            let (mut nonce, _) = wire::read(&mut *connection).unwrap();
            match mode {
                0 => {
                    nonce[0] ^= 1;
                    wire::write(&mut *connection, nonce, &Message::Peers(Vec::new())).unwrap();
                }
                1 => {
                    wire::write(&mut *connection, nonce, &Message::Stored).unwrap();
                }
                2 => {
                    let mut bytes = wire::encode(nonce, &Message::Peers(Vec::new())).unwrap();
                    bytes[crate::MAX_FRAME - 1] = 1;
                    connection.write_all(&bytes).unwrap();
                }
                _ => {
                    connection.write_all(b"Y").unwrap();
                    thread::sleep(Duration::from_millis(400));
                }
            }
        });
        (Contact { id, endpoint }, task)
    }
    #[test]
    fn unsolicited_wrong_type_and_malformed_responses_are_rejected() {
        let target = *NodeIdentity::generate().unwrap().id();
        for mode in 0..3 {
            let (peer, task) = malicious(mode);
            assert!(query(peer, Message::Find(target), Duration::from_secs(2)).is_err());
            task.join().unwrap();
        }
    }
    #[test]
    fn partial_response_cannot_exceed_rpc_deadline() {
        let (peer, task) = malicious(3);
        let started = Instant::now();
        let result = query(peer, Message::Find(peer.id), Duration::from_millis(200));
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
        task.join().unwrap();
    }
}
