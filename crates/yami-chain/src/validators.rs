//! Self-signed discovery hints, never stake, eligibility or committee authority.
//! Epochs and restore commitments must come from independently authenticated state.
use crate::{
    codec::{Reader, Writer},
    hash, verify, Digest, Error, Result, Signer as _, SigningKey, VerifyingKey,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::{Ipv4Addr, SocketAddrV4},
    sync::OnceLock,
};

pub const MAX_CANDIDATES: usize = 128;
pub const MAX_DIRECTORY_BYTES: usize = 65_536;
pub const MAX_PAGE: usize = 16;
const DOMAIN: &[u8] = b"YC-CANDIDATE-1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddressPolicy {
    /// Numeric global-unicast IPv4 only. Never dial DNS or private endpoints.
    Public,
    /// Explicit local harness policy. No public listener is supplied here.
    LoopbackSimulation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Announcement {
    pub scope: Digest,
    pub epoch: u64,
    pub sequence: u64,
    pub consensus: [u8; 32],
    pub transport: [u8; 32],
    pub endpoint: SocketAddrV4,
    pub registered: bool,
    pub consensus_signature: [u8; 64],
    pub transport_signature: [u8; 64],
}
impl Announcement {
    fn preimage(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.raw(DOMAIN);
        w.raw(&self.scope.0);
        w.u64(self.epoch);
        w.u64(self.sequence);
        w.raw(&self.consensus);
        w.raw(&self.transport);
        w.raw(&self.endpoint.ip().octets());
        w.u16(self.endpoint.port());
        w.u8(u8::from(self.registered));
        w.0
    }
    pub fn sign(
        scope: Digest,
        epoch: u64,
        sequence: u64,
        endpoint: SocketAddrV4,
        registered: bool,
        consensus: &SigningKey,
        transport: &SigningKey,
    ) -> Self {
        let mut record = Self {
            scope,
            epoch,
            sequence,
            endpoint,
            registered,
            consensus: consensus.verifying_key().to_bytes(),
            transport: transport.verifying_key().to_bytes(),
            consensus_signature: [0; 64],
            transport_signature: [0; 64],
        };
        let bytes = record.preimage();
        record.consensus_signature = consensus.sign(&bytes).to_bytes();
        record.transport_signature = transport.sign(&bytes).to_bytes();
        record
    }
    pub fn canonical(&self) -> Vec<u8> {
        let mut bytes = self.preimage();
        bytes.extend_from_slice(&self.consensus_signature);
        bytes.extend_from_slice(&self.transport_signature);
        bytes
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes, 320)?;
        if r.take(DOMAIN.len())? != DOMAIN {
            return Err(Error::Encoding);
        }
        let scope = Digest(r.array()?);
        let epoch = r.u64()?;
        let sequence = r.u64()?;
        let consensus = r.array()?;
        let transport = r.array()?;
        let endpoint = SocketAddrV4::new(Ipv4Addr::from(r.array::<4>()?), r.u16()?);
        let registered = match r.u8()? {
            0 => false,
            1 => true,
            _ => return Err(Error::Encoding),
        };
        let consensus_signature = r.array()?;
        let transport_signature = r.array()?;
        r.end()?;
        Ok(Self {
            scope,
            epoch,
            sequence,
            consensus,
            transport,
            endpoint,
            registered,
            consensus_signature,
            transport_signature,
        })
    }
    pub fn verify(&self, scope: Digest, epoch: u64, policy: AddressPolicy) -> Result<()> {
        if self.scope != scope
            || self.epoch != epoch
            || epoch == 0
            || self.sequence == 0
            || self.endpoint.port() < 1024
            || self.consensus == self.transport
        {
            return Err(Error::Genesis);
        }
        for key in [self.consensus, self.transport] {
            if VerifyingKey::from_bytes(&key)
                .map_err(|_| Error::Signature)?
                .is_weak()
            {
                return Err(Error::Signature);
            }
            if policy == AddressPolicy::Public && known_development_keys().contains(&key) {
                return Err(Error::Unsupported);
            }
        }
        let allowed = match policy {
            AddressPolicy::Public => public_ipv4(*self.endpoint.ip()),
            AddressPolicy::LoopbackSimulation => self.endpoint.ip().is_loopback(),
        };
        if !allowed {
            return Err(Error::Unsupported);
        }
        let bytes = self.preimage();
        verify(&self.consensus, &bytes, &self.consensus_signature)?;
        verify(&self.transport, &bytes, &self.transport_signature)
    }
}

