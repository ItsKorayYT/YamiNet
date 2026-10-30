//! Explicitly configured reverse relay. Outer TLS authenticates the relay; inner
//! TLS authenticates the destination. Public operation requires a separate opt-in.
mod server;
mod wire;
pub use server::{Limits, Relay, Statistics};
use std::{
    io,
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use yami_identity::{NodeId, NodeIdentity, RelayRegistration};
use ynp::Connection;
use ynp_tls::{ByteStream, SecureStream, TlsAcceptor, TlsTransport};

pub const REGISTRATION_LEASE: Duration = Duration::from_secs(30);
pub const CHALLENGE_TTL: u64 = 5;
pub const TUNNEL_LIFETIME: Duration = Duration::from_secs(15);
pub const CLIENT_LIFETIME: Duration = Duration::from_secs(20);
pub const MAX_TUNNEL_BYTES: usize = 2 * 1024 * 1024;
pub(crate) fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
pub(crate) fn now() -> io::Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|t| t.as_secs())
        .map_err(io::Error::other)
}
pub(crate) fn timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

#[derive(Clone, Copy, Debug)]
pub struct Route {
    pub relay: NodeId,
    pub endpoint: SocketAddr,
    pub destination: NodeId,
}
impl Route {
    fn session(&self, lifetime: Duration, public: bool) -> io::Result<Box<dyn SecureStream>> {
        if public {
            let SocketAddr::V4(endpoint) = self.endpoint else {
                return Err(invalid("public relay requires IPv4"));
            };
            if endpoint.port() == 0 || !ynp_tls::is_public_ipv4(*endpoint.ip()) {
                return Err(invalid("public relay endpoint must be public unicast IPv4"));
            }
        } else {
            ynp::require_loopback(self.endpoint)?;
        }
        TlsTransport::new(self.relay)?.connect_session(self.endpoint, lifetime)
    }
    /// Return only a byte tunnel. This is NOT authenticated as the destination.
    /// Use connect() or explicitly apply the destination's pinned TLS verifier.
    pub fn open_tunnel(&self) -> io::Result<Box<dyn ByteStream>> {
        self.open_tunnel_budget(false, CLIENT_LIFETIME)
    }
    fn open_tunnel_budget(
        &self,
        public: bool,
        budget: Duration,
    ) -> io::Result<Box<dyn ByteStream>> {
        if budget.is_zero() || budget > CLIENT_LIFETIME {
            return Err(invalid("invalid relay client budget"));
        }
        let deadline = Instant::now() + budget;
        let mut outer = self.session(budget, public)?;
        let until = deadline.min(Instant::now() + Duration::from_secs(CHALLENGE_TTL));
        wire::write(
            &mut Control {
                stream: &mut *outer,
                until,
            },
            &wire::Frame::Connect(self.destination.public_key()),
        )?;
        match wire::expect(wire::read(&mut Control {
            stream: &mut *outer,
            until,
        })?)? {
            wire::Frame::Paired(id) if id == self.destination.public_key() => {
                Ok(Box::new(Tunnel(outer)))
            }
            _ => Err(invalid("relay did not pair the requested destination")),
        }
    }
    pub fn connect(&self) -> io::Result<Box<dyn Connection>> {
        self.connect_with_timeout(false, CLIENT_LIFETIME)
    }
    /// Policy comes from explicit configuration, never from an advertisement.
    /// The total budget covers outer TLS, pairing, inner TLS and document I/O.
    pub fn connect_with_timeout(
        &self,
        public: bool,
        budget: Duration,
    ) -> io::Result<Box<dyn Connection>> {
        let started = Instant::now();
        let tunnel = self.open_tunnel_budget(public, budget)?;
        let remaining = ynp_tls::DEADLINE.min(budget.saturating_sub(started.elapsed()));
        TlsTransport::new(self.destination)?.connect_over(tunnel, remaining)
    }
    pub fn fetch(&self, key: &str) -> io::Result<ynp::DocumentData> {
        ynp::validate_key(key)?;
        let mut connection = self.connect()?;
        if connection.protection() != ynp::Protection::Tls13PinnedServer {
            return Err(invalid("destination TLS authentication required"));
        }
        ynp::request(&mut *connection, key)
    }
}
struct Tunnel(Box<dyn SecureStream>);
impl io::Read for Tunnel {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.0.read(bytes)
    }
}
impl io::Write for Tunnel {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}
impl ByteStream for Tunnel {
    fn read_timeout(&mut self, timeout: Duration) -> io::Result<()> {
        self.0.read_timeout(timeout)
    }
    fn write_timeout(&mut self, timeout: Duration) -> io::Result<()> {
        self.0.write_timeout(timeout)
    }
    fn close(&mut self) -> io::Result<()> {
        self.0.close()
    }
    fn limit_deadline(&mut self, deadline: Instant) -> io::Result<()> {
        self.0.limit_deadline(deadline)
    }
}
impl Drop for Tunnel {
    fn drop(&mut self) {
        let _ = self.0.close();
    }
}

