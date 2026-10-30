//! Small, opt-in signed peer lookup over the existing pinned TLS transport.
//! Not a production DHT, naming authority, anonymity system or liveness proof.
mod client;
mod record;
mod routing;
mod state;
mod storage;
#[cfg(test)]
mod tests;
mod wire;

pub use client::{
    exchange, lookup, lookup_newer, lookup_newer_with_connector, lookup_with_connector, publish,
    publish_with_connector, Lookup, PinnedTls, RpcConnector,
};
pub use record::{PeerRecord, MAX_RECORD_BYTES, MAX_RECORD_TTL};
pub use routing::{distance, Contact, RoutingTable, BUCKET_SIZE, MAX_ROUTES};
pub use state::{State, MAX_CACHE};
use std::{
    io,
    net::SocketAddrV4,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
pub use storage::Service;
pub use wire::MAGIC;

pub const MAX_FRAME: usize = 2048;
pub const MAX_REPLY_RECORDS: usize = 8;
pub const MAX_QUERIES: usize = 16;
pub const MAX_CANDIDATES: usize = 64;
pub const RPC_TIMEOUT: Duration = Duration::from_secs(3);
pub const LOOKUP_TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
pub fn now() -> io::Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(io::Error::other)
}

/// DHT record time only. TLS validity and socket deadlines always use real time.
pub trait Clock: Send + Sync {
    fn now(&self) -> io::Result<u64>;
}
pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> io::Result<u64> {
        now()
    }
}

/// Public DHTs cannot cause private/LAN/loopback probes. The isolated simulator
/// must explicitly select Loopback; records cannot cross these two policies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Network {
    Public,
    Loopback,
}
impl Network {
    pub fn check(self, endpoint: SocketAddrV4) -> io::Result<()> {
        let allowed = match self {
            Self::Public => ynp_tls::is_public_ipv4(*endpoint.ip()),
            Self::Loopback => endpoint.ip().is_loopback(),
        };
        if endpoint.port() == 0 || !allowed {
            return Err(invalid(
                "DHT endpoint is outside the explicitly selected public/loopback IPv4 policy",
            ));
        }
        Ok(())
    }
    pub(crate) fn byte(self) -> u8 {
        match self {
            Self::Public => 1,
            Self::Loopback => 2,
        }
    }
}

pub use yami_identity::RelayRoute;
