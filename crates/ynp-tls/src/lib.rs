//! Experimental authenticated, encrypted YNP: unchanged YNP frames inside
//! TLS 1.3 (rustls with the ring provider), between loopback addresses or,
//! when a listener explicitly opts in, on private or public IPv4 ([`Scope`]).
//!
//! A node's TLS key is its persistent Ed25519 node identity key. The node
//! presents a short-lived X.509 certificate for that key, self-signed by it,
//! and signs every handshake with it. A client names, before connecting, the
//! one node ID it trusts, and accepts the server only if the certificate holds
//! exactly that key, is correctly self-signed and currently valid, and the
//! handshake signature verifies with it. There is no certificate authority,
//! name lookup, trust on first use or fallback to plaintext. Clients are not
//! authenticated. See `docs/specs/v0.1/ynp-tls.md`.
mod admission;
mod credential;
mod verify;

use rustls::{
    client::Resumption, crypto::CryptoProvider, pki_types::ServerName,
    server::NoServerSessionStorage, ClientConfig, ClientConnection, ConnectionCommon, ServerConfig,
    ServerConnection, SideData, StreamOwned,
};
use std::{
    io::{self, Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use yami_identity::{NodeId, NodeIdentity};
use ynp::{Connection, Incoming, Protection, Transport};

pub use credential::{Credential, BACKDATE};
pub use verify::IdentityMismatch;

pub const WARNING: &str = "EXPERIMENTAL: TLS 1.3 to an exact pinned node ID. Public listening requires --public. Not security-reviewed or production-safe; no anonymity.";
/// Negotiated by both sides, so a YNP peer never talks to another TLS service.
pub const ALPN: &[u8] = b"ynp/1";
pub const SCHEME: &str = "ynps://";
/// How long a node certificate is valid. Listeners renew halfway through.
pub const CREDENTIAL_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);
/// Total time for one connection including the handshake, on both sides, so
/// a peer trickling bytes cannot hold it open within the socket timeouts.
pub const DEADLINE: Duration = Duration::from_secs(10);
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
pub const TLS_BUFFER_LIMIT: usize = 64 * 1024;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
fn other(error: impl std::error::Error + Send + Sync + 'static) -> io::Error {
    io::Error::other(error)
}

/// Which peer addresses a listener or client may use. Plaintext YNP stays
/// loopback-only in the `ynp` crate; only this authenticated transport, and
/// only after a listener opts in, reaches beyond the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Numeric loopback addresses only, the default for every listener.
    Loopback,
    /// Loopback and private IPv4 addresses (RFC 1918: 10.0.0.0/8,
    /// 172.16.0.0/12, 192.168.0.0/16). Never public, link-local, multicast,
    /// broadcast or unspecified addresses, and no IPv6 beyond `::1`.
    Lan,
    /// Existing local modes plus ordinary public IPv4 unicast. No names,
    /// wildcard, link-local, shared/CGNAT, documentation or reserved addresses.
    Internet,
}
impl Scope {
    pub fn check(self, address: SocketAddr) -> io::Result<()> {
        match self {
            Self::Loopback => ynp::require_loopback(address),
            Self::Lan if address.ip().is_loopback() || is_private_lan(address.ip()) => Ok(()),
            Self::Internet if address.ip().is_loopback() || is_private_lan(address.ip())
                || matches!(address.ip(), IpAddr::V4(ip) if is_public_ipv4(ip)) => Ok(()),
            Self::Internet => Err(io::Error::new(io::ErrorKind::PermissionDenied,
                "TLS requires numeric loopback, private LAN, or public unicast IPv4; special-use addresses and remote IPv6 are refused")),
            Self::Lan => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "only loopback and private IPv4 LAN addresses (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16) are allowed",
            )),
        }
    }
}
/// A private IPv4 address (RFC 1918), the only kind a LAN listener binds.
pub fn is_private_lan(ip: IpAddr) -> bool {
    matches!(ip, IpAddr::V4(ip) if ip.is_private())
}
/// Conservative public-unicast policy; not a claim that an address is routable.
/// Special-use exclusions follow the IANA IPv4 special-purpose registry.
pub fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || a == 0
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0 && (c == 0 || c == 2))
        || (a == 192 && b == 88 && c == 99)
        || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
        || (a == 203 && b == 0 && c == 113))
}

