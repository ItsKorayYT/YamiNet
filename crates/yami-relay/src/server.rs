use crate::{
    invalid, now, timeout,
    wire::{self, Frame},
    CHALLENGE_TTL, MAX_TUNNEL_BYTES, REGISTRATION_LEASE, TUNNEL_LIFETIME,
};
use std::{
    collections::HashMap,
    io::{self, Read},
    net::{IpAddr, SocketAddr},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use yami_identity::{NodeId, NodeIdentity, RelayRegistration, Signature};
use ynp_tls::{SecureStream, TlsListener};

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub connections: usize,
    pub per_ip: usize,
    pub registrations: usize,
    pub tunnels: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            connections: 32,
            per_ip: 8,
            registrations: 8,
            tunnels: 4,
        }
    }
}
impl Limits {
    fn validate(self) -> io::Result<Self> {
        if self.connections == 0
            || self.connections > 32
            || self.per_ip == 0
            || self.per_ip > self.connections
            || self.registrations == 0
            || self.registrations > 8
            || self.tunnels == 0
            || self.tunnels > 4
        {
            return Err(invalid("invalid relay resource limits"));
        }
        Ok(self)
    }
}
struct Entry {
    generation: u64,
    until: Instant,
    busy: bool,
    sender: SyncSender<Request>,
}
struct State {
    connections: usize,
    per_ip: HashMap<IpAddr, usize>,
    rates: HashMap<IpAddr, (Instant, usize)>,
    entries: HashMap<NodeId, Entry>,
    tunnels: usize,
    next_generation: u64,
}
struct Shared {
    state: Mutex<State>,
    limits: Limits,
    forwarded: AtomicU64,
    stopped: AtomicBool,
}
#[derive(Debug, Clone, Copy)]
pub struct Statistics {
    pub connections: usize,
    pub registrations: usize,
    pub tunnels: usize,
    pub forwarded_bytes: u64,
}

