//! Fail-closed isolated-network profile. Discovery supplies endpoints, never votes.
//! Numeric RFC1918 endpoints only; public deployment is intentionally unavailable.
use eyre::{eyre, Result};
use std::{collections::BTreeMap, net::SocketAddrV4, path::Path};
use yami_chain::{
    codec::{Reader, Writer},
    committee::{Epoch, Member, ValidatorSet},
    Genesis, Signer as _, SigningKey,
};
use yami_chain_storage::read_bounded;

const MAX_BYTES: usize = 32768;
const DOMAIN: &[u8] = b"YC-PEERS-1";

#[derive(Clone)]
pub struct IsolatedNetwork {
    pub(crate) index: u8,
    pub(crate) directory: std::path::PathBuf,
    records: BTreeMap<[u8; 32], Vec<u8>>,
    pub(crate) hints: BTreeMap<[u8; 32], (u64, SocketAddrV4)>,
    pub(crate) genesis: Genesis,
    pub(crate) consensus: SigningKey,
    pub(crate) transport: SigningKey,
    pub(crate) listen: SocketAddrV4,
    pub(crate) peers: Vec<(SocketAddrV4, [u8; 32])>,
}
fn endpoint(r: &mut Reader<'_>) -> yami_chain::Result<SocketAddrV4> {
    Ok(SocketAddrV4::new(r.array::<4>()?.into(), r.u16()?))
}
fn allowed(a: SocketAddrV4) -> bool {
    a.ip().is_private()
        && a.ip().octets()[3] != 0
        && a.ip().octets()[3] != 255
        && a.port() >= 1024
        && a.port() < u16::MAX
}
fn preimage(g: &Genesis, key: &[u8; 32], a: SocketAddrV4, expires: u64) -> Vec<u8> {
    let mut w = Writer::default();
    w.raw(DOMAIN);
    w.raw(&g.id().0);
    w.raw(key);
    w.raw(&a.ip().octets());
    w.u16(a.port());
    w.u64(expires);
    w.0
}
impl IsolatedNetwork {
    pub fn genesis(&self) -> &Genesis {
        &self.genesis
    }
    /// Bootstrap files are untrusted. Check every record before returning any dial target.
    /// Lifetime bounds stop old endpoint hints being usable indefinitely after restart.
    pub fn load(directory: &Path, index: u8, now: u64) -> Result<Self> {
        Self::load_sources(directory, index, now, &[])
    }
    pub fn load_with_store(directory: &Path, index: u8, now: u64, store: &Path) -> Result<Self> {
        match read_bounded(&store.join("network-hints.bin"), MAX_BYTES) {
            Ok(bytes) => {
                if bytes.len() < 33 {
                    return Err(eyre!("truncated discovery checkpoint"));
                }
                let n = Self::load_sources(directory, index, now, &[&bytes[32..]])?;
                if bytes[..32] != n.genesis.id().0 {
                    return Err(eyre!("discovery checkpoint genesis mismatch"));
                }
                Ok(n)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::load(directory, index, now),
            Err(e) => Err(e.into()),
        }
    }
    pub(crate) fn refresh(&self, page: Option<&[u8]>, now: u64) -> Result<Self> {
        if page.is_some_and(|b| b.is_empty() || b.len() > 1761 || b[0] > 16) {
            return Err(eyre!("oversized discovery gossip page"));
        }
        let mut remembered = vec![self.records.len() as u8];
        for record in self.records.values() {
            remembered.extend_from_slice(record);
        }
        let mut sources = vec![remembered.as_slice()];
        if let Some(page) = page {
            sources.push(page);
        }
        Self::load_sources(&self.directory, self.index, now, &sources)
    }
    pub(crate) fn page(&self, cursor: usize) -> Vec<u8> {
        let count = self.records.len().min(16);
        let mut out = vec![count as u8];
        for record in self
            .records
            .values()
            .cycle()
            .skip(cursor % self.records.len())
            .take(count)
        {
            out.extend_from_slice(record);
        }
        out
    }
    fn load_sources(directory: &Path, index: u8, now: u64, additional: &[&[u8]]) -> Result<Self> {
        let bytes = read_bounded(&directory.join("roster.bin"), MAX_BYTES)?;
        let mut r = Reader::new(&bytes, MAX_BYTES)?;
        if r.take(10)? != b"YC-ROSTER1" {
            return Err(eyre!("invalid pinned roster"));
        }
        let n = r.u8()? as usize;
        if !(4..=100).contains(&n) || index as usize >= n {
            return Err(eyre!("invalid isolated roster size/index"));
        }
        let mut members = Vec::new();
        for _ in 0..n {
            members.push(Member {
                consensus_key: r.array()?,
                transport_key: r.array()?,
                power: 1,
            });
        }
        r.end()?;
        let own = members[index as usize].clone();
        // Public fixture keys must never escape loopback through this profile.
        static KNOWN: std::sync::OnceLock<std::collections::BTreeSet<[u8; 32]>> =
            std::sync::OnceLock::new();
        let known = KNOWN.get_or_init(|| {
            (0..100)
                .flat_map(|i| {
                    ["consensus", "transport"].map(|role| {
                        yami_chain::development_key(role, i)
                            .verifying_key()
                            .to_bytes()
                    })
                })
                .collect()
        });
        if members
            .iter()
            .any(|m| known.contains(&m.consensus_key) || known.contains(&m.transport_key))
        {
            return Err(eyre!("development key in isolated roster"));
        }
        let mut genesis = Genesis::local();
        genesis.epochs = vec![Epoch {
            first_height: 1,
            validators: ValidatorSet::new(members)?,
        }];
        genesis.validate()?;
        let bytes = read_bounded(&directory.join(format!("identity-{index}.bin")), 64)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if std::fs::metadata(directory.join(format!("identity-{index}.bin")))?
                .permissions()
                .mode()
                & 0o077
                != 0
            {
                return Err(eyre!("identity must be owner-only"));
            }
        }
        if bytes.len() != 64 {
            return Err(eyre!("identity length"));
        }
        let consensus = SigningKey::from_bytes(&bytes[..32].try_into()?);
        let transport = SigningKey::from_bytes(&bytes[32..].try_into()?);
        if consensus.verifying_key().to_bytes() != own.consensus_key
            || transport.verifying_key().to_bytes() != own.transport_key
        {
            return Err(eyre!("identity does not match pinned roster"));
        }
        let mut peers = BTreeMap::new();
        let mut hints = BTreeMap::<[u8; 32], (u64, SocketAddrV4)>::new();
        let mut records = BTreeMap::new();
        let files = (0..2)
            .map(|source| {
                read_bounded(
                    &directory.join(format!("bootstrap-{source}.bin")),
                    MAX_BYTES,
                )
            })
            .collect::<std::io::Result<Vec<_>>>()?;
        for bytes in files
            .iter()
            .map(Vec::as_slice)
            .chain(additional.iter().copied())
        {
            let mut r = Reader::new(bytes, MAX_BYTES)?;
            let count = r.u8()? as usize;
            if count > n {
                return Err(eyre!("oversized discovery table"));
            }
            for _ in 0..count {
                let key = r.array::<32>()?;
                let a = endpoint(&mut r)?;
                let expires = r.u64()?;
                let signature = r.array::<64>()?;
                if !allowed(a) || expires <= now || expires > now.saturating_add(3600) {
                    return Err(eyre!("invalid discovery address or lifetime"));
                }
                let member = genesis.epochs[0]
                    .validators
                    .member(&key)
                    .ok_or_else(|| eyre!("discovery identity not pinned"))?;
                yami_chain::verify(&key, &preimage(&genesis, &key, a, expires), &signature)?;
                if let Some((old_expires, old_address)) = hints.get(&key) {
                    if expires < *old_expires {
                        continue;
                    }
                    if expires == *old_expires && a != *old_address {
                        return Err(eyre!("conflicting discovery sources"));
                    }
                }
                let mut record = Writer::default();
                record.raw(&key);
                record.raw(&a.ip().octets());
                record.u16(a.port());
                record.u64(expires);
                record.raw(&signature);
                records.insert(key, record.0);
                hints.insert(key, (expires, a));
                peers.insert(key, (a, member.transport_key));
            }
            r.end()?;
        }
        let listen = peers
            .remove(&own.consensus_key)
            .ok_or_else(|| eyre!("self announcement missing"))?
            .0;
        if peers.len() < 2 {
            return Err(eyre!("at least two authenticated bootstrap peers required"));
        }
        Ok(Self {
            directory: directory.to_owned(),
            records,
            hints,
            index,
            genesis,
            consensus,
            transport,
            listen,
            peers: peers.into_values().collect(),
        })
    }
    /// Persist per-identity freshness under the journal's exclusive lock. Tombstones
    /// remain bounded by the pinned roster; an omitted hint cannot erase its history.
    pub(crate) fn checkpoint(&self, journal: &yami_chain_storage::Journal) -> Result<()> {
        let path = journal.path().join("network-hints.bin");
        let mut high = BTreeMap::new();
        match read_bounded(&path, MAX_BYTES) {
            Ok(bytes) => {
                let mut r = Reader::new(&bytes, MAX_BYTES)?;
                if r.array::<32>()? != self.genesis.id().0 {
                    return Err(eyre!("discovery checkpoint genesis mismatch"));
                }
                let n = r.u8()? as usize;
                if n > 100 {
                    return Err(eyre!("discovery checkpoint size"));
                }
                for _ in 0..n {
                    let key = r.array::<32>()?;
                    let a = endpoint(&mut r)?;
                    let expiry = r.u64()?;
                    let signature = r.array::<64>()?;
                    yami_chain::verify(
                        &key,
                        &preimage(&self.genesis, &key, a, expiry),
                        &signature,
                    )?;
                    let mut record = Writer::default();
                    record.raw(&key);
                    record.raw(&a.ip().octets());
                    record.u16(a.port());
                    record.u64(expiry);
                    record.raw(&signature);
                    if !self.genesis.has_consensus_key(&key)
                        || high.insert(key, (expiry, a, record.0)).is_some()
                    {
                        return Err(eyre!("invalid discovery checkpoint"));
                    }
                }
                r.end()?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
        for (key, hint) in &self.hints {
            if high
                .get(key)
                .is_some_and(|old| hint.0 < old.0 || (hint.0 == old.0 && hint.1 != old.1))
            {
                return Err(eyre!("replayed discovery hint"));
            }
            high.insert(*key, (hint.0, hint.1, self.records[key].clone()));
        }
        let mut w = Writer::default();
        w.raw(&self.genesis.id().0);
        w.u8(high.len() as u8);
        for (_, (_, _, record)) in high {
            w.raw(&record);
        }
        if read_bounded(&path, MAX_BYTES).ok().as_deref() == Some(w.0.as_slice()) {
            return Ok(());
        }
        journal.replace_network_hints(&w.0)?;
        Ok(())
    }
}
/// Generates independent disposable lab identities; never prints secrets or overwrites files.
pub fn initialize_isolated_network(
    directory: &Path,
    endpoints: &[SocketAddrV4],
    now: u64,
) -> Result<()> {
    use std::io::{Read as _, Write as _};
    if !(4..=100).contains(&endpoints.len())
        || endpoints.iter().any(|a| !allowed(*a))
        || endpoints
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != endpoints.len()
    {
        return Err(eyre!("need 4..100 unique private IPv4 endpoints"));
    }
    std::fs::create_dir(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    }
    let mut random = std::fs::File::open("/dev/urandom")?;
    let mut keys = Vec::new();
    let mut members = Vec::new();
    for i in 0..endpoints.len() {
        let mut bytes = [0; 64];
        random.read_exact(&mut bytes)?;
        let c = SigningKey::from_bytes(&bytes[..32].try_into()?);
        let t = SigningKey::from_bytes(&bytes[32..].try_into()?);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(directory.join(format!("identity-{i}.bin")))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        members.push(Member {
            consensus_key: c.verifying_key().to_bytes(),
            transport_key: t.verifying_key().to_bytes(),
            power: 1,
        });
        keys.push(c);
    }
    let mut w = Writer::default();
    w.raw(b"YC-ROSTER1");
    w.u8(members.len() as u8);
    for m in &members {
        w.raw(&m.consensus_key);
        w.raw(&m.transport_key);
    }
    std::fs::write(directory.join("roster.bin"), w.0)?;
    let mut g = Genesis::local();
    g.epochs = vec![Epoch {
        first_height: 1,
        validators: ValidatorSet::new(members)?,
    }];
    for source in 0..2 {
        let mut w = Writer::default();
        w.u8(endpoints.len() as u8);
        for (key, a) in keys.iter().zip(endpoints) {
            let k = key.verifying_key().to_bytes();
            let expires = now
                .checked_add(3600)
                .ok_or_else(|| eyre!("clock overflow"))?;
            w.raw(&k);
            w.raw(&a.ip().octets());
            w.u16(a.port());
            w.u64(expires);
            w.raw(&key.sign(&preimage(&g, &k, *a, expires)).to_bytes());
        }
        std::fs::write(directory.join(format!("bootstrap-{source}.bin")), w.0)?;
    }
    Ok(())
}

/// Publish only the caller's authenticated endpoint. Replicate this public file
/// through independent bootstrap sources; never distribute identity files.
pub fn announce_isolated_endpoint(
    directory: &Path,
    index: u8,
    address: SocketAddrV4,
    now: u64,
) -> Result<()> {
    if !allowed(address) {
        return Err(eyre!("only private IPv4 lab endpoints are supported"));
    }
    let n = IsolatedNetwork::load(directory, index, now)?;
    let key = n.consensus.verifying_key().to_bytes();
    let expires = now
        .checked_add(3600)
        .ok_or_else(|| eyre!("clock overflow"))?;
    if n.hints.get(&key).is_some_and(|old| expires <= old.0) {
        return Err(eyre!("announcement clock must advance"));
    }
    let mut w = Writer::default();
    w.u8(1);
    w.raw(&key);
    w.raw(&address.ip().octets());
    w.u16(address.port());
    w.u64(expires);
    w.raw(
        &n.consensus
            .sign(&preimage(&n.genesis, &key, address, expires))
            .to_bytes(),
    );
    let path = directory.join("bootstrap-0.bin");
    let temporary = directory.join("bootstrap-0.next");
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(&w.0)?;
    file.sync_all()?;
    std::fs::rename(temporary, path)?;
    std::fs::File::open(directory)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Lab(std::path::PathBuf);
    impl Lab {
        fn new() -> Self {
            static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let p = std::env::temp_dir().join(format!(
                "yc-network-{}-{}",
                std::process::id(),
                ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            initialize_isolated_network(
                &p,
                &(1..=4)
                    .map(|i| format!("10.81.0.{i}:28100").parse().unwrap())
                    .collect::<Vec<_>>(),
                100,
            )
            .unwrap();
            Self(p)
        }
        fn load(&self) -> Result<IsolatedNetwork> {
            IsolatedNetwork::load(&self.0, 0, 101)
        }
    }
    impl Drop for Lab {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn independent_identity_and_authenticated_bootstraps() {
        let a = Lab::new();
        let b = Lab::new();
        let n = a.load().unwrap();
        assert_eq!(n.peers.len(), 3);
        assert_ne!(n.genesis, b.load().unwrap().genesis);
        assert_ne!(n.genesis, Genesis::local());
        // Imported discovery cannot change the pinned genesis or grant voting power.
        std::fs::copy(b.0.join("bootstrap-0.bin"), a.0.join("bootstrap-0.bin")).unwrap();
        assert!(a.load().is_err());
    }
    #[test]
    fn forged_truncated_oversized_expired_and_future_hints_fail_closed() {
        let a = Lab::new();
        assert!(IsolatedNetwork::load(&a.0, 0, 3700).is_err());
        assert!(IsolatedNetwork::load(&a.0, 0, 0).is_err());
        assert!(IsolatedNetwork::load(&a.0, 4, 101).is_err());
        let path = a.0.join("bootstrap-0.bin");
        let original = std::fs::read(&path).unwrap();
        for len in 0..original.len() {
            std::fs::write(&path, &original[..len]).unwrap();
            assert!(a.load().is_err(), "truncation {len}");
        }
        for offset in [0, 1, 33, 37, 39, 47, 110] {
            let mut bytes = original.clone();
            bytes[offset] ^= 1;
            std::fs::write(&path, bytes).unwrap();
            assert!(a.load().is_err(), "tamper {offset}");
        }
        std::fs::write(&path, vec![0; MAX_BYTES + 1]).unwrap();
        assert!(a.load().is_err());
    }
    #[test]
    fn identity_permissions_and_roster_binding() {
        let a = Lab::new();
        let b = Lab::new();
        std::fs::copy(b.0.join("identity-0.bin"), a.0.join("identity-0.bin")).unwrap();
        assert!(a.load().is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                b.0.join("identity-0.bin"),
                std::fs::Permissions::from_mode(0o644),
            )
            .unwrap();
            assert!(b.load().is_err());
        }
    }
    #[test]
    fn discovery_pressure_preserves_durable_freshness_on_restart() {
        use yami_chain_storage::{Journal, StorageLimits};
        let a = Lab::new();
        let store = a.0.join("store");
        let journal = Journal::open(&store).unwrap();
        a.load().unwrap().checkpoint(&journal).unwrap();
        let original = std::fs::read(store.join("network-hints.bin")).unwrap();
        drop(journal);
        let journal = Journal::open_with_limits(
            &store,
            100,
            StorageLimits {
                max_bytes: 1,
                reserve_bytes: 0,
            },
        )
        .unwrap();
        // An unchanged freshness record needs no new allocation.
        a.load().unwrap().checkpoint(&journal).unwrap();
        announce_isolated_endpoint(&a.0, 0, "10.81.0.9:28100".parse().unwrap(), 101).unwrap();
        assert!(a.load().unwrap().checkpoint(&journal).is_err());
        assert_eq!(
            std::fs::read(store.join("network-hints.bin")).unwrap(),
            original
        );
        assert!(!store.join("network-hints.next").exists());
        drop(journal);
        let journal = Journal::open(&store).unwrap();
        a.load().unwrap().checkpoint(&journal).unwrap();
        assert_ne!(
            std::fs::read(store.join("network-hints.bin")).unwrap(),
            original
        );
    }
    #[test]
    fn signed_endpoint_updates_reject_rollback_after_restart() {
        let a = Lab::new();
        let store = a.0.join("store");
        let journal = yami_chain_storage::Journal::open(&store).unwrap();
        a.load().unwrap().checkpoint(&journal).unwrap();
        let original = std::fs::read(a.0.join("bootstrap-0.bin")).unwrap();
        announce_isolated_endpoint(&a.0, 0, "10.81.0.9:28100".parse().unwrap(), 101).unwrap();
        let updated = a.load().unwrap();
        assert_eq!(updated.listen, "10.81.0.9:28100".parse().unwrap());
        updated.checkpoint(&journal).unwrap();
        drop(journal);
        let journal = yami_chain_storage::Journal::open(&store).unwrap();
        std::fs::write(a.0.join("bootstrap-0.bin"), original).unwrap();
        assert!(a.load().unwrap().checkpoint(&journal).is_err());
    }
    #[tokio::test]
    async fn independent_proof_binds_pinned_consensus_and_noise_identity() {
        use malachite_signing::Verifier as _;
        let a = Lab::new();
        let n = a.load().unwrap();
        let key = n.consensus.verifying_key().to_bytes();
        let peer = malachite::app::types::Keypair::ed25519_from_bytes(n.transport.to_bytes())
            .unwrap()
            .public()
            .to_peer_id()
            .to_bytes();
        let mut proof = malachite::app::types::core::ValidatorProof::<crate::Context>::new(
            key.to_vec(),
            peer,
            [0; 64],
        );
        proof.signature = n.consensus.sign(&proof.preimage()).to_bytes();
        let verify = crate::signing::SignatureVerifier(n.genesis.clone());
        assert!(verify
            .verify_validator_proof(&proof)
            .await
            .unwrap()
            .is_valid());
        let b = Lab::new();
        let other = b.load().unwrap();
        proof.peer_id =
            malachite::app::types::Keypair::ed25519_from_bytes(other.transport.to_bytes())
                .unwrap()
                .public()
                .to_peer_id()
                .to_bytes();
        proof.signature = n.consensus.sign(&proof.preimage()).to_bytes();
        assert!(!verify
            .verify_validator_proof(&proof)
            .await
            .unwrap()
            .is_valid());
        proof.public_key = other.consensus.verifying_key().to_bytes().to_vec();
        proof.signature = other.consensus.sign(&proof.preimage()).to_bytes();
        assert!(!verify
            .verify_validator_proof(&proof)
            .await
            .unwrap()
            .is_valid());
    }
    #[test]
    fn bounded_gossip_discovers_missing_pinned_peer_and_survives_restart() {
        let a = Lab::new();
        let full = a.load().unwrap();
        // Keep self and two anchors; the fourth endpoint is unknown at startup.
        let self_key = full.consensus.verifying_key().to_bytes();
        let remove = *full.records.keys().find(|key| **key != self_key).unwrap();
        let mut thin = vec![3];
        for (key, record) in &full.records {
            if *key != remove {
                thin.extend_from_slice(record);
            }
        }
        for source in 0..2 {
            std::fs::write(a.0.join(format!("bootstrap-{source}.bin")), &thin).unwrap();
        }
        let initial = a.load().unwrap();
        assert_eq!(initial.peers.len(), 2);
        let page = full.page(0);
        assert!(page.len() <= 1761);
        let discovered = initial.refresh(Some(&page), 101).unwrap();
        assert_eq!(discovered.peers.len(), 3);
        let store = a.0.join("store");
        let journal = yami_chain_storage::Journal::open(&store).unwrap();
        discovered.checkpoint(&journal).unwrap();
        drop(journal);
        let restored = IsolatedNetwork::load_with_store(&a.0, 0, 101, &store).unwrap();
        assert_eq!(restored.peers.len(), 3);
        let mut forged = page.clone();
        *forged.last_mut().unwrap() ^= 1;
        assert!(initial.refresh(Some(&forged), 101).is_err());
        assert!(initial.refresh(Some(&vec![0; 1762]), 101).is_err());
        let mut oversized_count = page;
        oversized_count[0] = 17;
        assert!(initial.refresh(Some(&oversized_count), 101).is_err());
    }
    #[test]
    fn no_public_wildcard_loopback_or_duplicate_endpoints() {
        for address in [
            "0.0.0.0:28100",
            "127.0.0.1:28100",
            "8.8.8.8:28100",
            "10.1.0.255:28100",
            "10.1.0.1:80",
        ] {
            assert!(!allowed(address.parse().unwrap()));
        }
        assert!(allowed("172.18.0.2:28100".parse().unwrap()));
    }
}