/// One authenticated outbound registration, kept on the exact challenged TLS
/// session. There is no bearer registration token to steal or replay.
pub struct Registration {
    outer: Box<dyn SecureStream>,
    host: NodeId,
    expires: u64,
}
impl Registration {
    pub fn connect(route: Route, identity: &NodeIdentity) -> io::Result<Self> {
        Self::connect_mode(route, identity, false)
    }
    pub fn connect_public(route: Route, identity: &NodeIdentity) -> io::Result<Self> {
        Self::connect_mode(route, identity, true)
    }
    fn connect_mode(route: Route, identity: &NodeIdentity, public: bool) -> io::Result<Self> {
        if route.destination != *identity.id() {
            return Err(invalid("registration signer is not the destination"));
        }
        let mut outer = route.session(ynp_tls::MAX_SESSION, public)?;
        let until = Instant::now() + Duration::from_secs(CHALLENGE_TTL);
        wire::write(
            &mut Control {
                stream: &mut *outer,
                until,
            },
            &wire::Frame::Register(identity.id().public_key()),
        )?;
        let wire::Frame::Challenge(nonce, expires) = wire::expect(wire::read(&mut Control {
            stream: &mut *outer,
            until,
        })?)?
        else {
            return Err(invalid("expected fresh relay challenge"));
        };
        let at = now()?;
        if expires <= at || expires > at.saturating_add(CHALLENGE_TTL) {
            return Err(invalid("expired or invalid relay challenge"));
        }
        let challenge = RelayRegistration {
            relay: route.relay,
            host: *identity.id(),
            nonce,
            channel: outer.channel_binding()?,
            expires,
        };
        wire::write(
            &mut Control {
                stream: &mut *outer,
                until,
            },
            &wire::Frame::Proof(identity.sign(&challenge).to_bytes()),
        )?;
        let wire::Frame::Registered(expires) = wire::expect(wire::read(&mut Control {
            stream: &mut *outer,
            until,
        })?)?
        else {
            return Err(invalid("relay did not accept registration"));
        };
        let at = now()?;
        if expires <= at || expires > at.saturating_add(REGISTRATION_LEASE.as_secs()) {
            return Err(invalid("invalid registration lease"));
        }
        Ok(Self {
            outer,
            host: *identity.id(),
            expires,
        })
    }
    /// Wait responsively for a client, then authenticate as the host over inner
    /// TLS. A stopped or naturally expired idle lease yields None.
    pub fn accept(
        mut self,
        acceptor: &TlsAcceptor,
        stop: &AtomicBool,
    ) -> io::Result<Option<Box<dyn Connection>>> {
        loop {
            if stop.load(Ordering::SeqCst) || now()? >= self.expires {
                return Ok(None);
            }
            self.outer.read_timeout(Duration::from_millis(200))?;
            let mut first = [0];
            match self.outer.read(&mut first) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "relay disconnected",
                    ))
                }
                Ok(_) => {
                    self.outer.read_timeout(ynp::TIMEOUT)?;
                    let until = Instant::now() + Duration::from_secs(CHALLENGE_TTL);
                    let frame = wire::expect(wire::read_prefixed(
                        &mut Control {
                            stream: &mut *self.outer,
                            until,
                        },
                        first[0],
                    )?)?;
                    if frame != wire::Frame::Go(self.host.public_key()) {
                        return Err(invalid("unexpected relay tunnel destination"));
                    }
                    return acceptor
                        .accept_over(Box::new(Tunnel(self.outer)), ynp_tls::DEADLINE)
                        .map(Some);
                }
                Err(error) if timeout(&error) => continue,
                Err(error) => return Err(error),
            }
        }
    }
}
/// Control phases check their total deadline before every underlying I/O,
/// including partial frames. Socket timeout limits are propagated to TLS.
pub(crate) struct Control<'a> {
    pub stream: &'a mut dyn SecureStream,
    pub until: Instant,
}
impl Control<'_> {
    fn remaining(&self) -> io::Result<Duration> {
        let duration = self.until.saturating_duration_since(Instant::now());
        if duration.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "relay control deadline exceeded",
            ));
        }
        Ok(duration.min(ynp::TIMEOUT))
    }
}
impl io::Read for Control<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.stream.read_timeout(self.remaining()?)?;
        self.stream.read(buffer)
    }
}
impl io::Write for Control<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.stream.write_timeout(self.remaining()?)?;
        self.stream.write(buffer)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream.write_timeout(self.remaining()?)?;
        self.stream.flush()
    }
}