fn known_development_keys() -> &'static BTreeSet<[u8; 32]> {
    static KEYS: OnceLock<BTreeSet<[u8; 32]>> = OnceLock::new();
    KEYS.get_or_init(|| {
        (0..100)
            .flat_map(|i| {
                ["consensus", "transport"]
                    .map(|role| crate::development_key(role, i).verifying_key().to_bytes())
            })
            .collect()
    })
}
fn public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || a == 10
        || a == 127
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && (b == 168 || (b == 0 && (c == 0 || c == 2)) || (b == 88 && c == 99)))
        || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
        || (a == 203 && b == 0 && c == 113))
}

/// Bounded epoch-local hints. Tombstones retain sequence authority until the
/// authenticated epoch changes. A discovery cache is never an election roster.
pub struct CandidateDirectory {
    scope: Digest,
    epoch: u64,
    policy: AddressPolicy,
    records: BTreeMap<[u8; 32], Announcement>,
}
impl CandidateDirectory {
    pub fn new(scope: Digest, epoch: u64, policy: AddressPolicy) -> Result<Self> {
        if epoch == 0 {
            return Err(Error::Genesis);
        }
        Ok(Self {
            scope,
            epoch,
            policy,
            records: BTreeMap::new(),
        })
    }
    /// Exact retransmission is idempotent; conflicting same-sequence records
    /// fail. Verify before mutation, including both proofs of key possession.
    pub fn apply(&mut self, record: Announcement) -> Result<()> {
        record.verify(self.scope, self.epoch, self.policy)?;
        if let Some(old) = self.records.get(&record.consensus) {
            if old == &record {
                return Ok(());
            }
            if record.sequence <= old.sequence {
                return Err(Error::Nonce);
            }
        } else if self.records.len() >= MAX_CANDIDATES {
            return Err(Error::Limit);
        }
        for old in self.records.values() {
            if old.consensus != record.consensus
                && (old.transport == record.transport
                    || old.transport == record.consensus
                    || old.consensus == record.transport)
            {
                return Err(Error::Duplicate);
            }
        }
        let quota = if self.policy == AddressPolicy::Public {
            4
        } else {
            MAX_CANDIDATES
        };
        if record.registered
            && self
                .records
                .values()
                .filter(|old| {
                    old.consensus != record.consensus
                        && old.registered
                        && old.endpoint.ip() == record.endpoint.ip()
                })
                .count()
                >= quota
        {
            return Err(Error::Limit);
        }
        self.records.insert(record.consensus, record);
        Ok(())
    }
    pub fn page(&self, after: Option<[u8; 32]>, limit: usize) -> Result<Vec<Announcement>> {
        if limit == 0 || limit > MAX_PAGE {
            return Err(Error::Limit);
        }
        Ok(self
            .records
            .iter()
            .filter(|(key, record)| after.is_none_or(|a| **key > a) && record.registered)
            .take(limit)
            .map(|(_, record)| record.clone())
            .collect())
    }
    /// Snapshot includes deregistration tombstones. Authenticate the commitment
    /// independently; a peer's self-supplied digest cannot establish freshness.
    pub fn snapshot(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.raw(b"YC-DISCOVERY-SNAPSHOT-1");
        w.raw(&self.scope.0);
        w.u64(self.epoch);
        w.u32(self.records.len() as u32);
        for record in self.records.values() {
            w.bytes(&record.canonical())
                .expect("fixed bounded announcement");
        }
        w.0
    }
    pub fn commitment(&self) -> Digest {
        hash(b"YC-DISCOVERY-CHECKPOINT-1", &self.snapshot())
    }
    pub fn restore(
        bytes: &[u8],
        trusted: Digest,
        scope: Digest,
        epoch: u64,
        policy: AddressPolicy,
    ) -> Result<Self> {
        if bytes.len() > MAX_DIRECTORY_BYTES || hash(b"YC-DISCOVERY-CHECKPOINT-1", bytes) != trusted
        {
            return Err(Error::Encoding);
        }
        let mut r = Reader::new(bytes, MAX_DIRECTORY_BYTES)?;
        if r.take(b"YC-DISCOVERY-SNAPSHOT-1".len())? != b"YC-DISCOVERY-SNAPSHOT-1"
            || r.array::<32>()? != scope.0
            || r.u64()? != epoch
        {
            return Err(Error::Genesis);
        }
        let count = r.u32()? as usize;
        if count > MAX_CANDIDATES {
            return Err(Error::Limit);
        }
        let mut directory = Self::new(scope, epoch, policy)?;
        let mut previous = None;
        for _ in 0..count {
            let record = Announcement::decode(r.bytes(320)?)?;
            if previous.is_some_and(|p| p >= record.consensus) {
                return Err(Error::Encoding);
            }
            previous = Some(record.consensus);
            directory.apply(record)?;
        }
        r.end()?;
        Ok(directory)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(i: u8, sequence: u64, registered: bool) -> Announcement {
        Announcement::sign(
            Digest([1; 32]),
            1,
            sequence,
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, 28100 + u16::from(i)),
            registered,
            &crate::development_key("consensus", i),
            &crate::development_key("transport", i),
        )
    }
    #[test]
    fn signed_registration_tombstones_restart_and_epoch_replay() {
        let mut d =
            CandidateDirectory::new(Digest([1; 32]), 1, AddressPolicy::LoopbackSimulation).unwrap();
        let a = record(0, 1, true);
        d.apply(a.clone()).unwrap();
        d.apply(a.clone()).unwrap();
        assert!(d.apply(record(0, 1, false)).is_err());
        let mut forged = a.clone();
        forged.endpoint.set_port(30000);
        assert!(d.apply(forged).is_err());
        d.apply(record(0, 2, false)).unwrap();
        assert!(d.apply(a.clone()).is_err());
        assert!(d.page(None, 16).unwrap().is_empty());
        let bytes = d.snapshot();
        let mut restored = CandidateDirectory::restore(
            &bytes,
            d.commitment(),
            Digest([1; 32]),
            1,
            AddressPolicy::LoopbackSimulation,
        )
        .unwrap();
        assert!(restored.apply(a.clone()).is_err());
        restored.apply(record(0, 3, true)).unwrap();
        assert_eq!(restored.page(None, 1).unwrap().len(), 1);
        assert!(CandidateDirectory::restore(
            &bytes,
            d.commitment(),
            Digest([1; 32]),
            2,
            AddressPolicy::LoopbackSimulation
        )
        .is_err());
        let mut bad = bytes;
        bad.push(0);
        assert!(CandidateDirectory::restore(
            &bad,
            d.commitment(),
            Digest([1; 32]),
            1,
            AddressPolicy::LoopbackSimulation
        )
        .is_err());
        for n in 0..a.canonical().len() {
            assert!(Announcement::decode(&a.canonical()[..n]).is_err());
        }
    }
    #[test]
    fn public_policy_rejects_private_endpoints_and_development_keys() {
        assert!(record(0, 1, true)
            .verify(Digest([1; 32]), 1, AddressPolicy::Public)
            .is_err());
        let c = SigningKey::from_bytes(&[71; 32]);
        let t = SigningKey::from_bytes(&[72; 32]);
        for ip in [
            "0.0.0.0",
            "10.0.0.1",
            "127.0.0.1",
            "169.254.1.1",
            "100.64.0.1",
            "172.16.1.1",
            "192.168.1.1",
            "192.0.2.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "240.0.0.1",
        ] {
            let a = Announcement::sign(
                Digest([1; 32]),
                1,
                1,
                SocketAddrV4::new(ip.parse().unwrap(), 28100),
                true,
                &c,
                &t,
            );
            assert!(a.verify(Digest([1; 32]), 1, AddressPolicy::Public).is_err());
        }
        let a = Announcement::sign(
            Digest([1; 32]),
            1,
            1,
            SocketAddrV4::new("8.8.8.8".parse().unwrap(), 28100),
            true,
            &c,
            &t,
        );
        a.verify(Digest([1; 32]), 1, AddressPolicy::Public).unwrap(); // No dialing.
        let mut dev = record(0, 1, true);
        dev.endpoint = a.endpoint;
        assert!(dev
            .verify(Digest([1; 32]), 1, AddressPolicy::Public)
            .is_err());
    }
    #[test]
    fn pages_key_aliases_and_floods_are_bounded() {
        let mut d =
            CandidateDirectory::new(Digest([1; 32]), 1, AddressPolicy::LoopbackSimulation).unwrap();
        for i in 0..100 {
            d.apply(record(i, 1, true)).unwrap();
        }
        let mut cursor = None;
        let mut seen = BTreeSet::new();
        loop {
            let page = d.page(cursor, 16).unwrap();
            if page.is_empty() {
                break;
            }
            for a in &page {
                assert!(seen.insert(a.consensus));
            }
            cursor = page.last().map(|a| a.consensus);
        }
        assert_eq!(seen.len(), 100);
        assert!(d.page(None, 17).is_err());
        let aliased = Announcement::sign(
            Digest([1; 32]),
            1,
            1,
            record(0, 1, true).endpoint,
            true,
            &SigningKey::from_bytes(&[87; 32]),
            &crate::development_key("transport", 0),
        );
        assert!(d.apply(aliased).is_err());
        let snapshot = d.snapshot();
        assert!(snapshot.len() < MAX_DIRECTORY_BYTES);
        assert_eq!(
            CandidateDirectory::restore(
                &snapshot,
                d.commitment(),
                Digest([1; 32]),
                1,
                AddressPolicy::LoopbackSimulation
            )
            .unwrap()
            .snapshot(),
            snapshot
        );
        for i in 100..128 {
            d.apply(record(i, 1, true)).unwrap();
        }
        assert!(d.apply(record(128, 1, true)).is_err());
        assert!(d.snapshot().len() < MAX_DIRECTORY_BYTES);
    }
    #[test]
    fn public_address_quota_is_atomic_and_signatures_do_not_prove_stake() {
        let mut d = CandidateDirectory::new(Digest([1; 32]), 1, AddressPolicy::Public).unwrap();
        let make = |i: u8, sequence, registered| {
            Announcement::sign(
                Digest([1; 32]),
                1,
                sequence,
                "8.8.8.8:28100".parse().unwrap(),
                registered,
                &SigningKey::from_bytes(&hash(b"test-consensus", &[i]).0),
                &SigningKey::from_bytes(&hash(b"test-transport", &[i]).0),
            )
        };
        for i in 0..4 {
            d.apply(make(i, 1, true)).unwrap();
        }
        let before = d.snapshot();
        assert!(d.apply(make(4, 1, true)).is_err());
        assert_eq!(before, d.snapshot());
        d.apply(make(0, 2, false)).unwrap();
        d.apply(make(4, 1, true)).unwrap();
        assert!(d.apply(make(0, 1, true)).is_err());
        assert_eq!(d.page(None, 16).unwrap().len(), 4);
        let mut no_pop = make(5, 1, true);
        no_pop.transport_signature = [0; 64];
        assert!(d.apply(no_pop).is_err());
        assert_eq!(d.page(None, 16).unwrap().len(), 4);
    }
}
