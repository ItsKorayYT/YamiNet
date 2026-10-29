//! Opt-in, link-local IPv4 discovery. Advertisements are claims, never trust.
//! See docs/specs/v0.1/lan-discovery.md for the wire format and limits.
use socket2::Domain;
use socket_pktinfo::PktInfoUdpSocket;
use std::{
    collections::VecDeque,
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use yami_identity::NodeId;

pub const DEFAULT_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 89, 78);
pub const DEFAULT_PORT: u16 = 39817;
pub const SEARCH_TIME: Duration = Duration::from_secs(3);
pub const MAX_SEARCH_TIME: Duration = Duration::from_secs(5);
pub const MAX_PACKET: usize = 192;
pub const MAX_RESULTS: usize = 64;
const MAX_DATAGRAMS: usize = 1024;
const IO_WAIT: Duration = Duration::from_millis(10);
const QUERY: &[u8; 5] = b"YND\x01Q";
const RESPONSE: &[u8; 5] = b"YND\x01R";
type RequestId = [u8; 16];

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub group: Ipv4Addr,
    pub port: u16,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            group: DEFAULT_GROUP,
            port: DEFAULT_PORT,
        }
    }
}
impl Config {
    pub fn validate(self) -> io::Result<Self> {
        if self.group.octets()[..2] != [239, 255] || self.port == 0 {
            return Err(invalid(
                "discovery requires a 239.255.x.x local multicast group and nonzero UDP port",
            ));
        }
        Ok(self)
    }
    /// Shared opt-in configuration for the browser and node. Invalid values fail closed.
    pub fn from_env() -> io::Result<Self> {
        let mut config = Self::default();
        if let Some(value) = std::env::var_os("YAMINET_DISCOVERY_GROUP") {
            config.group = value
                .to_str()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| invalid("invalid YAMINET_DISCOVERY_GROUP"))?;
        }
        if let Some(value) = std::env::var_os("YAMINET_DISCOVERY_PORT") {
            config.port = value
                .to_str()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| invalid("invalid YAMINET_DISCOVERY_PORT"))?;
        }
        config.validate()
    }
    fn destination(self) -> SocketAddr {
        SocketAddrV4::new(self.group, self.port).into()
    }
}

/// A locally enumerated private interface. Construction cannot select a route
/// implicitly; callers must choose one address from `interfaces()`.
#[derive(Clone, Debug)]
pub struct LanInterface {
    pub name: String,
    address: Ipv4Addr,
    netmask: Ipv4Addr,
    index: u32,
}
impl LanInterface {
    pub fn address(&self) -> Ipv4Addr {
        self.address
    }
    pub fn index(&self) -> u32 {
        self.index
    }
    fn contains(&self, ip: Ipv4Addr) -> bool {
        let mask = u32::from(self.netmask);
        let host = u32::from(ip) & !mask;
        ip.is_private()
            && u32::from(ip) & mask == u32::from(self.address) & mask
            && host != 0
            && host != !mask
    }
    fn accepts(&self, source: SocketAddr, index: u64) -> bool {
        matches!(source, SocketAddr::V4(addr) if addr.port() != 0 && self.contains(*addr.ip()))
            && index == u64::from(self.index)
    }
    fn current(&self) -> io::Result<()> {
        if interfaces()?.iter().any(|i| {
            i.address == self.address && i.index == self.index && i.netmask == self.netmask
        }) {
            Ok(())
        } else {
            Err(invalid("selected LAN interface is no longer available"))
        }
    }
}
/// Enumerates locally, sends no traffic, and never selects an interface.
pub fn interfaces() -> io::Result<Vec<LanInterface>> {
    let mut result = Vec::new();
    for iface in if_addrs::get_if_addrs()? {
        if let (if_addrs::IfAddr::V4(addr), Some(index)) = (iface.addr, iface.index) {
            let mask = u32::from(addr.netmask);
            // Require a contiguous subnet mask with room for ordinary hosts.
            if addr.ip.is_private()
                && index != 0
                && mask.leading_ones() >= 8
                && mask.leading_ones() <= 30
                && (!mask).wrapping_add(1).is_power_of_two()
            {
                let interface = LanInterface {
                    name: iface.name,
                    address: addr.ip,
                    netmask: addr.netmask,
                    index,
                };
                if interface.contains(addr.ip) {
                    result.push(interface);
                }
            }
        }
    }
    result.sort_by_key(|i| (i.index, i.address));
    result.dedup_by_key(|i| (i.index, i.address));
    Ok(result)
}
pub fn interface(address: Ipv4Addr) -> io::Result<LanInterface> {
    interfaces()?
        .into_iter()
        .find(|i| i.address == address)
        .ok_or_else(|| invalid("select a private IPv4 address of a local LAN interface"))
}