#[derive(Debug, Clone, Copy)]
pub enum HostEvent {
    Registered,
    Served,
    Retrying,
    Unregistered,
}
/// One outbound slot per host with bounded jittered reconnection, not discovery.
pub fn run_host(
    route: Route,
    identity: Arc<NodeIdentity>,
    stop: &AtomicBool,
    handler: &dyn Fn(&mut dyn Connection) -> io::Result<()>,
    report: &dyn Fn(HostEvent),
) -> io::Result<()> {
    run_host_mode(route, identity, stop, handler, report, false)
}
pub fn run_host_public(
    route: Route,
    identity: Arc<NodeIdentity>,
    stop: &AtomicBool,
    handler: &dyn Fn(&mut dyn Connection) -> io::Result<()>,
    report: &dyn Fn(HostEvent),
) -> io::Result<()> {
    run_host_mode(route, identity, stop, handler, report, true)
}
fn run_host_mode(
    route: Route,
    identity: Arc<NodeIdentity>,
    stop: &AtomicBool,
    handler: &dyn Fn(&mut dyn Connection) -> io::Result<()>,
    report: &dyn Fn(HostEvent),
    public: bool,
) -> io::Result<()> {
    let acceptor = TlsAcceptor::new(identity.clone())?;
    let mut failures = 0u32;
    while !stop.load(Ordering::SeqCst) {
        let registration = if public {
            Registration::connect_public(route, &identity)
        } else {
            Registration::connect(route, &identity)
        };
        let attempt = registration.and_then(|registration| {
            report(HostEvent::Registered);
            if let Some(mut connection) = registration.accept(&acceptor, stop)? {
                handler(&mut *connection)?;
                report(HostEvent::Served);
            }
            Ok(())
        });
        report(HostEvent::Unregistered);
        if attempt.is_err() {
            failures = failures.saturating_add(1);
            report(HostEvent::Retrying);
        } else {
            failures = 0;
        }
        let mut entropy = [0; 8];
        getrandom::fill(&mut entropy).map_err(io::Error::other)?;
        let base = 250u64.saturating_mul(1 << failures.min(4)).min(4000);
        let delay = base + u64::from_ne_bytes(entropy) % (base / 4 + 1);
        let until = Instant::now() + Duration::from_millis(delay);
        while !stop.load(Ordering::SeqCst) && Instant::now() < until {
            thread::sleep(Duration::from_millis(25));
        }
    }
    Ok(())
}