pub struct Relay {
    listener: TlsListener,
    id: NodeId,
    shared: Arc<Shared>,
}
impl Relay {
    pub fn bind(
        endpoint: SocketAddr,
        identity: Arc<NodeIdentity>,
        limits: Limits,
    ) -> io::Result<Self> {
        ynp::require_loopback(endpoint)?;
        Self::bind_mode(endpoint, identity, limits, false)
    }
    /// Explicit public listening mode. No wildcard bind, implicit identity or endpoint discovery.
    pub fn bind_public(
        endpoint: SocketAddr,
        identity: Arc<NodeIdentity>,
        limits: Limits,
    ) -> io::Result<Self> {
        if !endpoint.is_ipv4() {
            return Err(invalid("public relay requires a specific IPv4 address"));
        }
        Self::bind_mode(endpoint, identity, limits, true)
    }
    fn bind_mode(
        endpoint: SocketAddr,
        identity: Arc<NodeIdentity>,
        limits: Limits,
        public: bool,
    ) -> io::Result<Self> {
        let limits = limits.validate()?;
        let id = *identity.id();
        let listener = if public {
            TlsListener::bind_public(endpoint, identity)?
        } else {
            TlsListener::bind(endpoint, identity)?
        };
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            id,
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    connections: 0,
                    per_ip: HashMap::new(),
                    rates: HashMap::new(),
                    entries: HashMap::new(),
                    tunnels: 0,
                    next_generation: 0,
                }),
                limits,
                forwarded: AtomicU64::new(0),
                stopped: AtomicBool::new(false),
            }),
        })
    }
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }
    pub fn statistics(&self) -> Statistics {
        let state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        Statistics {
            connections: state.connections,
            registrations: state.entries.len(),
            tunnels: state.tunnels,
            forwarded_bytes: self.shared.forwarded.load(Ordering::Relaxed),
        }
    }
    /// Bounded tracked tasks, joined on shutdown. No detached connection threads.
    pub fn run(&self, stop: &AtomicBool) -> io::Result<()> {
        let mut tasks: Vec<thread::JoinHandle<()>> = Vec::new();
        while !stop.load(Ordering::SeqCst) {
            let mut index = 0;
            while index < tasks.len() {
                if tasks[index].is_finished() {
                    let _ = tasks.swap_remove(index).join();
                } else {
                    index += 1;
                }
            }
            if tasks.len() >= self.shared.limits.connections {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            match self.listener.accept_session_from(ynp_tls::MAX_SESSION) {
                Ok((peer, tls)) => {
                    let Some(lease) = ConnectionLease::acquire(self.shared.clone(), peer.ip())
                    else {
                        drop(tls);
                        continue;
                    };
                    let shared = self.shared.clone();
                    let id = self.id;
                    let session = Session { tls, _lease: lease };
                    let task = thread::Builder::new()
                        .name("yami-relay-session".into())
                        .spawn(move || {
                            let _ = handle(session, id, shared);
                        });
                    match task {
                        Ok(task) => tasks.push(task),
                        Err(error) => {
                            self.shared.stopping();
                            for task in tasks {
                                let _ = task.join();
                            }
                            return Err(error);
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10))
                }
                Err(error) => {
                    self.shared.stopping();
                    for task in tasks {
                        let _ = task.join();
                    }
                    return Err(error);
                }
            }
        }
        // A stop marker makes idle registrations and active pumps stop between
        // bounded polls. Handshake and control each have a five-second budget.
        self.shared.stopping();
        for task in tasks {
            let _ = task.join();
        }
        Ok(())
    }
}
impl Shared {
    fn stopping(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
    fn stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }
}
struct ConnectionLease {
    shared: Arc<Shared>,
    ip: IpAddr,
}
impl ConnectionLease {
    fn acquire(shared: Arc<Shared>, ip: IpAddr) -> Option<Self> {
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        let at = Instant::now();
        state
            .rates
            .retain(|_, (start, _)| at.duration_since(*start) < Duration::from_secs(1));
        if state.connections >= shared.limits.connections
            || state.per_ip.get(&ip).copied().unwrap_or(0) >= shared.limits.per_ip
            || (!state.rates.contains_key(&ip) && state.rates.len() >= 64)
        {
            return None;
        }
        let rate = state.rates.entry(ip).or_insert((at, 0));
        if rate.1 >= 16 {
            return None;
        }
        rate.1 += 1;
        state.connections += 1;
        *state.per_ip.entry(ip).or_default() += 1;
        drop(state);
        Some(Self { shared, ip })
    }
}
impl Drop for ConnectionLease {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        state.connections -= 1;
        if let Some(count) = state.per_ip.get_mut(&self.ip) {
            *count -= 1;
            if *count == 0 {
                state.per_ip.remove(&self.ip);
            }
        }
    }
}
struct Session {
    tls: Box<dyn SecureStream>,
    _lease: ConnectionLease,
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.tls.close();
    }
}
struct TunnelLease(Arc<Shared>);
impl Drop for TunnelLease {
    fn drop(&mut self) {
        self.0
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tunnels -= 1;
    }
}
struct Request {
    client: Session,
    _tunnel: TunnelLease,
}
struct RegistrationLease {
    shared: Arc<Shared>,
    host: NodeId,
    generation: u64,
}
impl Drop for RegistrationLease {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if state
            .entries
            .get(&self.host)
            .is_some_and(|e| e.generation == self.generation)
        {
            state.entries.remove(&self.host);
        }
    }
}
fn refuse(session: &mut Session, code: u8) -> io::Result<()> {
    wire::write(&mut *session.tls, &Frame::Error(code))
}
fn handle(mut session: Session, relay: NodeId, shared: Arc<Shared>) -> io::Result<()> {
    session.tls.complete_handshake()?;
    let until = Instant::now() + Duration::from_secs(CHALLENGE_TTL);
    let request = wire::read(&mut crate::Control {
        stream: &mut *session.tls,
        until,
    });
    match request {
        Ok(Frame::Register(key)) => register(
            session,
            relay,
            NodeId::from_public_key(key).map_err(io::Error::other)?,
            shared,
            until,
        ),
        Ok(Frame::Connect(key)) => pair(
            session,
            NodeId::from_public_key(key).map_err(io::Error::other)?,
            shared,
        ),
        _ => {
            let _ = refuse(&mut session, 4);
            Err(invalid("invalid initial relay request"))
        }
    }
}
fn register(
    mut host: Session,
    relay: NodeId,
    id: NodeId,
    shared: Arc<Shared>,
    until: Instant,
) -> io::Result<()> {
    let mut nonce = [0; 32];
    getrandom::fill(&mut nonce).map_err(io::Error::other)?;
    let expires = now()?.saturating_add(CHALLENGE_TTL);
    wire::write(
        &mut crate::Control {
            stream: &mut *host.tls,
            until,
        },
        &Frame::Challenge(nonce, expires),
    )?;
    let challenge = RelayRegistration {
        relay,
        host: id,
        nonce,
        channel: host.tls.channel_binding()?,
        expires,
    };
    let proof = wire::read(&mut crate::Control {
        stream: &mut *host.tls,
        until,
    })?;
    let valid = matches!(proof, Frame::Proof(signature) if verify_proof(&challenge, &Signature::from_bytes(signature), now()?).is_ok());
    if !valid {
        let _ = refuse(&mut host, 3);
        return Err(invalid("relay registration proof rejected"));
    }
    let (sender, receive) = mpsc::sync_channel(1);
    let lease = {
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        let error = if state.entries.contains_key(&id) {
            Some(1)
        } else if state.entries.len() >= shared.limits.registrations
            || state.next_generation == u64::MAX
        {
            Some(5)
        } else {
            None
        };
        if let Some(code) = error {
            drop(state);
            let _ = refuse(&mut host, code);
            return Err(invalid("relay registration conflict/capacity"));
        }
        state.next_generation += 1;
        let generation = state.next_generation;
        state.entries.insert(
            id,
            Entry {
                generation,
                until: Instant::now() + REGISTRATION_LEASE,
                busy: false,
                sender,
            },
        );
        RegistrationLease {
            shared: shared.clone(),
            host: id,
            generation,
        }
    };
    wire::write(
        &mut *host.tls,
        &Frame::Registered(now()?.saturating_add(REGISTRATION_LEASE.as_secs())),
    )?;
    wait_for_client(&mut host, id, receive, &shared)?;
    drop(lease);
    Ok(())
}
fn verify_proof(challenge: &RelayRegistration, signature: &Signature, at: u64) -> io::Result<()> {
    if challenge.expires <= at || challenge.expires > at.saturating_add(CHALLENGE_TTL) {
        return Err(invalid("expired relay challenge"));
    }
    challenge
        .host
        .verify(challenge, signature)
        .map_err(io::Error::other)
}
fn wait_for_client(
    host: &mut Session,
    id: NodeId,
    receive: Receiver<Request>,
    shared: &Shared,
) -> io::Result<()> {
    let until = Instant::now() + REGISTRATION_LEASE;
    host.tls.read_timeout(Duration::from_millis(25))?;
    while Instant::now() < until && !shared.stopped() {
        if let Ok(mut request) = receive.try_recv() {
            wire::write(&mut *host.tls, &Frame::Go(id.public_key()))?;
            wire::write(&mut *request.client.tls, &Frame::Paired(id.public_key()))?;
            return pump(&mut *host.tls, &mut *request.client.tls, shared);
        }
        // The host must remain silent until GO. Polling also detects dead
        // registrations promptly instead of leaving a stale ID claim behind.
        match host.tls.read(&mut [0; 1]) {
            Err(error) if timeout(&error) => {}
            _ => {
                return Err(invalid(
                    "registered host disconnected or sent unsolicited bytes",
                ))
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "relay registration lease ended",
    ))
}
fn pair(mut client: Session, id: NodeId, shared: Arc<Shared>) -> io::Result<()> {
    let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
    let code = match state.entries.get(&id) {
        None => Some(2),
        Some(entry) if entry.busy => Some(1),
        Some(entry) if entry.until <= Instant::now() => Some(6),
        _ => None,
    }
    .or_else(|| (state.tunnels >= shared.limits.tunnels).then_some(5));
    if let Some(code) = code {
        drop(state);
        return refuse(&mut client, code);
    }
    state.tunnels += 1;
    let entry = state
        .entries
        .get_mut(&id)
        .ok_or_else(|| invalid("registration disappeared"))?;
    entry.busy = true;
    let sender = entry.sender.clone();
    drop(state);
    let request = Request {
        client,
        _tunnel: TunnelLease(shared),
    };
    if let Err(error) = sender.try_send(request) {
        let (mpsc::TrySendError::Full(mut request) | mpsc::TrySendError::Disconnected(mut request)) =
            error;
        let _ = refuse(&mut request.client, 2);
    }
    Ok(())
}
fn pump(
    host: &mut dyn SecureStream,
    client: &mut dyn SecureStream,
    shared: &Shared,
) -> io::Result<()> {
    host.read_timeout(Duration::from_millis(10))?;
    client.read_timeout(Duration::from_millis(10))?;
    host.write_timeout(Duration::from_secs(1))?;
    client.write_timeout(Duration::from_secs(1))?;
    let until = Instant::now() + TUNNEL_LIFETIME;
    let mut buffer = [0; 8192];
    let mut total = 0;
    let mut window = (Instant::now(), 0usize);
    let mut activity = Instant::now();
    while Instant::now() < until && !shared.stopped() {
        if window.0.elapsed() >= Duration::from_secs(1) {
            window = (Instant::now(), total);
        }
        let before = total;
        if !forward(host, client, &mut buffer, &mut total, shared)? {
            return Ok(());
        }
        if !forward(client, host, &mut buffer, &mut total, shared)? {
            return Ok(());
        }
        if total != before {
            activity = Instant::now();
        }
        if total - window.1 > 256 * 1024 {
            return Err(invalid("relay tunnel bandwidth quota exceeded"));
        }
        if activity.elapsed() >= Duration::from_secs(5) {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "relay tunnel idle timeout",
            ));
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "relay tunnel lifetime ended",
    ))
}