/// An unverified claim. Only a TLS handshake to `id` authenticates this key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Advertisement {
    id: NodeId,
    endpoint: SocketAddrV4,
    homepage: String,
}
impl Advertisement {
    pub fn new(id: NodeId, endpoint: SocketAddrV4, homepage: &str) -> io::Result<Self> {
        if !endpoint.ip().is_private() || endpoint.port() == 0 || homepage.len() > 64 {
            return Err(invalid(
                "discovery advertises only private IPv4 TLS endpoints and bounded homepages",
            ));
        }
        // Reuse the document-key policy and encrypted URL formatter. No
        // alternate scheme can be advertised by this binary protocol.
        ynp::validate_key(homepage)?;
        Ok(Self {
            id,
            endpoint,
            homepage: homepage.into(),
        })
    }
    pub fn id(&self) -> NodeId {
        self.id
    }
    pub fn endpoint(&self) -> SocketAddrV4 {
        self.endpoint
    }
    pub fn url(&self) -> String {
        ynp_tls::format_url(&self.id, self.endpoint.into(), &self.homepage)
    }
    fn encode(&self, request: RequestId) -> Vec<u8> {
        let mut packet = Vec::with_capacity(60 + self.homepage.len());
        packet.extend_from_slice(RESPONSE);
        packet.extend_from_slice(&request);
        packet.extend_from_slice(&self.id.public_key());
        packet.extend_from_slice(&self.endpoint.ip().octets());
        packet.extend_from_slice(&self.endpoint.port().to_be_bytes());
        packet.push(self.homepage.len() as u8);
        packet.extend_from_slice(self.homepage.as_bytes());
        packet
    }
}
fn query(id: RequestId) -> [u8; MAX_PACKET] {
    let mut packet = [0; MAX_PACKET];
    packet[..5].copy_from_slice(QUERY);
    packet[5..21].copy_from_slice(&id);
    packet
}
fn parse_query(packet: &[u8]) -> Option<RequestId> {
    if packet.len() != MAX_PACKET || &packet[..5] != QUERY || packet[21..].iter().any(|b| *b != 0) {
        return None;
    }
    packet[5..21].try_into().ok()
}
fn parse_response(packet: &[u8], request: RequestId, source: SocketAddr) -> Option<Advertisement> {
    if !(61..=124).contains(&packet.len())
        || &packet[..5] != RESPONSE
        || packet[5..21] != request
        || usize::from(packet[59]) + 60 != packet.len()
    {
        return None;
    }
    let id = NodeId::from_public_key(packet[21..53].try_into().ok()?).ok()?;
    let ip = Ipv4Addr::new(packet[53], packet[54], packet[55], packet[56]);
    if source.ip() != IpAddr::V4(ip) {
        return None;
    }
    let port = u16::from_be_bytes(packet[57..59].try_into().ok()?);
    Advertisement::new(
        id,
        SocketAddrV4::new(ip, port),
        std::str::from_utf8(&packet[60..]).ok()?,
    )
    .ok()
}
fn collect_response(
    results: &mut Vec<Advertisement>,
    packet: &[u8],
    request: RequestId,
    source: SocketAddr,
) {
    if results.len() >= MAX_RESULTS {
        return;
    }
    if let Some(advert) = parse_response(packet, request, source) {
        if !results
            .iter()
            .any(|node| node.id == advert.id && node.endpoint == advert.endpoint)
        {
            results.push(advert);
        }
    }
}

