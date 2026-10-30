use super::*;
use std::{fs, io, net::SocketAddrV4, path::PathBuf};
use yami_identity::NodeIdentity;
fn identity() -> NodeIdentity {
    NodeIdentity::generate().unwrap()
}
fn record(identity: &NodeIdentity, sequence: u64, expires: u64, now: u64) -> PeerRecord {
    PeerRecord::sign(
        identity,
        vec!["127.0.0.1:7443".parse().unwrap()],
        expires,
        sequence,
        Network::Loopback,
        now,
    )
    .unwrap()
}
struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("yami-dht-{name}-{}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn records_are_canonical_signed_expiring_and_policy_scoped() {
    let owner = identity();
    let at = 10_000;
    let signed = record(&owner, 4, at + 60, at);
    assert_eq!(PeerRecord::decode(&signed.encode()).unwrap(), signed);
    assert_eq!(signed.id(), *owner.id());
    signed.verify(Network::Loopback, at).unwrap();
    assert!(signed.verify(Network::Public, at).is_err());
    assert!(signed.verify(Network::Loopback, at + 60).is_err());
    assert!(signed.verify(Network::Loopback, 1).is_ok());
    for offset in [0, 32, 40, 49, 53, 55, signed.encode().len() - 1] {
        let mut bytes = signed.encode();
        bytes[offset] ^= 1;
        if let Ok(bad) = PeerRecord::decode(&bytes) {
            assert!(bad.verify(Network::Loopback, at).is_err(), "{offset}");
        }
    }
    for size in 0..signed.encode().len() {
        assert!(PeerRecord::decode(&signed.encode()[..size]).is_err());
    }
    let mut extra = signed.encode();
    extra.push(0);
    assert!(PeerRecord::decode(&extra).is_err());
    for endpoint in [
        "0.0.0.0:1",
        "127.0.0.1:0",
        "192.168.1.2:1",
        "169.254.1.1:1",
        "8.8.8.8:1",
    ] {
        assert!(PeerRecord::sign(
            &owner,
            vec![endpoint.parse().unwrap()],
            at + 10,
            1,
            Network::Loopback,
            at
        )
        .is_err());
    }
    for endpoint in [
        "127.0.0.1:1",
        "192.168.1.2:1",
        "100.64.0.1:1",
        "198.51.100.1:1",
        "224.0.0.1:1",
    ] {
        assert!(PeerRecord::sign(
            &owner,
            vec![endpoint.parse().unwrap()],
            at + 10,
            1,
            Network::Public,
            at
        )
        .is_err());
    }
    assert!(PeerRecord::sign(
        &owner,
        vec!["8.8.8.8:7443".parse().unwrap()],
        at + 10,
        1,
        Network::Public,
        at
    )
    .is_ok());
    let endpoint: SocketAddrV4 = "127.0.0.1:1".parse().unwrap();
    for endpoints in [vec![], vec![endpoint; 2], vec![endpoint; 5]] {
        assert!(PeerRecord::sign(&owner, endpoints, at + 10, 1, Network::Loopback, at).is_err());
    }
    assert!(PeerRecord::sign(
        &owner,
        vec![endpoint],
        at + MAX_RECORD_TTL + 1,
        1,
        Network::Loopback,
        at
    )
    .is_err());
    assert!(PeerRecord::sign(&owner, vec![endpoint], at + 10, 0, Network::Loopback, at).is_err());
}

#[test]
fn rollback_and_equivocation_are_rejected_even_after_expiry() {
    let owner = identity();
    let mut state = State::new(*identity().id(), Network::Loopback);
    let old = record(&owner, 1, 200, 100);
    let newer = record(&owner, 2, 150, 100);
    assert!(state.accept(newer.clone(), 100).unwrap());
    assert!(!state.accept(newer, 100).unwrap());
    assert!(state.accept(old.clone(), 100).is_err());
    assert!(state.get(*owner.id(), 151).is_none());
    assert!(
        state.accept(old, 151).is_err(),
        "expiry cannot erase sequence history"
    );
    assert!(
        state.accept(record(&owner, 2, 201, 151), 151).is_err(),
        "equal sequence with changed fields"
    );
    assert!(state.accept(record(&owner, 3, 201, 151), 151).unwrap());
    assert!(state.get(*owner.id(), 202).is_none());
    assert!(
        state.accept(record(&owner, 4, 203, 200), 203).is_err(),
        "expired on acceptance"
    );
}

