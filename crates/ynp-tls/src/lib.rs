//! Experimental authenticated, encrypted YNP: unchanged YNP frames inside
//! TLS 1.3 (rustls with the ring provider), between loopback addresses only.
//!
//! A node's TLS key is its persistent Ed25519 node identity key. The node
//! presents a short-lived X.509 certificate for that key, self-signed by it,
//! and signs every handshake with it. A client names, before connecting, the
//! one node ID it trusts, and accepts the server only if the certificate holds
//! exactly that key, is correctly self-signed and currently valid, and the
//! handshake signature verifies with it. There is no certificate authority,
//! name lookup, trust on first use or fallback to plaintext. Clients are not
//! authenticated. See `docs/specs/v0.1/ynp-tls.md`.
mod credential;
mod verify;

use rustls::{
    client::Resumption, crypto::CryptoProvider, pki_types::ServerName,
    server::NoServerSessionStorage, ClientConfig, ClientConnection, ConnectionCommon, ServerConfig,
    ServerConnection, SideData, StreamOwned,
};
use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    ops::{Deref, DerefMut},
    sync::Arc,
    time::{Duration, Instant},
};
use yami_identity::{NodeId, NodeIdentity};
use ynp::{Connection, Incoming, Protection, Transport};

pub use credential::{Credential, BACKDATE};
pub use verify::IdentityMismatch;

pub const WARNING: &str = "EXPERIMENTAL: TLS 1.3 to a pinned node ID, localhost only. Not independently reviewed; no anonymity.";
/// Negotiated by both sides, so a YNP peer never talks to another TLS service.
pub const ALPN: &[u8] = b"ynp/1";
pub const SCHEME: &str = "ynps://";
/// How long a node certificate is valid. Listeners renew halfway through.
pub const CREDENTIAL_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);
/// Total time for one connection including the handshake, on both sides, so
/// a peer trickling bytes cannot hold it open within the socket timeouts.
pub const DEADLINE: Duration = Duration::from_secs(10);

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
fn other(error: impl std::error::Error + Send + Sync + 'static) -> io::Error {
    io::Error::other(error)
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
}
impl Transport for TlsTransport {
    /// Completes the handshake before returning, so no request is written
    /// until the server has proved the trusted identity.
    fn connect(&self, address: SocketAddr) -> io::Result<Box<dyn Connection>> {
        ynp::require_loopback(address)?;
        let name = ServerName::try_from(self.trusted.to_string()).map_err(other)?;
        let tls = ClientConnection::new(Arc::clone(&self.config), name).map_err(other)?;
        let stream = TcpStream::connect_timeout(&address, ynp::TIMEOUT)?;
        ynp::configure_stream(&stream)?;
        let mut connection = TlsConnection::new(tls, stream);
        connection.handshake().map_err(|error| {
            // Report a wrong identity as such, rather than as a TLS failure.
            match mismatch(&error) {
                Some(mismatch) => io::Error::new(io::ErrorKind::PermissionDenied, mismatch),
                None => error,
            }
        })?;
        Ok(Box::new(connection))
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

/// Accepts loopback clients and authenticates as one node identity.
pub struct TlsListener {
    listener: TcpListener,
    config: Arc<ServerConfig>,
    id: NodeId,
}
impl TlsListener {
    /// Serve as `identity`, issuing and renewing certificates as needed.
    pub fn bind(address: SocketAddr, identity: Arc<NodeIdentity>) -> io::Result<Self> {
        let id = *identity.id();
        Self::with_resolver(address, credential::Resolver::renewing(identity)?, id)
    }
    /// Present exactly this credential until the listener is dropped.
    pub fn bind_with_credential(address: SocketAddr, credential: &Credential) -> io::Result<Self> {
        let id = *credential.node_id();
        Self::with_resolver(address, credential::Resolver::fixed(credential), id)
    }
    fn with_resolver(
        address: SocketAddr,
        resolver: credential::Resolver,
        id: NodeId,
    ) -> io::Result<Self> {
        ynp::require_loopback(address)?;
        let mut config = ServerConfig::builder_with_provider(provider())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(other)?
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(resolver));
        config.alpn_protocols = vec![ALPN.to_vec()];
        config.session_storage = Arc::new(NoServerSessionStorage {});
        config.send_tls13_tickets = 0;
        Ok(Self {
            listener: TcpListener::bind(address)?,
            config: Arc::new(config),
            id,
        })
    }
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }
    pub fn node_id(&self) -> &NodeId {
        &self.id
    }
}
impl Incoming for TlsListener {
    /// Returns once TCP is accepted. The handshake runs on first use of the
    /// connection, inside its deadline, so a failed handshake affects only
    /// that connection.
    fn accept(&self) -> io::Result<Box<dyn Connection>> {
        let (stream, peer) = self.listener.accept()?;
        ynp::require_loopback(peer)?;
        ynp::configure_stream(&stream)?;
        let tls = ServerConnection::new(Arc::clone(&self.config)).map_err(other)?;
        Ok(Box::new(TlsConnection::new(tls, stream)))
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
}
impl Read for Socket {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.check()?;
        self.stream.read(buf)
    }
}
impl Write for Socket {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.check()?;
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
}
impl<C, S> TlsConnection<C>
where
    C: DerefMut + Deref<Target = ConnectionCommon<S>>,
    S: SideData,
{
    fn new(tls: C, stream: TcpStream) -> Self {
        let socket = Socket {
            stream,
            until: Instant::now() + DEADLINE,
        };
        Self {
            tls: StreamOwned::new(tls, socket),
            ready: false,
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
        self.ready = true;
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
        self.tls.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.handshake()?;
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

/// `ynps://<node-id>@<numeric-loopback-IP>:<port>/<key>`. The node ID is the
/// identity the client requires; address and key follow the `ynp://` rules.
pub fn parse_url(url: &str) -> io::Result<(NodeId, SocketAddr, String)> {
    let rest = url
        .strip_prefix(SCHEME)
        .ok_or_else(|| invalid("expected ynps://node-id@numeric-loopback:port/key"))?;
    let (id, rest) = rest
        .split_once('@')
        .ok_or_else(|| invalid("missing trusted node ID: ynps://node-id@address/key"))?;
    let id: NodeId = id
        .parse()
        .map_err(|error| invalid(format!("trusted node ID: {error}")))?;
    let (address, key) = ynp::parse_url(&format!("ynp://{rest}"))?;
    Ok((id, address, key))
}
pub fn format_url(id: &NodeId, address: SocketAddr, key: &str) -> String {
    format!("{SCHEME}{id}@{address}/{key}")
}
