//! Discovery supplies endpoints, never votes. The isolated profile retains its
//! private-IPv4 gate; software live-PoS discovery is separately feature-gated.
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

/// Permissionless software-PoS discovery. This does not open the isolated-network
/// address gate or any production financial verifier. Discovery keys supply
/// availability only; finalized committees remain the sole voting authority.
#[cfg(feature = "insecure-devnet-input")]
pub(crate) mod live {
    use super::*;
    use std::{
        collections::{BTreeMap, BTreeSet},
        net::{Ipv4Addr, SocketAddrV4},
    };
    const PAGE: &[u8; 8] = b"YCLIVE01";
    const TRANSACTION: &[u8; 8] = b"YCTXGSP1";
    const MAX_PEERS: usize = 40;
    pub(crate) const MAX_PACKET: usize = 32768;
    const ADVERT_BYTES: usize = 238;
    const LIFETIME: u64 = 600;

    #[derive(Clone, PartialEq, Eq)]
    pub(crate) struct Advert {
        network: [u8; 32],
        pub(crate) consensus: [u8; 32],
        pub(crate) transport: [u8; 32],
        pub(crate) address: SocketAddrV4,
        expires: u64,
        signatures: [[u8; 64]; 2],
    }
    fn network(genesis: &Genesis) -> Result<[u8; 32]> {
        let scope = genesis
            .live_scope()
            .ok_or_else(|| eyre!("live PoS policy required for discovery"))?;
        let mut raw = genesis.id().0.to_vec();
        raw.extend_from_slice(&scope);
        Ok(yami_chain::hash(b"YC-LIVE-NETWORK-1", &raw).0)
    }
    fn address_allowed(address: SocketAddrV4) -> bool {
        let ip = address.ip();
        let first = ip.octets()[0];
        !ip.is_unspecified()
            && !ip.is_multicast()
            && !ip.is_broadcast()
            && !ip.is_link_local()
            && first != 0
            && first < 224
            && (1024..65535).contains(&address.port())
    }
    fn fixture_key(key: &[u8; 32]) -> bool {
        static KEYS: std::sync::OnceLock<BTreeSet<[u8; 32]>> = std::sync::OnceLock::new();
        KEYS.get_or_init(|| {
            (0..100)
                .flat_map(|i| {
                    ["consensus", "transport"].map(|role| {
                        yami_chain::development_key(role, i)
                            .verifying_key()
                            .to_bytes()
                    })
                })
                .collect()
        })
        .contains(key)
    }
    impl Advert {
        fn message(&self) -> Vec<u8> {
            let mut w = Writer::default();
            w.raw(PAGE);
            w.raw(&self.network);
            w.raw(&self.consensus);
            w.raw(&self.transport);
            w.raw(&self.address.ip().octets());
            w.u16(self.address.port());
            w.u64(self.expires);
            w.0
        }
        fn new(
            g: &Genesis,
            consensus: &SigningKey,
            transport: &SigningKey,
            address: SocketAddrV4,
            now: u64,
        ) -> Result<Self> {
            let mut result = Self {
                network: network(g)?,
                consensus: consensus.verifying_key().to_bytes(),
                transport: transport.verifying_key().to_bytes(),
                address,
                expires: now
                    .checked_add(LIFETIME)
                    .ok_or_else(|| eyre!("discovery clock overflow"))?,
                signatures: [[0; 64]; 2],
            };
            result.signatures = [
                consensus.sign(&result.message()).to_bytes(),
                transport.sign(&result.message()).to_bytes(),
            ];
            result.verify(g, now)?;
            Ok(result)
        }
        fn encode(&self, w: &mut Writer) {
            w.raw(&self.network);
            w.raw(&self.consensus);
            w.raw(&self.transport);
            w.raw(&self.address.ip().octets());
            w.u16(self.address.port());
            w.u64(self.expires);
            for signature in self.signatures {
                w.raw(&signature);
            }
        }
        fn decode(r: &mut Reader<'_>) -> Result<Self> {
            Ok(Self {
                network: r.array()?,
                consensus: r.array()?,
                transport: r.array()?,
                address: SocketAddrV4::new(Ipv4Addr::from(r.array::<4>()?), r.u16()?),
                expires: r.u64()?,
                signatures: [r.array()?, r.array()?],
            })
        }
        fn verify(&self, g: &Genesis, now: u64) -> Result<()> {
            if self.network != network(g)?
                || self.consensus == self.transport
                || !address_allowed(self.address)
                || self.expires <= now
                || self.expires > now.saturating_add(LIFETIME + 30)
            {
                return Err(eyre!("invalid discovery network, endpoint or lifetime"));
            }
            if !self.address.ip().is_loopback()
                && (fixture_key(&self.consensus) || fixture_key(&self.transport))
            {
                return Err(eyre!("public fixture identities are loopback only"));
            }
            if let Some(expected) = g.live_transport(&self.consensus) {
                if expected != self.transport {
                    return Err(eyre!("finalized PoS transport binding mismatch"));
                }
            } else if g.is_reserved_key(&self.consensus) || g.is_reserved_key(&self.transport) {
                return Err(eyre!("reserved identity cannot be relabeled by discovery"));
            }
            yami_chain::verify(&self.consensus, &self.message(), &self.signatures[0])?;
            yami_chain::verify(&self.transport, &self.message(), &self.signatures[1])?;
            Ok(())
        }
    }
    pub(crate) struct View {
        genesis: Genesis,
        signing: SigningKey,
        transport: SigningKey,
        own: Advert,
        records: BTreeMap<[u8; 32], Advert>,
        height: u64,
        seeds: Vec<SocketAddrV4>,
    }
    pub(crate) fn settings(
        index: u8,
        peer_base: u16,
    ) -> Result<Option<(SocketAddrV4, Vec<SocketAddrV4>)>> {
        if std::env::var("YAMI_NATIVE_DISCOVERY").ok().as_deref() != Some("1") {
            return Ok(None);
        }
        let address = std::env::var("YAMI_NATIVE_DISCOVERY_LISTEN")
            .unwrap_or_else(|_| format!("127.0.0.1:{}", peer_base + u16::from(index)))
            .parse::<SocketAddrV4>()?;
        let seeds = std::env::var("YAMI_NATIVE_DISCOVERY_SEEDS")
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.is_empty())
            .map(str::parse)
            .collect::<std::result::Result<Vec<SocketAddrV4>, _>>()?;
        if seeds.len() > 8
            || !address_allowed(address)
            || seeds.iter().any(|a| !address_allowed(*a))
        {
            return Err(eyre!("invalid bounded discovery configuration"));
        }
        Ok(Some((address, seeds)))
    }
    impl View {
        pub(crate) fn new(
            g: Genesis,
            signing: SigningKey,
            transport: SigningKey,
            address: SocketAddrV4,
            seeds: Vec<SocketAddrV4>,
            journal: &yami_chain_storage::Journal,
            now: u64,
        ) -> Result<Self> {
            let own = Advert::new(&g, &signing, &transport, address, now)?;
            let mut result = Self {
                genesis: g,
                signing,
                transport,
                own,
                records: BTreeMap::new(),
                height: 1,
                seeds,
            };
            match read_bounded(&journal.path().join("network-hints.bin"), 32768) {
                Ok(raw) => {
                    let mut r = Reader::new(&raw, 32768)?;
                    if r.take(8)? != PAGE {
                        return Err(eyre!("discovery checkpoint profile mismatch"));
                    }
                    let count = r.u8()? as usize;
                    if count > MAX_PEERS {
                        return Err(eyre!("discovery checkpoint capacity"));
                    }
                    for _ in 0..count {
                        let advert = Advert::decode(&mut r)?;
                        if advert.network != result.own.network {
                            return Err(eyre!("discovery checkpoint network mismatch"));
                        }
                        if advert.expires > now {
                            result.admit(advert, now)?;
                        }
                    }
                    r.end()?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Err(error) => return Err(error.into()),
            }
            Ok(result)
        }
        pub(crate) fn set_height(&mut self, height: u64) {
            self.height = height;
        }
        fn protected(&self, key: &[u8; 32]) -> bool {
            self.genesis
                .committee_at(self.height)
                .is_ok_and(|c| c.member(key).is_some())
        }
        fn admit(&mut self, advert: Advert, now: u64) -> Result<()> {
            advert.verify(&self.genesis, now)?;
            if advert.consensus == self.own.consensus {
                if advert.transport != self.own.transport {
                    return Err(eyre!("own discovery identity mismatch"));
                }
                return Ok(());
            }
            if advert.address == self.own.address
                || advert.transport == self.own.transport
                || advert.consensus == self.own.transport
                || advert.transport == self.own.consensus
            {
                return Err(eyre!("own endpoint/keys cannot be relabeled"));
            }
            if let Some(old) = self.records.get(&advert.consensus) {
                if advert.expires < old.expires || (advert.expires == old.expires && advert != *old)
                {
                    return Err(eyre!("replayed or conflicting endpoint advertisement"));
                }
            }
            if self.records.values().any(|r| {
                r.consensus != advert.consensus
                    && (r.transport == advert.transport
                        || r.address == advert.address
                        || r.transport == advert.consensus
                        || r.consensus == advert.transport)
            }) {
                return Err(eyre!("duplicate discovery endpoint or identity"));
            }
            let per_ip = if advert.address.ip().is_loopback() {
                16
            } else {
                4
            };
            if self
                .records
                .values()
                .filter(|r| {
                    r.consensus != advert.consensus && r.address.ip() == advert.address.ip()
                })
                .count()
                >= per_ip
            {
                return Err(eyre!("discovery per-IP capacity"));
            }
            if !self.records.contains_key(&advert.consensus) && self.records.len() >= MAX_PEERS {
                // The bounded hint cache is not a fixed permissioned roster.
                // New authenticated candidates may replace other nonvoting hints;
                // active finalized identities cannot be displaced this way.
                let victim = self
                    .records
                    .keys()
                    .find(|key| !self.protected(key))
                    .copied()
                    .ok_or_else(|| eyre!("discovery protected-peer capacity"))?;
                self.records.remove(&victim);
            }
            self.records.insert(advert.consensus, advert);
            Ok(())
        }
        pub(crate) fn refresh(&mut self, now: u64) -> Result<()> {
            self.records
                .retain(|_, a| a.verify(&self.genesis, now).is_ok());
            self.own = Advert::new(
                &self.genesis,
                &self.signing,
                &self.transport,
                self.own.address,
                now,
            )?;
            Ok(())
        }
        pub(crate) fn checkpoint(&self, journal: &yami_chain_storage::Journal) -> Result<()> {
            let mut w = Writer::default();
            w.raw(PAGE);
            w.u8(self.records.len() as u8);
            for record in self.records.values() {
                record.encode(&mut w);
            }
            if read_bounded(&journal.path().join("network-hints.bin"), 32768)
                .ok()
                .as_deref()
                != Some(&w.0)
            {
                journal.replace_network_hints(&w.0)?;
            }
            Ok(())
        }
        pub(crate) fn page(&self, cursor: usize) -> Vec<u8> {
            let mut w = Writer::default();
            w.raw(PAGE);
            w.u8((1 + self.records.len().min(7)) as u8);
            self.own.encode(&mut w);
            if !self.records.is_empty() {
                for record in self
                    .records
                    .values()
                    .cycle()
                    .skip(cursor % self.records.len())
                    .take(7.min(self.records.len()))
                {
                    record.encode(&mut w);
                }
            }
            w.0
        }
        pub(crate) fn targets(&self, cursor: usize) -> Vec<SocketAddrV4> {
            let mut addresses: BTreeSet<_> = self
                .seeds
                .iter()
                .copied()
                .filter(|a| *a != self.own.address)
                .collect();
            if !self.records.is_empty() {
                for record in self
                    .records
                    .values()
                    .cycle()
                    .skip(cursor % self.records.len())
                    .take(8)
                {
                    addresses.insert(record.address);
                }
            }
            addresses.into_iter().take(16).collect()
        }
        pub(crate) fn peers(&self, height: u64) -> Vec<(SocketAddrV4, [u8; 32])> {
            let committee = self.genesis.committee_at(height).ok();
            let mut records: Vec<_> = self.records.values().collect();
            records.sort_by_key(|a| {
                (
                    committee
                        .as_ref()
                        .is_none_or(|c| c.member(&a.consensus).is_none()),
                    a.consensus,
                )
            });
            records
                .into_iter()
                .take(16)
                .map(|a| (a.address, a.transport))
                .collect()
        }
        pub(crate) fn ingest_page(
            &mut self,
            raw: &[u8],
            source: SocketAddrV4,
            now: u64,
        ) -> Result<()> {
            let mut r = Reader::new(raw, 8 + 1 + 8 * ADVERT_BYTES)?;
            if r.take(8)? != PAGE {
                return Err(eyre!("discovery packet domain"));
            }
            let count = r.u8()? as usize;
            if !(1..=8).contains(&count) {
                return Err(eyre!("discovery page capacity"));
            }
            let first = Advert::decode(&mut r)?;
            if first.address != source {
                return Err(eyre!("discovery source endpoint mismatch"));
            }
            // Validate every bounded record before changing the directory.
            let mut records = vec![first];
            for _ in 1..count {
                records.push(Advert::decode(&mut r)?);
            }
            r.end()?;
            for advert in &records {
                advert.verify(&self.genesis, now)?;
            }
            for advert in records {
                self.admit(advert, now)?;
            }
            Ok(())
        }
        pub(crate) fn transaction(&self, height: u64, payload: &[u8]) -> Result<Vec<u8>> {
            if payload.len() > 30000 {
                return Err(eyre!("transaction gossip byte cap"));
            }
            let mut w = Writer::default();
            w.raw(TRANSACTION);
            self.own.encode(&mut w);
            w.u64(height);
            w.bytes(payload)?;
            let digest = yami_chain::hash(b"YC-LIVE-TRANSACTION-1", &w.0);
            w.raw(&self.transport.sign(&digest.0).to_bytes());
            Ok(w.0)
        }
        pub(crate) fn receive_transaction(
            &mut self,
            raw: &[u8],
            source: SocketAddrV4,
            now: u64,
        ) -> Result<(u64, Vec<u8>)> {
            let mut r = Reader::new(raw, MAX_PACKET)?;
            if r.take(8)? != TRANSACTION {
                return Err(eyre!("transaction gossip domain"));
            }
            let advert = Advert::decode(&mut r)?;
            if advert.address != source {
                return Err(eyre!("transaction gossip source"));
            }
            advert.verify(&self.genesis, now)?;
            let height = r.u64()?;
            let payload = r.bytes(30000)?.to_vec();
            let signature = r.array::<64>()?;
            r.end()?;
            let digest = yami_chain::hash(b"YC-LIVE-TRANSACTION-1", &raw[..raw.len() - 64]);
            yami_chain::verify(&advert.transport, &digest.0, &signature)?;
            self.admit(advert, now)?;
            Ok((height, payload))
        }
        pub(crate) fn is_page(raw: &[u8]) -> bool {
            raw.starts_with(PAGE)
        }
    }
    /// A fresh window has finite verification and receive budgets. Source maps
    /// are bounded and never allocate one entry per packet or advertised identity.
    pub(crate) struct Admission {
        sources: BTreeMap<Ipv4Addr, u8>,
        remaining: u16,
    }
    impl Admission {
        pub(crate) fn new() -> Self {
            Self {
                sources: BTreeMap::new(),
                remaining: 32,
            }
        }
        pub(crate) fn allow(&mut self, source: Ipv4Addr) -> bool {
            if self.remaining == 0
                || (self.sources.len() >= 64 && !self.sources.contains_key(&source))
            {
                return false;
            }
            let count = self.sources.entry(source).or_default();
            let limit = if source.is_loopback() { 32 } else { 4 };
            if *count >= limit {
                return false;
            }
            *count += 1;
            self.remaining -= 1;
            true
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        struct Temporary(std::path::PathBuf);
        impl Temporary {
            fn new() -> Self {
                static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();
                let path = std::env::temp_dir().join(format!(
                    "yami-live-peers-{}-{stamp}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                ));
                std::fs::create_dir(&path).unwrap();
                Self(path)
            }
            fn path(&self) -> &Path {
                &self.0
            }
        }
        impl Drop for Temporary {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        fn genesis() -> Genesis {
            let mut g = Genesis::local();
            g.enable_live_committees([7; 32]).unwrap();
            g
        }
        fn fixture(index: u8, port: u16, journal: &yami_chain_storage::Journal) -> View {
            View::new(
                genesis(),
                yami_chain::development_key("consensus", index),
                yami_chain::development_key("transport", index),
                SocketAddrV4::new(Ipv4Addr::LOCALHOST, port),
                vec![],
                journal,
                100,
            )
            .unwrap()
        }
        #[test]
        fn gossip_discovers_unknown_candidates_without_granting_authority_and_survives_restart() {
            let dir = Temporary::new();
            let journal = yami_chain_storage::Journal::open(&dir.path().join("node")).unwrap();
            let mut a = fixture(0, 32000, &journal);
            let mut b = fixture(1, 32001, &journal);
            let c = View::new(
                genesis(),
                SigningKey::from_bytes(&[101; 32]),
                SigningKey::from_bytes(&[102; 32]),
                "127.0.0.1:32002".parse().unwrap(),
                vec![],
                &journal,
                100,
            )
            .unwrap();
            b.ingest_page(&c.page(0), c.own.address, 100).unwrap();
            a.ingest_page(&b.page(0), b.own.address, 100).unwrap();
            assert!(a.records.contains_key(&c.own.consensus));
            assert!(a
                .genesis
                .committee_at(1)
                .unwrap()
                .member(&c.own.consensus)
                .is_none());
            assert!(a.genesis.committee_at(2).is_err());
            a.checkpoint(&journal).unwrap();
            let restored = fixture(0, 32000, &journal);
            assert_eq!(restored.records.len(), 2);
            assert!(restored
                .peers(1)
                .iter()
                .any(|(_, key)| *key == c.own.transport));
        }
        #[test]
        fn source_forgery_replayed_endpoints_wrong_network_and_reserved_keys_refuse() {
            let dir = Temporary::new();
            let journal = yami_chain_storage::Journal::open(&dir.path().join("node")).unwrap();
            let mut a = fixture(0, 32000, &journal);
            let b = fixture(1, 32001, &journal);
            a.ingest_page(&b.page(0), b.own.address, 100).unwrap();
            assert!(a.ingest_page(&b.page(0), a.own.address, 100).is_err());
            let mut forged = b.page(0);
            *forged.last_mut().unwrap() ^= 1;
            assert!(a.ingest_page(&forged, b.own.address, 100).is_err());
            let old = Advert::new(
                &genesis(),
                &b.signing,
                &b.transport,
                "127.0.0.1:32003".parse().unwrap(),
                99,
            )
            .unwrap();
            assert!(a.admit(old, 100).is_err());
            let mut foreign = genesis();
            foreign.live_committees = Default::default();
            foreign.enable_live_committees([8; 32]).unwrap();
            let advert =
                Advert::new(&foreign, &b.signing, &b.transport, b.own.address, 100).unwrap();
            assert!(a.admit(advert, 100).is_err());
            assert!(Advert::new(
                &genesis(),
                &b.signing,
                &SigningKey::from_bytes(&[111; 32]),
                b.own.address,
                100
            )
            .is_err());
            assert!(Advert::new(
                &genesis(),
                &b.signing,
                &b.transport,
                "10.2.3.4:32001".parse().unwrap(),
                100
            )
            .is_err());
            assert!(b.own.verify(&genesis(), 701).is_err());
            assert_eq!(a.records.len(), 1);
        }
        #[test]
        fn bounded_hint_cache_keeps_admitting_candidates_and_protects_finalized_identities() {
            let dir = Temporary::new();
            let journal = yami_chain_storage::Journal::open(&dir.path().join("node")).unwrap();
            let mut a = fixture(0, 32000, &journal);
            let b = fixture(1, 32001, &journal);
            a.admit(b.own.clone(), 100).unwrap();
            for index in 0..39u8 {
                let advert = Advert::new(
                    &genesis(),
                    &SigningKey::from_bytes(&[120 + index; 32]),
                    &SigningKey::from_bytes(&[170 + index; 32]),
                    SocketAddrV4::new(Ipv4Addr::new(10, 1, index, 1), 32002),
                    100,
                )
                .unwrap();
                a.admit(advert, 100).unwrap();
            }
            let newcomer = Advert::new(
                &genesis(),
                &SigningKey::from_bytes(&[240; 32]),
                &SigningKey::from_bytes(&[241; 32]),
                "10.2.3.4:32002".parse().unwrap(),
                100,
            )
            .unwrap();
            let key = newcomer.consensus;
            a.admit(newcomer, 100).unwrap();
            assert_eq!(a.records.len(), MAX_PEERS);
            assert!(a.records.contains_key(&key));
            assert!(a.records.contains_key(&b.own.consensus));
            assert!(a.genesis.committee_at(1).unwrap().member(&key).is_none());
            let mut members = a.genesis.committee_at(1).unwrap().members().to_vec();
            let retired = members
                .iter()
                .position(|m| m.consensus_key == b.own.consensus)
                .unwrap();
            members[retired] = Member {
                consensus_key: key,
                transport_key: a.records[&key].transport,
                power: 1,
            };
            a.genesis
                .install_finalized_committee(2, ValidatorSet::new(members).unwrap())
                .unwrap();
            a.set_height(2);
            assert!(!a.protected(&b.own.consensus));
            assert!(a.protected(&key));
        }
        #[test]
        fn transaction_envelopes_bind_transport_network_payload_and_source_with_bounded_admission()
        {
            let dir = Temporary::new();
            let journal = yami_chain_storage::Journal::open(&dir.path().join("node")).unwrap();
            let mut a = fixture(0, 32000, &journal);
            let b = fixture(1, 32001, &journal);
            let packet = b.transaction(10, b"opaque certificate").unwrap();
            assert_eq!(
                a.receive_transaction(&packet, b.own.address, 100).unwrap(),
                (10, b"opaque certificate".to_vec())
            );
            let mut tampered = packet.clone();
            tampered[260] ^= 1;
            assert!(a
                .receive_transaction(&tampered, b.own.address, 100)
                .is_err());
            assert!(a.receive_transaction(&packet, a.own.address, 100).is_err());
            let mut admission = Admission::new();
            for _ in 0..4 {
                assert!(admission.allow(Ipv4Addr::new(10, 2, 3, 4)));
            }
            assert!(!admission.allow(Ipv4Addr::new(10, 2, 3, 4)));
            let mut admission = Admission::new();
            for _ in 0..32 {
                assert!(admission.allow(Ipv4Addr::LOCALHOST));
            }
            assert!(!admission.allow(Ipv4Addr::LOCALHOST));
            assert!(b.transaction(10, &vec![0; 30001]).is_err());
        }
    }
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