fn socket(interface: &LanInterface) -> io::Result<PktInfoUdpSocket> {
    interface.current()?;
    let socket = PktInfoUdpSocket::new(Domain::IPV4)?;
    socket.set_multicast_if_v4(&interface.address)?;
    socket.set_multicast_ttl_v4(1)?;
    socket.set_multicast_loop_v4(true)?;
    // WSARecvMsg does not reliably observe SO_RCVTIMEO on Windows. Poll a
    // nonblocking socket so search deadlines and responder shutdown are real.
    socket.set_nonblocking(true)?;
    Ok(socket)
}
fn ignorable(error: &io::Error) -> bool {
    matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted)
        // Windows: truncated UDP datagram / ICMP port unreachable.
        || matches!(error.raw_os_error(), Some(10040 | 10054))
}

/// One query, one selected interface, at most five seconds, 64 results and
/// 1024 datagrams. The ephemeral socket closes on return; late replies vanish.
pub fn discover(
    interface: &LanInterface,
    config: Config,
    timeout: Duration,
) -> io::Result<Vec<Advertisement>> {
    let config = config.validate()?;
    if timeout.is_zero() || timeout > MAX_SEARCH_TIME {
        return Err(invalid("discovery timeout must be >0 and <=5 seconds"));
    }
    let socket = socket(interface)?;
    socket.bind(&SocketAddr::from((interface.address, 0)).into())?;
    let mut request = [0; 16];
    getrandom::fill(&mut request).map_err(io::Error::other)?;
    socket.send_to(&query(request), &config.destination().into())?;
    let deadline = Instant::now() + timeout;
    let mut results = Vec::new();
    let mut packet = [0; MAX_PACKET + 1];
    let mut received = 0;
    while Instant::now() < deadline && received < MAX_DATAGRAMS && results.len() < MAX_RESULTS {
        match socket.recv(&mut packet) {
            Ok((len, info)) => {
                received += 1;
                if Instant::now() >= deadline
                    || len > MAX_PACKET
                    || !interface.accepts(info.addr_src, info.if_index)
                    || info.addr_dst != IpAddr::V4(interface.address)
                {
                    continue;
                }
                collect_response(&mut results, &packet[..len], request, info.addr_src);
            }
            Err(error) if ignorable(&error) => {
                if error.kind() != io::ErrorKind::WouldBlock {
                    received += 1;
                }
                thread::sleep(IO_WAIT.min(deadline.saturating_duration_since(Instant::now())));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(results)
}

struct ResponseLimit {
    recent: VecDeque<(Instant, SocketAddr, RequestId)>,
}
impl ResponseLimit {
    fn allow(&mut self, now: Instant, source: SocketAddr, request: RequestId) -> bool {
        while self
            .recent
            .front()
            .is_some_and(|(time, _, _)| now.duration_since(*time) >= Duration::from_secs(5))
        {
            self.recent.pop_front();
        }
        if self
            .recent
            .iter()
            .any(|(_, addr, id)| *addr == source && *id == request)
            || self
                .recent
                .iter()
                .filter(|(time, _, _)| now.duration_since(*time) < Duration::from_secs(1))
                .count()
                >= 10
            || self.recent.iter().any(|(time, addr, _)| {
                addr.ip() == source.ip() && now.duration_since(*time) < Duration::from_secs(1)
            })
        {
            return false;
        }
        // The global rate cap guarantees <=50 entries over the five-second window.
        self.recent.push_back((now, source, request));
        true
    }
}

/// Lifetime guard for an explicitly started responder. No announcements.
pub struct Responder {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Responder {
    pub fn start(
        interface: LanInterface,
        config: Config,
        advert: Advertisement,
    ) -> io::Result<Self> {
        let config = config.validate()?;
        if *advert.endpoint.ip() != interface.address {
            return Err(invalid("advertisement must name the selected interface"));
        }
        let socket = socket(&interface)?;
        socket.set_reuse_address(true)?;
        // Multicast reception needs a wildcard UDP bind on Windows. Packet-info
        // enforces BOTH receiving interface index and multicast destination.
        socket.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, config.port)).into())?;
        socket.join_multicast_v4(&config.group, &interface.address)?;
        let replies = UdpSocket::bind((interface.address, 0))?;
        replies.set_ttl(1)?;
        replies.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("ynp-discovery".into())
            .spawn(move || {
                let mut limit = ResponseLimit {
                    recent: VecDeque::new(),
                };
                let mut packet = [0; MAX_PACKET + 1];
                let mut processed = 0usize;
                while !stopping.load(Ordering::Relaxed) {
                    match socket.recv(&mut packet) {
                        Ok((len, info)) => {
                            processed += 1;
                            // Bound work even under a continuous stream of garbage.
                            if processed.is_multiple_of(64) {
                                thread::sleep(Duration::from_millis(1));
                            }
                            if len > MAX_PACKET
                                || info.addr_dst != IpAddr::V4(config.group)
                                || !interface.accepts(info.addr_src, info.if_index)
                            {
                                continue;
                            }
                            if let Some(request) = parse_query(&packet[..len]) {
                                if limit.allow(Instant::now(), info.addr_src, request) {
                                    let _ = replies.send_to(&advert.encode(request), info.addr_src);
                                }
                            }
                        }
                        Err(error) if ignorable(&error) => thread::sleep(IO_WAIT),
                        Err(error) => {
                            eprintln!("LAN discovery responder stopped: {error}");
                            break;
                        }
                    }
                }
            })?;
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }
}
impl Drop for Responder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yami_identity::NodeIdentity;

    fn lan() -> LanInterface {
        LanInterface {
            name: "test".into(),
            address: "192.168.1.10".parse().unwrap(),
            netmask: "255.255.255.0".parse().unwrap(),
            index: 12,
        }
    }
    fn advert() -> Advertisement {
        Advertisement::new(
            *NodeIdentity::generate().unwrap().id(),
            "192.168.1.20:7443".parse().unwrap(),
            "home",
        )
        .unwrap()
    }
    #[test]
    fn wire_roundtrip_matches_id_and_encrypted_endpoint() {
        let node = advert();
        let request = [17; 16];
        let source = "192.168.1.20:12345".parse().unwrap();
        let packet = node.encode(request);
        assert_eq!(parse_query(&query(request)), Some(request));
        assert!(
            packet.len() < query(request).len(),
            "no response amplification"
        );
        assert_eq!(parse_response(&packet, request, source), Some(node.clone()));
        let (id, endpoint, key) = ynp_tls::parse_url(&node.url()).unwrap();
        assert_eq!(id, node.id());
        assert_eq!(endpoint, SocketAddr::V4(node.endpoint()));
        assert_eq!(key, "home");
        assert!(node.url().starts_with("ynps://"));
    }
    #[test]
    fn rejects_malformed_unsolicited_and_redirected_packets() {
        let node = advert();
        let request = [17; 16];
        let source = "192.168.1.20:12345".parse().unwrap();
        let packet = node.encode(request);
        assert!(parse_response(&packet, [18; 16], source).is_none());
        assert!(parse_response(&packet, request, "192.168.1.21:1".parse().unwrap()).is_none());
        for len in 0..packet.len() {
            assert!(parse_response(&packet[..len], request, source).is_none());
        }
        for (offset, value) in [(0, 0), (3, 2), (4, b'Q'), (59, 255), (60, b'/'), (61, 255)] {
            let mut bad = packet.clone();
            bad[offset] = value;
            assert!(
                parse_response(&bad, request, source).is_none(),
                "offset {offset}"
            );
        }
        let mut bad = packet.clone();
        bad[21..53].fill(0);
        assert!(parse_response(&bad, request, source).is_none());
        let mut bad = packet.clone();
        bad[57..59].fill(0);
        assert!(parse_response(&bad, request, source).is_none());
        let mut bad = packet.clone();
        bad.push(0);
        assert!(parse_response(&bad, request, source).is_none());
        assert!(parse_response(&[0; MAX_PACKET + 1], request, source).is_none());
        assert!(parse_response(b"ynp://192.168.1.20:7443/home", request, source).is_none());
        for ip in [
            "127.0.0.1",
            "0.0.0.0",
            "8.8.8.8",
            "239.255.89.78",
            "169.254.1.1",
        ] {
            assert!(
                Advertisement::new(node.id(), format!("{ip}:7443").parse().unwrap(), "home")
                    .is_err()
            );
        }
        for key in ["", "a/b", "../home", "é", &"a".repeat(65)] {
            assert!(Advertisement::new(node.id(), node.endpoint(), key).is_err());
        }
        for len in 0..MAX_PACKET {
            assert!(parse_query(&query(request)[..len]).is_none());
        }
        let mut bad = query(request);
        bad[191] = 1;
        assert!(parse_query(&bad).is_none());
        assert!(parse_query(&packet).is_none());
    }
    #[test]
    fn duplicate_suppression_and_results_are_bounded() {
        let node = advert();
        let request = [7; 16];
        let source = SocketAddr::from((*node.endpoint().ip(), 1234));
        let mut results = Vec::new();
        collect_response(&mut results, &node.encode([8; 16]), request, source);
        assert!(results.is_empty());
        for _ in 0..10 {
            collect_response(&mut results, &node.encode(request), request, source);
        }
        let other_home = Advertisement::new(node.id(), node.endpoint(), "other").unwrap();
        collect_response(&mut results, &other_home.encode(request), request, source);
        assert_eq!(results.as_slice(), std::slice::from_ref(&node));
        for port in 1000..1100 {
            let different = Advertisement::new(
                node.id(),
                SocketAddrV4::new(*node.endpoint().ip(), port),
                "home",
            )
            .unwrap();
            collect_response(&mut results, &different.encode(request), request, source);
        }
        assert_eq!(results.len(), MAX_RESULTS);
    }
    #[test]
    fn only_selected_interface_and_private_subnet_are_accepted() {
        let interface = lan();
        assert!(interface.accepts("192.168.1.20:99".parse().unwrap(), 12));
        for (addr, index) in [
            ("192.168.1.20:99", 13),
            ("192.168.2.20:99", 12),
            ("8.8.8.8:99", 12),
            ("192.168.1.255:99", 12),
            ("192.168.1.0:99", 12),
            ("192.168.1.20:0", 12),
        ] {
            assert!(!interface.accepts(addr.parse().unwrap(), index));
        }
        for group in ["224.0.0.1", "239.254.1.2", "8.8.8.8"] {
            assert!(Config {
                group: group.parse().unwrap(),
                port: DEFAULT_PORT
            }
            .validate()
            .is_err());
        }
        assert!(Config {
            port: 0,
            ..Config::default()
        }
        .validate()
        .is_err());
        assert!(discover(&interface, Config::default(), Duration::ZERO).is_err());
        assert!(discover(&interface, Config::default(), Duration::from_secs(6)).is_err());
    }
    #[test]
    fn response_rate_and_replay_memory_are_bounded() {
        let now = Instant::now();
        let mut limit = ResponseLimit {
            recent: VecDeque::new(),
        };
        let source = "192.168.1.20:99".parse().unwrap();
        assert!(limit.allow(now, source, [1; 16]));
        assert!(!limit.allow(now, source, [1; 16]));
        assert!(!limit.allow(now, "192.168.1.20:100".parse().unwrap(), [2; 16]));
        assert!(!limit.allow(now + Duration::from_secs(2), source, [1; 16]));
        for host in 21..30 {
            assert!(limit.allow(
                now,
                format!("192.168.1.{host}:99").parse().unwrap(),
                [1; 16]
            ));
        }
        assert!(!limit.allow(now, "192.168.1.30:99".parse().unwrap(), [1; 16]));
        assert_eq!(limit.recent.len(), 10);
        assert!(limit.allow(now + Duration::from_secs(5), source, [1; 16]));
        assert_eq!(limit.recent.len(), 1);
    }
    #[test]
    fn arbitrary_packets_never_panic() {
        let mut seed = 7u64;
        for len in 0..=MAX_PACKET + 32 {
            let mut packet = vec![0; len];
            for byte in &mut packet {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                *byte = (seed >> 32) as u8;
            }
            let _ = parse_query(&packet);
            let _ = parse_response(&packet, [1; 16], "192.168.1.20:99".parse().unwrap());
        }
    }
}