#[test]
fn cache_and_xor_buckets_stay_bounded_and_routes_expire() {
    let local = *identity().id();
    let mut state = State::new(local, Network::Loopback);
    for _ in 0..MAX_CACHE {
        state.accept(record(&identity(), 1, 200, 100), 100).unwrap();
    }
    assert_eq!(state.records().len(), MAX_CACHE);
    assert!(state.accept(record(&identity(), 1, 200, 100), 100).is_err());
    let mut routing = RoutingTable::new(local);
    let mut accepted = 0;
    for _ in 0..128 {
        let id = *identity().id();
        if routing::bucket(local, id) == Some(0) {
            let c = Contact::new(id, "127.0.0.1:7443".parse().unwrap(), Network::Loopback).unwrap();
            if accepted < BUCKET_SIZE {
                routing.observed(c, 100).unwrap();
                accepted += 1;
            } else {
                assert!(routing.observed(c, 100).is_err());
            }
        }
    }
    assert_eq!(accepted, BUCKET_SIZE);
    assert_eq!(routing.contacts().len(), BUCKET_SIZE);
    routing.expire(100 + routing::ROUTE_TTL);
    assert!(routing.contacts().is_empty());
    assert_eq!(distance(local, local), [0; 32]);
    let other = *identity().id();
    assert_eq!(distance(local, other), distance(other, local));
    assert_ne!(distance(local, other), [0; 32]);
}