/// TLS 1.3 only, every ring cipher suite and key exchange group of it.
fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Connects only to the node whose ID it was created with.
#[derive(Debug, Clone)]
pub struct TlsTransport {
    trusted: NodeId,
    config: Arc<ClientConfig>,
}
impl TlsTransport {
    pub fn new(trusted: NodeId) -> io::Result<Self> {
        let provider = provider();
        let mut config = ClientConfig::builder_with_provider(Arc::clone(&provider))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(other)?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verify::PinnedServer { trusted, provider }))
            .with_no_client_auth();
        config.alpn_protocols = vec![ALPN.to_vec()];
        // Every connection is a full handshake that checks the identity again,
        // and no ticket links one connection to the next.
        config.resumption = Resumption::disabled();
        // The node ID is the server name; it is checked but never sent.
        config.enable_sni = false;
        Ok(Self {
            trusted,
            config: Arc::new(config),
        })
    }
    pub fn trusted(&self) -> &NodeId {
        &self.trusted
    }
    /// Same verifier/transport, with a caller budget covering TCP connect,
    /// handshake AND subsequent I/O. Used by bounded iterative lookup RPCs.
    pub fn connect_with_timeout(
        &self,
        address: SocketAddr,
        timeout: Duration,
    ) -> io::Result<Box<dyn Connection>> {
        if timeout.is_zero() || timeout > DEADLINE {
            return Err(invalid("TLS caller budget must be >0 and <=10 seconds"));
        }
        self.connect_inner(address, Some(Instant::now() + timeout))
    }
    fn connect_inner(
        &self,
        address: SocketAddr,
        deadline: Option<Instant>,
    ) -> io::Result<Box<dyn Connection>> {
        Scope::Internet.check(address)?;
        if address.port() == 0 {
            return Err(invalid("TLS destination port must be nonzero"));
        }
        let name = ServerName::try_from(self.trusted.to_string()).map_err(other)?;
        let mut tls = ClientConnection::new(Arc::clone(&self.config), name).map_err(other)?;
        tls.set_buffer_limit(Some(TLS_BUFFER_LIMIT));
        let timeout = deadline.map_or(ynp::TIMEOUT, |until| {
            until
                .saturating_duration_since(Instant::now())
                .min(ynp::TIMEOUT)
        });
        if timeout.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "TLS caller deadline exceeded",
            ));
        }
        let stream = TcpStream::connect_timeout(&address, timeout)?;
        ynp::configure_stream(&stream)?;
        let mut connection = TlsConnection::new(tls, stream);
        if let Some(until) = deadline {
            connection.until = connection.until.min(until);
            connection.tls.sock.until = connection.tls.sock.until.min(until);
        }
        connection
            .handshake()
            .map_err(|error| match mismatch(&error) {
                Some(mismatch) => io::Error::new(io::ErrorKind::PermissionDenied, mismatch),
                None => error,
            })?;
        Ok(Box::new(connection))
    }
}
impl Transport for TlsTransport {
    /// Completes the handshake before returning, so no request is written
    /// until the server has proved the trusted identity. Reaches loopback and
    /// private LAN and public IPv4 addresses; the same pinned identity is checked.
    fn connect(&self, address: SocketAddr) -> io::Result<Box<dyn Connection>> {
        self.connect_inner(address, None)
    }
}
fn mismatch(error: &io::Error) -> Option<IdentityMismatch> {
    match error.get_ref()?.downcast_ref::<rustls::Error>()? {
        rustls::Error::InvalidCertificate(rustls::CertificateError::Other(other)) => {
            other.0.downcast_ref::<IdentityMismatch>().cloned()
        }
        _ => None,
    }
}

