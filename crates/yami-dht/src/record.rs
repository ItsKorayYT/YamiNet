use crate::{invalid, Network};
use std::{
    io,
    net::{Ipv4Addr, SocketAddrV4},
};
use yami_identity::{NodeId, NodeIdentity, PeerRecordMetadata, Signable, Signature};

pub const MAX_RECORD_BYTES: usize = 137;
pub const MAX_RECORD_TTL: u64 = 24 * 60 * 60;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerRecord {
    metadata: PeerRecordMetadata,
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
        };
        record.verify(network, now)?;
        Ok(record)
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
        self.id()
            .verify(&self.metadata, &self.signature)
            .map_err(io::Error::other)
    }
    pub(crate) fn live(&self, now: u64) -> bool {
        self.expires() > now && self.expires().saturating_sub(now) <= MAX_RECORD_TTL
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = self.metadata.canonical_bytes();
        bytes.extend_from_slice(&self.signature.to_bytes());
        bytes
    }
    /// Structural decoding only. Callers must verify signature/policy/freshness
    /// before accepting a live record. State/Service enforce this themselves.
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        if !(119..=MAX_RECORD_BYTES).contains(&bytes.len()) {
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
            signature,
        })
    }
}