#[test]
fn persisted_signed_high_water_and_self_sequence_survive_restart() {
    let scratch = Scratch::new("restart");
    let path = scratch.0.join("node.cache");
    let owner = identity();
    let at = now().unwrap();
    let service = Service::open(&path, *owner.id(), Network::Loopback).unwrap();
    assert!(Service::open(&path, *owner.id(), Network::Loopback).is_err());
    let first = service
        .prepare_self(&owner, vec!["127.0.0.1:7443".parse().unwrap()], at)
        .unwrap();
    let other = identity();
    let before = record(&other, 1, at + 500, at);
    let latest = record(&other, 2, at + 2, at);
    service.accept(latest, at).unwrap();
    drop(service);
    let service = Service::open(&path, *owner.id(), Network::Loopback).unwrap();
    assert!(service.accept(before, at + 3).is_err());
    let second = service
        .prepare_self(&owner, vec!["127.0.0.1:7444".parse().unwrap()], at + 3)
        .unwrap();
    assert_eq!(first.sequence() + 1, second.sequence());
    assert_eq!(
        service
            .snapshot()
            .unwrap()
            .get(*owner.id(), at + 3)
            .unwrap(),
        second
    );
    let original = fs::read(&path).unwrap();
    fs::write(path.with_extension("tmp"), b"stale file").unwrap();
    assert!(service
        .accept(record(&identity(), 1, at + 100, at), at)
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(service.snapshot().unwrap().records().len(), 2);
    drop(service);
    let mut corrupted = original;
    let last = corrupted.len() - 1;
    corrupted[last] ^= 1;
    fs::write(&path, corrupted).unwrap();
    assert!(Service::open(&path, *owner.id(), Network::Loopback).is_err());
}

#[test]
fn wire_limits_versions_padding_duplicates_and_arbitrary_bytes_fail_closed() {
    use wire::{decode, encode, Message};
    let node = *identity().id();
    let query = encode([1; 16], &Message::Find(node)).unwrap();
    assert!(matches!(decode(&query).unwrap().1,Message::Find(id) if id==node));
    for size in 0..MAX_FRAME {
        assert!(decode(&query[..size]).is_err());
    }
    for (offset, value) in [(0, 0), (4, 2), (5, 7), (22, 255), (MAX_FRAME - 1, 1)] {
        let mut packet = query;
        packet[offset] = value;
        assert!(decode(&packet).is_err());
    }
    let signed = record(&identity(), 1, 200, 100);
    assert!(decode(&encode([1; 16], &Message::Peers(vec![signed.clone(); 2])).unwrap()).is_err());
    assert!(encode(
        [1; 16],
        &Message::Peers(vec![signed; MAX_REPLY_RECORDS + 1])
    )
    .is_err());
    let mut seed = 7u64;
    for _ in 0..100 {
        let mut bytes = [0; MAX_FRAME];
        for b in &mut bytes {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            *b = (seed >> 32) as u8;
        }
        assert!(decode(&bytes).is_err());
    }
}

#[test]
fn unavailable_bootstraps_obey_query_budget_without_crashing() {
    let scratch = Scratch::new("budget");
    let owner = *identity().id();
    let service = Service::open(scratch.0.join("node.cache"), owner, Network::Loopback).unwrap();
    let reserve = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let std::net::SocketAddr::V4(endpoint) = reserve.local_addr().unwrap() else {
        unreachable!()
    };
    drop(reserve);
    let seeds: Vec<_> = (0..2)
        .map(|_| Contact::new(*identity().id(), endpoint, Network::Loopback).unwrap())
        .collect();
    let started = std::time::Instant::now();
    let found = lookup(&service, *identity().id(), &seeds).unwrap();
    assert!(found.record.is_none());
    assert!(found.queries > 0 && found.queries <= MAX_QUERIES);
    assert_eq!(found.failures, found.queries);
    assert!(started.elapsed() < LOOKUP_TIMEOUT + std::time::Duration::from_secs(1));
    assert!(service.snapshot().unwrap().routing.contacts().is_empty());
    assert!(lookup(
        &service,
        owner,
        &[Contact {
            id: owner,
            endpoint: "8.8.8.8:1".parse().unwrap()
        }]
    )
    .is_err());
}

#[test]
fn batch_acceptance_is_atomic_on_invalid_records() {
    let scratch = Scratch::new("batch");
    let service = Service::open(
        scratch.0.join("node.cache"),
        *identity().id(),
        Network::Loopback,
    )
    .unwrap();
    let good = record(&identity(), 1, 200, 100);
    let mut bytes = record(&identity(), 1, 200, 100).encode();
    *bytes.last_mut().unwrap() ^= 1;
    let bad = PeerRecord::decode(&bytes).unwrap();
    assert!(service.accept_records(vec![good, bad], 100).is_err());
    assert!(service.snapshot().unwrap().records().is_empty());
}

#[test]
fn connector_query_budget_is_exact_and_invalid_addresses_never_reach_it() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Refused(AtomicUsize);
    impl RpcConnector for Refused {
        fn connect(
            &self,
            _: Contact,
            budget: std::time::Duration,
        ) -> io::Result<Box<dyn ynp::Connection>> {
            assert!(budget > std::time::Duration::ZERO && budget <= RPC_TIMEOUT);
            self.0.fetch_add(1, Ordering::Relaxed);
            Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "simulated unavailable peer",
            ))
        }
    }
    let scratch = Scratch::new("connector");
    let service = Service::open(
        scratch.0.join("node.cache"),
        *identity().id(),
        Network::Loopback,
    )
    .unwrap();
    let seeds: Vec<_> = (0..40)
        .map(|_| {
            Contact::new(
                *identity().id(),
                "127.0.0.1:1".parse().unwrap(),
                Network::Loopback,
            )
            .unwrap()
        })
        .collect();
    let connector = Refused(AtomicUsize::new(0));
    let found = lookup_with_connector(&service, *identity().id(), &seeds, &connector).unwrap();
    assert_eq!(found.queries, MAX_QUERIES);
    assert_eq!(found.failures, MAX_QUERIES);
    assert_eq!(connector.0.load(Ordering::Relaxed), MAX_QUERIES);
    let bad = [Contact {
        id: *identity().id(),
        endpoint: "192.168.1.1:7443".parse().unwrap(),
    }];
    assert!(lookup_with_connector(&service, *identity().id(), &bad, &connector).is_err());
    assert_eq!(connector.0.load(Ordering::Relaxed), MAX_QUERIES);
}