#[cfg(test)]
mod public_tests {
    use super::*;
    #[test]
    fn public_mode_is_explicit_and_rejects_wildcard_and_special_endpoints() {
        let identity = Arc::new(NodeIdentity::generate().unwrap());
        for endpoint in [
            "0.0.0.0:7443",
            "224.0.0.1:7443",
            "[::]:7443",
            "198.51.100.1:7443",
        ] {
            assert!(Relay::bind_public(
                endpoint.parse().unwrap(),
                identity.clone(),
                Limits::default()
            )
            .is_err());
        }
        assert!(Relay::bind(
            "192.168.1.1:7443".parse().unwrap(),
            identity.clone(),
            Limits::default()
        )
        .is_err());
        let relay = Relay::bind_public(
            "127.0.0.1:0".parse().unwrap(),
            identity.clone(),
            Limits::default(),
        )
        .unwrap();
        assert!(relay.local_addr().unwrap().port() > 0);
        let route = Route {
            relay: *identity.id(),
            destination: *NodeIdentity::generate().unwrap().id(),
            endpoint: relay.local_addr().unwrap(),
        };
        assert!(route
            .connect_with_timeout(true, Duration::from_secs(1))
            .is_err());
        assert!(route.connect_with_timeout(false, Duration::ZERO).is_err());
        assert!(route
            .connect_with_timeout(false, CLIENT_LIFETIME + Duration::from_secs(1))
            .is_err());
    }
}

/// Isolated, socket-free NAT research prerequisites. Not a NAT transport.
pub mod rendezvous;

#[cfg(test)]
mod slow_control_tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn authenticated_relay_cannot_extend_go_control_by_dripping_bytes() {
        let relay = Arc::new(NodeIdentity::generate().unwrap());
        let host = Arc::new(NodeIdentity::generate().unwrap());
        let relay_id = *relay.id();
        let host_id = *host.id();
        let listener = ynp_tls::TlsListener::bind("127.0.0.1:0".parse().unwrap(), relay).unwrap();
        let endpoint = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let mut outer = listener.accept_session(ynp_tls::MAX_SESSION).unwrap();
            outer.complete_handshake().unwrap();
            assert_eq!(
                wire::read(&mut *outer).unwrap(),
                wire::Frame::Register(host_id.public_key())
            );
            let nonce = [7; 32];
            let expires = now().unwrap() + CHALLENGE_TTL;
            wire::write(&mut *outer, &wire::Frame::Challenge(nonce, expires)).unwrap();
            let wire::Frame::Proof(proof) = wire::read(&mut *outer).unwrap() else {
                panic!("proof")
            };
            let challenge = RelayRegistration {
                relay: relay_id,
                host: host_id,
                nonce,
                channel: outer.channel_binding().unwrap(),
                expires,
            };
            host_id
                .verify(&challenge, &yami_identity::Signature::from_bytes(proof))
                .unwrap();
            wire::write(
                &mut *outer,
                &wire::Frame::Registered(now().unwrap() + REGISTRATION_LEASE.as_secs()),
            )
            .unwrap();
            let mut bytes = Vec::new();
            wire::write(&mut bytes, &wire::Frame::Go(host_id.public_key())).unwrap();
            for byte in bytes {
                if outer
                    .write_all(&[byte])
                    .and_then(|_| outer.flush())
                    .is_err()
                {
                    break;
                }
                thread::sleep(Duration::from_millis(200));
            }
        });
        let registration = Registration::connect(
            Route {
                relay: relay_id,
                destination: host_id,
                endpoint,
            },
            &host,
        )
        .unwrap();
        let acceptor = TlsAcceptor::new(host).unwrap();
        let started = Instant::now();
        assert!(registration
            .accept(&acceptor, &AtomicBool::new(false))
            .is_err());
        assert!(started.elapsed() < Duration::from_secs(6));
        server.join().unwrap();
    }
}
