//! Loopback-only reverse relay MVP. Outer TLS authenticates the relay; inner
//! TLS authenticates the destination. No DHT advertisements or browser policy.
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
    fn session(&self, lifetime: Duration) -> io::Result<Box<dyn SecureStream>> {
        // Deliberately no public deployment mode in this milestone.
        ynp::require_loopback(self.endpoint)?;
        TlsTransport::new(self.relay)?.connect_session(self.endpoint, lifetime)
    }
    /// Return only a byte tunnel. This is NOT authenticated as the destination.
    /// Use connect() or explicitly apply the destination's pinned TLS verifier.
    pub fn open_tunnel(&self) -> io::Result<Box<dyn ByteStream>> {
        let mut outer = self.session(CLIENT_LIFETIME)?;
        let until = Instant::now() + Duration::from_secs(CHALLENGE_TTL);
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
        let started = Instant::now();
        let tunnel = self.open_tunnel()?;
        let budget = ynp_tls::DEADLINE.min(CLIENT_LIFETIME.saturating_sub(started.elapsed()));
        TlsTransport::new(self.destination)?.connect_over(tunnel, budget)
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
        if route.destination != *identity.id() {
            return Err(invalid("registration signer is not the destination"));
        }
        let mut outer = route.session(ynp_tls::MAX_SESSION)?;
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
                    let frame = wire::expect(wire::read_prefixed(&mut *self.outer, first[0])?)?;
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
}
/// One outbound slot per host with bounded jittered reconnection, not discovery.
pub fn run_host(
    route: Route,
    identity: Arc<NodeIdentity>,
    stop: &AtomicBool,
    handler: &dyn Fn(&mut dyn Connection) -> io::Result<()>,
    report: &dyn Fn(HostEvent),
) -> io::Result<()> {
    let acceptor = TlsAcceptor::new(identity.clone())?;
    let mut failures = 0u32;
    while !stop.load(Ordering::SeqCst) {
        let attempt = Registration::connect(route, &identity).and_then(|registration| {
            report(HostEvent::Registered);
            if let Some(mut connection) = registration.accept(&acceptor, stop)? {
                handler(&mut *connection)?;
                report(HostEvent::Served);
            }
            Ok(())
        });
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