fn rpc_server(service: Service, listener: ynp_tls::TlsListener) -> std::thread::JoinHandle<()> {
    use std::io::Read;
    use ynp::Incoming;
    std::thread::spawn(move || {
        let mut connection = listener.accept().unwrap();
        let mut prefix = [0; 4];
        connection.read_exact(&mut prefix).unwrap();
        service.handle(&mut *connection, prefix).unwrap();
    })
}
#[test]
fn iterative_lookup_follows_an_intermediate_peer_and_updates_routes() {
    use std::sync::Arc;
    let scratch = Scratch::new("iterative");
    let at = now().unwrap();
    let b = Arc::new(identity());
    let d = Arc::new(identity());
    let a = identity();
    let c = identity();
    let bl = ynp_tls::TlsListener::bind("127.0.0.1:0".parse().unwrap(), Arc::clone(&b)).unwrap();
    let dl = ynp_tls::TlsListener::bind("127.0.0.1:0".parse().unwrap(), Arc::clone(&d)).unwrap();
    let std::net::SocketAddr::V4(be) = bl.local_addr().unwrap() else {
        unreachable!()
    };
    let std::net::SocketAddr::V4(de) = dl.local_addr().unwrap() else {
        unreachable!()
    };
    let bs = Service::open(scratch.0.join("b.cache"), *b.id(), Network::Loopback).unwrap();
    let ds = Service::open(scratch.0.join("d.cache"), *d.id(), Network::Loopback).unwrap();
    let cs = Service::open(scratch.0.join("c.cache"), *c.id(), Network::Loopback).unwrap();
    bs.prepare_self(&b, vec![be], at).unwrap();
    let d_record = ds.prepare_self(&d, vec![de], at).unwrap();
    bs.accept(d_record, at).unwrap();
    let a_record = record(&a, 1, at + 60, at);
    ds.accept(a_record.clone(), at).unwrap();
    let bt = rpc_server(bs, bl);
    let dt = rpc_server(ds, dl);
    let found = lookup(
        &cs,
        *a.id(),
        &[Contact::new(*b.id(), be, Network::Loopback).unwrap()],
    )
    .unwrap();
    assert_eq!(found.queries, 2);
    assert_eq!(found.record, Some(a_record));
    let routes = cs.snapshot().unwrap().routing.contacts();
    assert!(routes.iter().any(|p| p.id == *b.id()));
    assert!(routes.iter().any(|p| p.id == *d.id()));
    bt.join().unwrap();
    dt.join().unwrap();
}

#[test]
fn lookup_refreshes_an_unavailable_cached_endpoint_from_a_newer_record() {
    use std::sync::Arc;
    let scratch = Scratch::new("refresh");
    let at = now().unwrap();
    let owner = identity();
    let bootstrap = Arc::new(identity());
    let reserve = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let std::net::SocketAddr::V4(old) = reserve.local_addr().unwrap() else {
        unreachable!()
    };
    drop(reserve);
    let listener =
        ynp_tls::TlsListener::bind("127.0.0.1:0".parse().unwrap(), Arc::clone(&bootstrap)).unwrap();
    let std::net::SocketAddr::V4(endpoint) = listener.local_addr().unwrap() else {
        unreachable!()
    };
    let server = Service::open(
        scratch.0.join("server.cache"),
        *bootstrap.id(),
        Network::Loopback,
    )
    .unwrap();
    let client = Service::open(
        scratch.0.join("client.cache"),
        *identity().id(),
        Network::Loopback,
    )
    .unwrap();
    let previous = PeerRecord::sign(&owner, vec![old], at + 60, 1, Network::Loopback, at).unwrap();
    let current = record(&owner, 2, at + 60, at);
    client.accept(previous, at).unwrap();
    server.accept(current.clone(), at).unwrap();
    let task = rpc_server(server, listener);
    let found = lookup(
        &client,
        *owner.id(),
        &[Contact::new(*bootstrap.id(), endpoint, Network::Loopback).unwrap()],
    )
    .unwrap();
    assert_eq!(found.record, Some(current));
    assert!(found.failures >= 1);
    task.join().unwrap();
}