/// Accepts clients within its [`Scope`] and authenticates as one node identity.
pub struct TlsListener {
    listener: TcpListener,
    config: Arc<ServerConfig>,
    id: NodeId,
    scope: Scope,
    admission: Option<Arc<Mutex<admission::Admission>>>,
}
impl TlsListener {
    /// Serve as `identity` on a loopback address, issuing and renewing
    /// certificates as needed.
    pub fn bind(address: SocketAddr, identity: Arc<NodeIdentity>) -> io::Result<Self> {
        let id = *identity.id();
        let resolver = credential::Resolver::renewing(identity)?;
        Self::with_resolver(address, Scope::Loopback, resolver, id)
    }
    /// Serve as `identity` on one explicitly chosen private IPv4 LAN address:
    /// never a wildcard, loopback or public address. Clients from outside
    /// loopback and private IPv4 ranges are dropped before the handshake.
    pub fn bind_lan(address: SocketAddr, identity: Arc<NodeIdentity>) -> io::Result<Self> {
        if !is_private_lan(address.ip()) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "a LAN listener needs one specific private IPv4 address (10.x.x.x, 172.16-31.x.x or 192.168.x.x), never 0.0.0.0, loopback or a public address",
            ));
        }
        let id = *identity.id();
        let resolver = credential::Resolver::renewing(identity)?;
        Self::with_resolver(address, Scope::Lan, resolver, id)
    }
    /// Explicit public-service opt-in. Binds one IPv4 address, including a
    /// private address behind a manually configured forward. Never a wildcard.
    /// Loopback is accepted for isolated testing. No router/firewall changes.
    pub fn bind_public(address: SocketAddr, identity: Arc<NodeIdentity>) -> io::Result<Self> {
        if !address.is_ipv4() {
            return Err(invalid(
                "public listening requires one specific IPv4 address",
            ));
        }
        let id = *identity.id();
        let resolver = credential::Resolver::renewing(identity)?;
        Self::with_resolver(address, Scope::Internet, resolver, id)
    }
    /// Present exactly this credential on a loopback address until the
    /// listener is dropped.
    pub fn bind_with_credential(address: SocketAddr, credential: &Credential) -> io::Result<Self> {
        let id = *credential.node_id();
        let resolver = credential::Resolver::fixed(credential);
        Self::with_resolver(address, Scope::Loopback, resolver, id)
    }
    fn with_resolver(
        address: SocketAddr,
        scope: Scope,
        resolver: credential::Resolver,
        id: NodeId,
    ) -> io::Result<Self> {
        scope.check(address)?;
        let mut config = ServerConfig::builder_with_provider(provider())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(other)?
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(resolver));
        config.alpn_protocols = vec![ALPN.to_vec()];
        config.session_storage = Arc::new(NoServerSessionStorage {});
        config.send_tls13_tickets = 0;
        let listener = if scope == Scope::Internet {
            let socket = socket2::Socket::new(
                socket2::Domain::IPV4,
                socket2::Type::STREAM,
                Some(socket2::Protocol::TCP),
            )?;
            #[cfg(unix)]
            socket.set_reuse_address(true)?;
            socket.bind(&address.into())?;
            socket.listen(128)?;
            socket.into()
        } else {
            TcpListener::bind(address)?
        };
        Ok(Self {
            listener,
            config: Arc::new(config),
            id,
            scope,
            admission: (scope == Scope::Internet)
                .then(|| Arc::new(Mutex::new(admission::Admission::default()))),
        })
    }
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }
    /// Nonblocking accept only; accepted streams retain bounded blocking TLS I/O.
    pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        self.listener.set_nonblocking(nonblocking)
    }
    pub fn node_id(&self) -> &NodeId {
        &self.id
    }
    pub fn scope(&self) -> Scope {
        self.scope
    }
}
impl Incoming for TlsListener {
    /// Returns once TCP is accepted. The handshake runs on first use of the
    /// connection, inside its deadline, so a failed handshake affects only
    /// that connection.
    fn accept(&self) -> io::Result<Box<dyn Connection>> {
        let (stream, peer) = self.listener.accept()?;
        // Some platforms inherit a listener's nonblocking mode on accept.
        // TLS uses bounded blocking I/O even when accept is polled for shutdown.
        stream.set_nonblocking(false)?;
        self.scope.check(peer)?;
        let lease = self
            .admission
            .as_ref()
            .map(|limits| admission::Lease::acquire(Arc::clone(limits), peer.ip()))
            .transpose()?;
        ynp::configure_stream(&stream)?;
        let mut tls = ServerConnection::new(Arc::clone(&self.config)).map_err(other)?;
        tls.set_buffer_limit(Some(TLS_BUFFER_LIMIT));
        let mut connection = TlsConnection::new(tls, stream);
        connection._lease = lease;
        Ok(Box::new(connection))
    }
}