fn forward(
    from: &mut dyn SecureStream,
    to: &mut dyn SecureStream,
    buffer: &mut [u8],
    total: &mut usize,
    shared: &Shared,
) -> io::Result<bool> {
    match from.read(buffer) {
        Ok(0) => Ok(false),
        Ok(length) => {
            *total += length;
            if *total > MAX_TUNNEL_BYTES {
                return Err(invalid("relay tunnel byte limit exceeded"));
            }
            to.write_all(&buffer[..length])?;
            to.flush()?;
            shared.forwarded.fetch_add(length as u64, Ordering::Relaxed);
            Ok(true)
        }
        Err(error) if timeout(&error) => Ok(true),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Registration, Route};
    use ynp_tls::TlsTransport;
    struct Running {
        relay: Arc<Relay>,
        stop: Arc<AtomicBool>,
        task: Option<thread::JoinHandle<io::Result<()>>>,
    }
    impl Running {
        fn start(limits: Limits) -> Self {
            let relay = Arc::new(
                Relay::bind(
                    "127.0.0.1:0".parse().unwrap(),
                    Arc::new(NodeIdentity::generate().unwrap()),
                    limits,
                )
                .unwrap(),
            );
            let stop = Arc::new(AtomicBool::new(false));
            let flag = stop.clone();
            let server = relay.clone();
            let task = thread::spawn(move || server.run(&flag));
            Self {
                relay,
                stop,
                task: Some(task),
            }
        }
        fn route(&self, identity: &NodeIdentity) -> Route {
            Route {
                relay: self.relay.id,
                endpoint: self.relay.local_addr().unwrap(),
                destination: *identity.id(),
            }
        }
    }
    impl Drop for Running {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            self.task.take().unwrap().join().unwrap().unwrap();
        }
    }
    fn wait(mut condition: impl FnMut() -> bool) {
        let until = Instant::now() + Duration::from_secs(3);
        while !condition() {
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(10));
        }
    }
    fn challenge(stream: &mut dyn SecureStream, route: Route) -> RelayRegistration {
        wire::write(stream, &Frame::Register(route.destination.public_key())).unwrap();
        let Frame::Challenge(nonce, expires) = wire::read(stream).unwrap() else {
            panic!("challenge")
        };
        RelayRegistration {
            relay: route.relay,
            host: route.destination,
            nonce,
            channel: stream.channel_binding().unwrap(),
            expires,
        }
    }
    #[test]
    fn fresh_challenges_reject_replay_hijacking_expiration_and_channel_substitution() {
        let server = Running::start(Limits::default());
        let host = NodeIdentity::generate().unwrap();
        let route = server.route(&host);
        let mut first = TlsTransport::new(route.relay)
            .unwrap()
            .connect_session(route.endpoint, ynp_tls::MAX_SESSION)
            .unwrap();
        let signed = challenge(&mut *first, route);
        let signature = host.sign(&signed);
        assert!(verify_proof(&signed, &signature, now().unwrap()).is_ok());
        assert!(verify_proof(&signed, &signature, signed.expires).is_err());
        for field in 0..5 {
            let mut changed = signed.clone();
            match field {
                0 => changed.nonce[0] ^= 1,
                1 => changed.channel[0] ^= 1,
                2 => changed.relay = *NodeIdentity::generate().unwrap().id(),
                3 => changed.host = *NodeIdentity::generate().unwrap().id(),
                _ => changed.expires -= 1,
            }
            assert!(verify_proof(&changed, &signature, now().unwrap()).is_err());
        }
        wire::write(&mut *first, &Frame::Proof(signature.to_bytes())).unwrap();
        assert!(matches!(
            wire::read(&mut *first).unwrap(),
            Frame::Registered(_)
        ));
        let mut replay = TlsTransport::new(route.relay)
            .unwrap()
            .connect_session(route.endpoint, ynp_tls::MAX_SESSION)
            .unwrap();
        let fresh = challenge(&mut *replay, route);
        assert_ne!(fresh.nonce, signed.nonce);
        assert_ne!(fresh.channel, signed.channel);
        wire::write(&mut *replay, &Frame::Proof(signature.to_bytes())).unwrap();
        assert_eq!(wire::read(&mut *replay).unwrap(), Frame::Error(3));
        let mut hijack = TlsTransport::new(route.relay)
            .unwrap()
            .connect_session(route.endpoint, ynp_tls::MAX_SESSION)
            .unwrap();
        let fresh = challenge(&mut *hijack, route);
        let attacker = NodeIdentity::generate().unwrap();
        wire::write(
            &mut *hijack,
            &Frame::Proof(attacker.sign(&fresh).to_bytes()),
        )
        .unwrap();
        assert_eq!(wire::read(&mut *hijack).unwrap(), Frame::Error(3));
        assert_eq!(server.relay.statistics().registrations, 1);
    }
    #[test]
    fn conflicts_expiry_and_capacity_fail_closed_and_disconnect_frees_the_slot() {
        let server = Running::start(Limits {
            registrations: 1,
            ..Limits::default()
        });
        let a = NodeIdentity::generate().unwrap();
        let b = NodeIdentity::generate().unwrap();
        let route = server.route(&a);
        let registration = Registration::connect(route, &a).unwrap();
        assert!(Registration::connect(route, &a)
            .err()
            .unwrap()
            .to_string()
            .contains("conflicting"));
        assert!(Registration::connect(server.route(&b), &b)
            .err()
            .unwrap()
            .to_string()
            .contains("resource limit"));
        server
            .relay
            .shared
            .state
            .lock()
            .unwrap()
            .entries
            .get_mut(a.id())
            .unwrap()
            .until = Instant::now() - Duration::from_secs(1);
        assert!(route
            .open_tunnel()
            .err()
            .unwrap()
            .to_string()
            .contains("expired"));
        drop(registration);
        wait(|| server.relay.statistics().registrations == 0);
        assert!(Registration::connect(route, &a).is_ok());
    }
    #[test]
    fn tunnels_and_per_host_and_per_ip_connections_are_bounded() {
        let server = Running::start(Limits {
            tunnels: 1,
            ..Limits::default()
        });
        let a = NodeIdentity::generate().unwrap();
        let b = NodeIdentity::generate().unwrap();
        let _ra = Registration::connect(server.route(&a), &a).unwrap();
        let _rb = Registration::connect(server.route(&b), &b).unwrap();
        let tunnel = server.route(&a).open_tunnel().unwrap();
        assert_eq!(server.relay.statistics().tunnels, 1);
        assert!(server
            .route(&a)
            .open_tunnel()
            .err()
            .unwrap()
            .to_string()
            .contains("busy"));
        assert!(server
            .route(&b)
            .open_tunnel()
            .err()
            .unwrap()
            .to_string()
            .contains("resource limit"));
        drop(tunnel);
        wait(|| server.relay.statistics().tunnels == 0);
        let limited = Running::start(Limits {
            connections: 2,
            per_ip: 1,
            registrations: 1,
            tunnels: 1,
        });
        let _registered = Registration::connect(limited.route(&a), &a).unwrap();
        assert!(limited.route(&a).open_tunnel().is_err());
        assert_eq!(limited.relay.statistics().connections, 1);
    }
    #[test]
    fn malformed_control_messages_do_not_create_registrations_or_tunnels() {
        let server = Running::start(Limits::default());
        let a = NodeIdentity::generate().unwrap();
        let route = server.route(&a);
        for packet in [
            b"YRL\0\x02\x01\0\x20".as_slice(),
            b"YRL\0\x01\x05\xff\xff",
            b"YRL\0\x01\x00\0\0",
        ] {
            let mut connection = TlsTransport::new(route.relay)
                .unwrap()
                .connect_session(route.endpoint, ynp_tls::MAX_SESSION)
                .unwrap();
            connection.write_all(packet).unwrap();
            connection.flush().unwrap();
            assert!(matches!(
                wire::read(&mut *connection),
                Ok(Frame::Error(4)) | Err(_)
            ));
        }
        wait(|| server.relay.statistics().connections == 0);
        assert_eq!(server.relay.statistics().registrations, 0);
        assert_eq!(server.relay.statistics().tunnels, 0);
        assert_eq!(server.relay.statistics().forwarded_bytes, 0);
    }
    #[test]
    fn loopback_mode_refuses_implicit_public_exposure() {
        assert!(Relay::bind(
            "0.0.0.0:0".parse().unwrap(),
            Arc::new(NodeIdentity::generate().unwrap()),
            Limits::default()
        )
        .is_err());
        assert!(Relay::bind(
            "192.168.1.1:7443".parse().unwrap(),
            Arc::new(NodeIdentity::generate().unwrap()),
            Limits::default()
        )
        .is_err());
        assert!(Relay::bind(
            "127.0.0.1:0".parse().unwrap(),
            Arc::new(NodeIdentity::generate().unwrap()),
            Limits {
                connections: 0,
                ..Limits::default()
            }
        )
        .is_err());
    }
    #[test]
    fn opaque_forwarding_closes_at_the_byte_quota_without_unbounded_buffering() {
        let server = Running::start(Limits::default());
        let identity = NodeIdentity::generate().unwrap();
        let route = server.route(&identity);
        let mut host = Registration::connect(route, &identity).unwrap();
        let mut client = route.open_tunnel().unwrap();
        assert_eq!(
            wire::read(&mut *host.outer).unwrap(),
            Frame::Go(identity.id().public_key())
        );
        let sender = thread::spawn(move || {
            let block = [7; 8192];
            for _ in 0..=(MAX_TUNNEL_BYTES / block.len()) {
                if host
                    .outer
                    .write_all(&block)
                    .and_then(|_| host.outer.flush())
                    .is_err()
                {
                    break;
                }
            }
            // Keep the host half alive until the relay enforces its own quota.
            host.outer.read_timeout(TUNNEL_LIFETIME).unwrap();
            let _ = host.outer.read(&mut [0; 1]);
        });
        client.read_timeout(Duration::from_secs(5)).unwrap();
        let mut total = 0;
        let mut buffer = [0; 8192];
        loop {
            match client.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(length) => {
                    total += length;
                    assert!(total <= MAX_TUNNEL_BYTES);
                }
            }
        }
        sender.join().unwrap();
        assert!((256 * 1024..=MAX_TUNNEL_BYTES).contains(&total));
        wait(|| server.relay.statistics().tunnels == 0);
        assert_eq!(server.relay.statistics().forwarded_bytes, total as u64);
    }
}