#[test]
fn relay_v2_binds_owner_relay_endpoint_and_preserves_v1() {
    let owner = identity();
    let relay = identity();
    let route = RelayRoute {
        relay: *relay.id(),
        endpoint: "127.0.0.1:7444".parse().unwrap(),
    };
    let signed = PeerRecord::sign_routes(
        &owner,
        vec![],
        vec![route],
        1060,
        2,
        Network::Loopback,
        1000,
    )
    .unwrap();
    assert!(signed.endpoints().is_empty());
    assert_eq!(signed.relay_routes(), &[route]);
    assert_eq!(PeerRecord::decode(&signed.encode()).unwrap(), signed);
    assert!(signed.verify(Network::Public, 1000).is_err());
    assert!(signed.verify(Network::Loopback, 1060).is_err());
    for offset in 0..signed.encode().len() {
        let mut bytes = signed.encode();
        bytes[offset] ^= 1;
        if let Ok(changed) = PeerRecord::decode(&bytes) {
            assert!(changed.verify(Network::Loopback, 1000).is_err(), "{offset}");
        }
    }
    for end in 0..signed.encode().len() {
        assert!(PeerRecord::decode(&signed.encode()[..end]).is_err());
    }
    let legacy = record(&owner, 1, 1060, 1000);
    assert_eq!(PeerRecord::decode(&legacy.encode()).unwrap(), legacy);
    let mut state = State::new(*owner.id(), Network::Loopback);
    state.accept(legacy.clone(), 1000).unwrap();
    state.accept(signed, 1000).unwrap();
    assert!(state.accept(legacy, 1000).is_err());
    let withdrawn =
        PeerRecord::sign_routes(&owner, vec![], vec![], 1060, 3, Network::Loopback, 1000).unwrap();
    state.accept(withdrawn, 1000).unwrap();
    assert!(state
        .get(*owner.id(), 1000)
        .unwrap()
        .relay_routes()
        .is_empty());
}

#[test]
fn maximal_relay_records_fit_existing_frame_and_snapshot_budgets() {
    let relay = identity();
    let routes = vec![
        RelayRoute {
            relay: *relay.id(),
            endpoint: "127.0.0.1:3".parse().unwrap(),
        },
        RelayRoute {
            relay: *relay.id(),
            endpoint: "127.0.0.1:4".parse().unwrap(),
        },
    ];
    let mut records = Vec::new();
    for _ in 0..MAX_REPLY_RECORDS {
        let owner = identity();
        records.push(
            PeerRecord::sign_routes(
                &owner,
                (5..9)
                    .map(|p| SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, p))
                    .collect(),
                routes.clone(),
                1060,
                1,
                Network::Loopback,
                1000,
            )
            .unwrap(),
        );
    }
    assert_eq!(records[0].encode().len(), MAX_RECORD_BYTES);
    assert_eq!(
        PeerRecord::decode(&records[0].encode()).unwrap(),
        records[0]
    );
    wire::encode([0; 16], &wire::Message::Peers(records)).unwrap();
    assert!(
        39 + MAX_CACHE
            * (2 + PeerRecord::sign_routes(
                &identity(),
                vec![],
                routes,
                1060,
                1,
                Network::Loopback,
                1000
            )
            .unwrap()
            .encode()
            .len())
            < 64 * 1024
    );
}
