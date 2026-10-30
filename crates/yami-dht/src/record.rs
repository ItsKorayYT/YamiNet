use crate::{invalid, Network};
use std::{
    io,
    net::{Ipv4Addr, SocketAddrV4},
};
use yami_identity::{
    NodeId, NodeIdentity, PeerRecordMetadata, RelayRoute, RoutedPeerRecordMetadata, Signable,
    Signature,
};

pub const MAX_RECORD_BYTES: usize = 218;
pub const MAX_RECORD_TTL: u64 = 24 * 60 * 60;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerRecord {
    metadata: PeerRecordMetadata,
    routed: Option<RoutedPeerRecordMetadata>,
    signature: Signature,
}
impl PeerRecord {
    pub fn sign(
        identity: &NodeIdentity,
        mut endpoints: Vec<SocketAddrV4>,
        expires: u64,
        sequence: u64,
        network: Network,
        now: u64,
    ) -> io::Result<Self> {
        endpoints.sort();
        let metadata = PeerRecordMetadata::new(*identity.id(), endpoints, expires, sequence)
            .map_err(io::Error::other)?;
        let record = Self {
            signature: identity.sign(&metadata),
            metadata,
            routed: None,
        };
        record.verify(network, now)?;
        Ok(record)
    }
    #[allow(clippy::too_many_arguments)] // Matches the v1 signing boundary plus bounded routes.
    pub fn sign_routes(
        identity: &NodeIdentity,
        mut endpoints: Vec<SocketAddrV4>,
        mut routes: Vec<RelayRoute>,
        expires: u64,
        sequence: u64,
        network: Network,
        now: u64,
    ) -> io::Result<Self> {
        endpoints.sort();
        routes.sort();
        let routed =
            RoutedPeerRecordMetadata::new(*identity.id(), endpoints, routes, expires, sequence)
                .map_err(io::Error::other)?;
        let record = Self {
            metadata: routed.direct().clone(),
            signature: identity.sign(&routed),
            routed: Some(routed),
        };
        record.verify(network, now)?;
        Ok(record)
    }
    pub fn relay_routes(&self) -> &[RelayRoute] {
        self.routed.as_ref().map_or(&[], |r| r.routes())
    }
    pub fn id(&self) -> NodeId {
        self.metadata.id()
    }
    pub fn endpoints(&self) -> &[SocketAddrV4] {
        self.metadata.endpoints()
    }
    pub fn sequence(&self) -> u64 {
        self.metadata.sequence()
    }
    pub fn expires(&self) -> u64 {
        self.metadata.expires()
    }
    pub fn verify(&self, network: Network, now: u64) -> io::Result<()> {
        self.verify_signature(network)?;
        if !self.live(now) {
            return Err(invalid(
                "expired peer record or expiration more than 24 hours ahead",
            ));
        }
        Ok(())
    }
    pub(crate) fn verify_signature(&self, network: Network) -> io::Result<()> {
        for endpoint in self.endpoints() {
            network.check(*endpoint)?;
        }
        for route in self.relay_routes() {
            network.check(route.endpoint)?;
        }
        if let Some(routed) = &self.routed {
            self.id().verify(routed, &self.signature)
        } else {
            self.id().verify(&self.metadata, &self.signature)
        }
        .map_err(io::Error::other)
    }
    pub(crate) fn live(&self, now: u64) -> bool {
        self.expires() > now && self.expires().saturating_sub(now) <= MAX_RECORD_TTL
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = self.routed.as_ref().map_or_else(
            || self.metadata.canonical_bytes(),
            Signable::canonical_bytes,
        );
        bytes.extend_from_slice(&self.signature.to_bytes());
        bytes
    }
    /// Structural decoding only. Callers must verify signature/policy/freshness
    /// before accepting a live record. State/Service enforce this themselves.
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        // v1 lengths are odd (113 + 6*n); v2 lengths are even (118 + 6*n + 38*r).
        // This prevents a legacy public key prefix from being mistaken for a version marker.
        if bytes.len().is_multiple_of(2) {
            return Self::decode_v2(bytes);
        }
        if !(119..=137).contains(&bytes.len()) {
            return Err(invalid("peer record size exceeds limits"));
        }
        let count = bytes[48] as usize;
        if !(1..=4).contains(&count) || bytes.len() != 49 + 6 * count + 64 {
            return Err(invalid("invalid endpoint count or trailing record bytes"));
        }
        let id = NodeId::from_public_key(bytes[..32].try_into().map_err(|_| invalid("node ID"))?)
            .map_err(io::Error::other)?;
        let expires = u64::from_be_bytes(bytes[32..40].try_into().map_err(|_| invalid("expiry"))?);
        let sequence =
            u64::from_be_bytes(bytes[40..48].try_into().map_err(|_| invalid("sequence"))?);
        let endpoints = bytes[49..49 + 6 * count]
            .chunks_exact(6)
            .map(|p| {
                SocketAddrV4::new(
                    Ipv4Addr::new(p[0], p[1], p[2], p[3]),
                    u16::from_be_bytes([p[4], p[5]]),
                )
            })
            .collect();
        let metadata =
            PeerRecordMetadata::new(id, endpoints, expires, sequence).map_err(io::Error::other)?;
        let signature = Signature::from_bytes(
            bytes[49 + 6 * count..]
                .try_into()
                .map_err(|_| invalid("signature"))?,
        );
        Ok(Self {
            metadata,
            routed: None,
            signature,
        })
    }
    fn decode_v2(bytes: &[u8]) -> io::Result<Self> {
        if !(118..=MAX_RECORD_BYTES).contains(&bytes.len()) || &bytes[..4] != b"YPR\x02" {
            return Err(invalid("invalid v2 peer record"));
        }
        let id =
            NodeId::from_public_key(bytes[4..36].try_into().unwrap()).map_err(io::Error::other)?;
        let expires = u64::from_be_bytes(bytes[36..44].try_into().unwrap());
        let sequence = u64::from_be_bytes(bytes[44..52].try_into().unwrap());
        let count = bytes[52] as usize;
        if count > 4 {
            return Err(invalid("too many direct endpoints"));
        }
        let offset = 53 + count * 6;
        let routes = *bytes.get(offset).ok_or_else(|| invalid("missing routes"))? as usize;
        if routes > 2 || bytes.len() != offset + 1 + routes * 38 + 64 {
            return Err(invalid("invalid relay count or trailing data"));
        }
        fn endpoint(p: &[u8]) -> SocketAddrV4 {
            SocketAddrV4::new(
                Ipv4Addr::new(p[0], p[1], p[2], p[3]),
                u16::from_be_bytes([p[4], p[5]]),
            )
        }
        let endpoints = bytes[53..offset].chunks_exact(6).map(endpoint).collect();
        let routes = bytes[offset + 1..bytes.len() - 64]
            .chunks_exact(38)
            .map(|p| {
                Ok(RelayRoute {
                    relay: NodeId::from_public_key(p[..32].try_into().unwrap())
                        .map_err(io::Error::other)?,
                    endpoint: endpoint(&p[32..]),
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        let routed = RoutedPeerRecordMetadata::new(id, endpoints, routes, expires, sequence)
            .map_err(io::Error::other)?;
        Ok(Self {
            metadata: routed.direct().clone(),
            routed: Some(routed),
            signature: Signature::from_bytes(bytes[bytes.len() - 64..].try_into().unwrap()),
        })
    }
}