/// A TCP stream that fails every operation once its deadline has passed.
struct Socket {
    stream: TcpStream,
    until: Instant,
}
impl Socket {
    fn check(&self) -> io::Result<()> {
        if Instant::now() > self.until {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "TLS connection deadline exceeded",
            ));
        }
        Ok(())
    }
    fn prepare(&self, reading: bool) -> io::Result<()> {
        self.check()?;
        let remaining = self.until.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "TLS deadline exceeded",
            ));
        }
        let timeout = Some(remaining.min(ynp::TIMEOUT));
        if reading {
            self.stream.set_read_timeout(timeout)
        } else {
            self.stream.set_write_timeout(timeout)
        }
    }
}
impl Read for Socket {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.prepare(true)?;
        self.stream.read(buf)
    }
}
impl Write for Socket {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.prepare(false)?;
        self.stream.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

/// One TLS 1.3 connection carrying YNP frames. Neither side sends
/// close_notify: YNP frames are length-delimited, so a truncated reply is
/// already an error.
struct TlsConnection<C> {
    tls: StreamOwned<C, Socket>,
    ready: bool,
    until: Instant,
    _lease: Option<admission::Lease>,
}
impl<C, S> TlsConnection<C>
where
    C: DerefMut + Deref<Target = ConnectionCommon<S>>,
    S: SideData,
{
    fn new(tls: C, stream: TcpStream) -> Self {
        let socket = Socket {
            stream,
            until: Instant::now() + HANDSHAKE_TIMEOUT,
        };
        Self {
            tls: StreamOwned::new(tls, socket),
            ready: false,
            until: Instant::now() + DEADLINE,
            _lease: None,
        }
    }
    fn handshake(&mut self) -> io::Result<()> {
        if self.ready {
            return Ok(());
        }
        let StreamOwned { conn, sock } = &mut self.tls;
        while conn.is_handshaking() {
            conn.complete_io(sock)?;
        }
        if conn.alpn_protocol() != Some(ALPN) {
            return Err(invalid("peer did not negotiate ynp/1"));
        }
        sock.check()?;
        self.ready = true;
        sock.until = self.until;
        Ok(())
    }
}
impl<C, S> Read for TlsConnection<C>
where
    C: DerefMut + Deref<Target = ConnectionCommon<S>>,
    S: SideData,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.handshake()?;
        self.tls.sock.check()?;
        self.tls.read(buf)
    }
}
impl<C, S> Write for TlsConnection<C>
where
    C: DerefMut + Deref<Target = ConnectionCommon<S>>,
    S: SideData,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.handshake()?;
        self.tls.sock.check()?;
        self.tls.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.handshake()?;
        self.tls.sock.check()?;
        self.tls.flush()
    }
}
impl<C, S> Connection for TlsConnection<C>
where
    C: DerefMut + Deref<Target = ConnectionCommon<S>> + Send,
    S: SideData,
{
    fn protection(&self) -> Protection {
        Protection::Tls13PinnedServer
    }
}

/// `ynps://<node-id>@<numeric-IP>:<port>/<key>`. The node ID is the identity
/// the client requires. The address follows [`Scope::Internet`] with a nonzero
/// port, never a name; the key
/// follows the `ynp://` rules.
pub fn parse_url(url: &str) -> io::Result<(NodeId, SocketAddr, String)> {
    let rest = url
        .strip_prefix(SCHEME)
        .ok_or_else(|| invalid("expected ynps://node-id@numeric-IP:port/key"))?;
    let (id, rest) = rest
        .split_once('@')
        .ok_or_else(|| invalid("missing trusted node ID: ynps://node-id@address/key"))?;
    let id: NodeId = id
        .parse()
        .map_err(|error| invalid(format!("trusted node ID: {error}")))?;
    let (address, key) = ynp::parse_endpoint(rest)?;
    Scope::Internet.check(address)?;
    Ok((id, address, key))
}
pub fn format_url(id: &NodeId, address: SocketAddr, key: &str) -> String {
    format!("{SCHEME}{id}@{address}/{key}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internet_scope_is_numeric_unicast_and_does_not_change_local_modes() {
        for text in [
            "8.8.8.8:7443",
            "1.1.1.1:7443",
            "127.0.0.1:7443",
            "192.168.1.2:7443",
            "[::1]:7443",
        ] {
            assert!(
                Scope::Internet.check(text.parse().unwrap()).is_ok(),
                "{text}"
            );
        }
        for text in [
            "0.0.0.0:7443",
            "0.1.2.3:7443",
            "100.64.1.1:7443",
            "169.254.1.1:7443",
            "192.0.2.1:7443",
            "198.51.100.1:7443",
            "203.0.113.1:7443",
            "198.18.1.1:7443",
            "224.0.0.1:7443",
            "240.0.0.1:7443",
            "255.255.255.255:7443",
            "[2001:4860::1]:7443",
            "[::ffff:8.8.8.8]:7443",
        ] {
            assert!(
                Scope::Internet.check(text.parse().unwrap()).is_err(),
                "{text}"
            );
        }
        assert!(Scope::Lan.check("8.8.8.8:7443".parse().unwrap()).is_err());
        assert!(Scope::Loopback
            .check("10.0.0.1:7443".parse().unwrap())
            .is_err());
        let id = *NodeIdentity::generate().unwrap().id();
        assert!(parse_url(&format!("ynps://{id}@public-host:7443/home")).is_err());
        assert!(TlsTransport::new(id)
            .unwrap()
            .connect("8.8.8.8:0".parse().unwrap())
            .is_err());
        for address in ["0.0.0.0:0", "[::]:0", "[::1]:0", "192.0.2.1:0"] {
            assert!(TlsListener::bind_public(
                address.parse().unwrap(),
                Arc::new(NodeIdentity::generate().unwrap())
            )
            .is_err());
        }
    }

    #[test]
    fn public_admission_releases_capacity_when_connections_drop() {
        let listener = TlsListener::bind_public(
            "127.0.0.1:0".parse().unwrap(),
            Arc::new(NodeIdentity::generate().unwrap()),
        )
        .unwrap();
        let address = listener.local_addr().unwrap();
        let _first = TcpStream::connect(address).unwrap();
        let first = listener.accept().unwrap();
        let _second = TcpStream::connect(address).unwrap();
        let second = listener.accept().unwrap();
        let _third = TcpStream::connect(address).unwrap();
        assert_eq!(
            listener.accept().err().unwrap().kind(),
            io::ErrorKind::WouldBlock
        );
        drop(first);
        let _fourth = TcpStream::connect(address).unwrap();
        let fourth = listener.accept().unwrap();
        drop((second, fourth));
    }

    #[test]
    fn socket_reads_cannot_exceed_remaining_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let started = Instant::now();
        let mut socket = Socket {
            stream,
            until: started + Duration::from_millis(40),
        };
        assert!(socket.read(&mut [0; 1]).is_err());
        assert!(started.elapsed() < Duration::from_millis(500));
        socket.until = Instant::now() - Duration::from_millis(1);
        assert_eq!(
            socket.write(b"data").unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn lan_scope_admits_only_loopback_and_private_ipv4() {
        for allowed in [
            "127.0.0.1:7443",
            "127.8.9.10:7443",
            "[::1]:7443",
            "10.0.0.1:7443",
            "10.255.255.254:7443",
            "172.16.0.1:7443",
            "172.31.255.254:7443",
            "192.168.0.1:7443",
            "192.168.1.50:7443",
        ] {
            let address: SocketAddr = allowed.parse().unwrap();
            assert!(Scope::Lan.check(address).is_ok(), "{allowed}");
        }
        for refused in [
            "0.0.0.0:7443",
            "255.255.255.255:7443",
            "8.8.8.8:7443",
            "192.0.2.1:7443",
            "172.15.255.255:7443",
            "172.32.0.1:7443",
            "192.169.0.1:7443",
            "169.254.1.1:7443",
            "100.64.0.1:7443",
            "224.0.0.1:7443",
            "[::]:7443",
            "[fe80::1]:7443",
            "[fd00::1]:7443",
            "[2001:db8::1]:7443",
            "[::ffff:192.168.1.50]:7443",
        ] {
            let address: SocketAddr = refused.parse().unwrap();
            let error = Scope::Lan.check(address).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{refused}");
        }
        // The default stays loopback, so LAN reach is always an explicit choice.
        assert!(Scope::Loopback
            .check("192.168.1.50:7443".parse().unwrap())
            .is_err());
        assert!(Scope::Loopback
            .check("127.0.0.1:7443".parse().unwrap())
            .is_ok());
    }

    #[test]
    fn lan_listeners_need_one_specific_private_address() {
        let identity = Arc::new(NodeIdentity::generate().unwrap());
        for refused in [
            "0.0.0.0:0",
            "127.0.0.1:0",
            "[::]:0",
            "[::1]:0",
            "8.8.8.8:0",
            "169.254.1.1:0",
        ] {
            let address: SocketAddr = refused.parse().unwrap();
            let error = TlsListener::bind_lan(address, Arc::clone(&identity))
                .err()
                .unwrap_or_else(|| panic!("bound {refused}"));
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{refused}");
        }
        // Without opting in, a listener refuses a private address too.
        let lan: SocketAddr = "192.168.1.50:0".parse().unwrap();
        assert!(TlsListener::bind(lan, identity).is_err());
    }

    #[test]
    fn ynps_urls_reach_local_and_public_ipv4_with_pinned_id() {
        let id = NodeIdentity::generate().unwrap().id().to_string();
        for (url, address) in [
            (
                format!("ynps://{id}@192.168.1.50:7443/home"),
                "192.168.1.50:7443",
            ),
            (format!("ynps://{id}@10.1.2.3:1/a-b_c"), "10.1.2.3:1"),
            (format!("ynps://{id}@127.0.0.1:7443/home"), "127.0.0.1:7443"),
            (format!("ynps://{id}@8.8.8.8:7443/home"), "8.8.8.8:7443"),
        ] {
            let (parsed, endpoint, _) = parse_url(&url).unwrap();
            assert_eq!(parsed.to_string(), id);
            assert_eq!(endpoint, address.parse().unwrap());
        }
        for bad in [
            format!("ynps://{id}@0.0.0.0:7443/home"),
            format!("ynps://{id}@198.51.100.1:7443/home"),
            format!("ynps://{id}@169.254.1.1:7443/home"),
            format!("ynps://{id}@[fd00::1]:7443/home"),
            format!("ynps://{id}@192.168.1.50:0/home"),
            format!("ynps://{id}@192.168.1.50/home"),
            format!("ynps://{id}@192.168.1.50:7443"),
            format!("ynps://{id}@192.168.1.50:7443/"),
            format!("ynps://{id}@192.168.1.50:7443/a/b"),
            format!("ynps://{id}@lan-host:7443/home"),
            format!("ynps://{id}@192.168.001.050:7443/home"),
            format!("ynps://{id}@ 192.168.1.50:7443/home"),
            "ynps://192.168.1.50:7443/home".to_owned(),
            "ynp://192.168.1.50:7443/home".to_owned(),
        ] {
            assert!(parse_url(&bad).is_err(), "{bad}");
        }
    }
}
