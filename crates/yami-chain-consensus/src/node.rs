pub use crate::signing::CrashPoint;
use crate::{
    candidates::Candidates,
    signing::{maybe_crash, DurableSigner, SignatureVerifier},
    types::*,
    wire::{
        decode_certificate, encode_certificate, verify_certificate, verify_record, WireCodec,
        MAX_SYNC_VALUES,
    },
};
use bytes::Bytes;
use eyre::{eyre, Result};
use m::Context as _;
use malachite::app::{
    config::{self, NodeConfig},
    engine::host::{Next, SyncedValueOutcome},
    streaming::{StreamContent, StreamId, StreamMessage},
    types::{
        codec::Codec, core as m, sync::RawDecidedValue, Keypair, LocallyProposedValue,
        ProposedValue,
    },
};
use malachite::{
    AppMsg, Channels, ConsensusContext, EngineBuilder, NetworkContext, NetworkIdentity, NetworkMsg,
    RequestContext, SyncContext, WalContext,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use yami_chain::{development_key, Block, Digest, Genesis, State};
use yami_chain_storage::{FinalizedRecord, Journal};

fn native_devnet() -> bool {
    #[cfg(feature = "insecure-devnet-input")]
    {
        std::env::var_os("YAMI_NATIVE_POLICY").is_some()
    }
    #[cfg(not(feature = "insecure-devnet-input"))]
    {
        false
    }
}
#[cfg(feature = "insecure-devnet-input")]
fn publish_native_record(journal: &Journal, record: &FinalizedRecord) -> Result<()> {
    if native_devnet() {
        journal.write_once(
            &format!("native-finalized-{}.bin", record.block.header.height),
            &record.canonical()?,
        )?;
    }
    Ok(())
}
#[derive(Clone)]
struct Config {
    moniker: String,
    consensus: config::ConsensusConfig,
    sync: config::ValueSyncConfig,
}
impl NodeConfig for Config {
    fn moniker(&self) -> &str {
        &self.moniker
    }
    fn consensus(&self) -> &config::ConsensusConfig {
        &self.consensus
    }
    fn consensus_mut(&mut self) -> &mut config::ConsensusConfig {
        &mut self.consensus
    }
    fn value_sync(&self) -> &config::ValueSyncConfig {
        &self.sync
    }
    fn value_sync_mut(&mut self) -> &mut config::ValueSyncConfig {
        &mut self.sync
    }
}
fn node_key(role: &str, index: u8) -> Result<yami_chain::SigningKey> {
    #[cfg(feature = "insecure-devnet-input")]
    if let Some(path) = std::env::var_os("YAMI_NATIVE_KEYS") {
        if !native_devnet() {
            return Err(eyre!("native identity requires pinned devnet policy"));
        }
        let raw = yami_chain_storage::read_bounded(std::path::Path::new(&path), 64)?;
        if raw.len() != 64 {
            return Err(eyre!("native identity length"));
        }
        let offset = if role == "consensus" { 0 } else { 32 };
        return Ok(yami_chain::SigningKey::from_bytes(
            raw[offset..offset + 32].try_into()?,
        ));
    }
    Ok(development_key(role, index))
}
fn transport(index: u8) -> Result<Keypair> {
    Ok(Keypair::ed25519_from_bytes(
        node_key("transport", index)?.to_bytes(),
    )?)
}
fn config(index: u8, base: u16, peer_base: u16, genesis: &Genesis, sparse: bool) -> Result<Config> {
    let mut consensus = config::ConsensusConfig::default();
    consensus.p2p.listen_addr =
        format!("/ip4/127.0.0.1/tcp/{}", base + u16::from(index)).parse()?;
    let scheduled: Vec<_> = (0..100u8)
        .filter(|i| {
            genesis.has_consensus_key(&development_key("consensus", *i).verifying_key().to_bytes())
        })
        .collect();
    let neighbors = sparse_neighbors(&scheduled, index);
    consensus.p2p.persistent_peers = (0..yami_chain::committee::MAX_VALIDATORS as u8)
        .filter(|i| {
            *i != index
                && (!sparse || neighbors.contains(i))
                && genesis
                    .has_consensus_key(&development_key("consensus", *i).verifying_key().to_bytes())
        })
        .map(|i| {
            let peer = Keypair::ed25519_from_bytes(development_key("transport", i).to_bytes())?
                .public()
                .to_peer_id();
            Ok(format!("/ip4/127.0.0.1/tcp/{}/p2p/{peer}", peer_base + u16::from(i)).parse()?)
        })
        .collect::<Result<_>>()?;
    #[cfg(feature = "insecure-devnet-input")]
    if native_devnet() {
        if let Ok(raw) = std::env::var("YAMI_NATIVE_VALIDATOR_ENDPOINTS") {
            let addresses = raw
                .split(',')
                .map(str::parse::<std::net::SocketAddrV4>)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if !(4..=100).contains(&addresses.len())
                || index as usize >= addresses.len()
                || addresses
                    .iter()
                    .any(|a| !(a.ip().is_private() || a.ip().is_loopback()) || a.port() < 1024)
            {
                return Err(eyre!(
                    "4..100 private IPv4 devnet validator endpoint slots required"
                ));
            }
            let own = addresses[index as usize];
            consensus.p2p.listen_addr = format!("/ip4/{}/tcp/{}", own.ip(), own.port()).parse()?;
            consensus.p2p.persistent_peers = addresses
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != index as usize && *i < 4)
                .map(|(i, a)| {
                    let peer = Keypair::ed25519_from_bytes(
                        development_key("transport", i as u8).to_bytes(),
                    )?
                    .public()
                    .to_peer_id();
                    Ok(format!("/ip4/{}/tcp/{}/p2p/{peer}", a.ip(), a.port()).parse()?)
                })
                .collect::<Result<_>>()?;
        }
    }
    consensus.p2p.persistent_peers_only = true;
    #[cfg(feature = "insecure-devnet-input")]
    if genesis.live_scope().is_some() {
        consensus.p2p.persistent_peers_only = false;
    }
    consensus.p2p.discovery.enabled = false;
    consensus.p2p.pubsub_max_size = bytesize::ByteSize::kib(128);
    consensus.p2p.rpc_max_size = bytesize::ByteSize::mib(1);
    consensus.queue_capacity = 4;
    consensus.queue_per_height_capacity = 128;
    let sync = config::ValueSyncConfig {
        status_update_interval: Duration::from_millis(500),
        batch_size: if native_devnet() { 1 } else { MAX_SYNC_VALUES },
        parallel_requests: 1,
        max_response_size: bytesize::ByteSize::mib(1),
        max_concurrent_inbound_requests: 4,
        max_pending_inbound_requests: 8,
        max_inbound_requests_per_window: 64,
        ..Default::default()
    };
    Ok(Config {
        moniker: format!("yamichain-local-{index}"),
        consensus,
        sync,
    })
}
fn params(genesis: &Genesis, height: u64, timing: Duration) -> Result<m::HeightParams<Context>> {
    Ok(m::HeightParams::new(
        Committee::at(genesis, height)?,
        m::LinearTimeouts {
            propose: timing,
            prevote: timing / 2,
            precommit: timing / 2,
            rebroadcast: timing * 2,
            propose_delta: if native_devnet() {
                Duration::ZERO
            } else {
                Duration::from_millis(500)
            },
            prevote_delta: if native_devnet() {
                Duration::ZERO
            } else {
                Duration::from_millis(500)
            },
            precommit_delta: if native_devnet() {
                Duration::ZERO
            } else {
                Duration::from_millis(500)
            },
            ..Default::default()
        },
        Some(Duration::from_millis(400)),
    ))
}

/// Defaults are loopback-only; the explicit isolated profile accepts private IPv4 labs.
pub struct NodeOptions {
    pub directory: PathBuf,
    pub isolated_network: Option<crate::network::IsolatedNetwork>,
    pub genesis: Genesis,
    pub index: u8,
    pub base_port: u16,
    pub peer_base_port: u16,
    pub test_transactions: usize,
    pub test_distinct_accounts: bool,
    pub sparse_peers: bool,
    pub start_at: Option<std::time::SystemTime>,
    pub round_timeout: Duration,
    pub target_height: u64,
    pub timeout: Duration,
    pub linger: Duration,
    pub crash: Option<CrashPoint>,
}
pub fn inspect(directory: &Path) -> Result<State> {
    inspect_with_genesis(directory, Genesis::local())
}
pub fn inspect_with_genesis(directory: &Path, genesis: Genesis) -> Result<State> {
    if !directory.join("genesis.bin").is_file() {
        return Err(eyre!("no initialized local chain store"));
    }
    let journal = Journal::open(directory)?;
    crate::replay_certified(&journal, &genesis)
}
#[derive(Clone, Copy, Debug, Default)]
pub struct StorageOptions {
    pub checkpoint_interval: Option<u64>,
    pub storage_limits: yami_chain_storage::StorageLimits,
    pub minimum_archive_anchor: Option<yami_chain_storage::ArchiveAnchor>,
    pub minimum_signing_anchor: Option<yami_chain_storage::SigningAnchor>,
}
pub async fn run_node(options: NodeOptions) -> Result<State> {
    run_node_with_storage(options, StorageOptions::default()).await
}
#[allow(unused_mut)]
pub async fn run_node_with_storage(
    mut options: NodeOptions,
    storage: StorageOptions,
) -> Result<State> {
    #[cfg(feature = "insecure-devnet-input")]
    if let Some(path) = std::env::var_os("YAMI_NATIVE_POLICY") {
        if let Some(scope) = yami_chain::native_scope(&std::fs::read(path)?)? {
            options.genesis.enable_live_committees(scope)?;
        }
    }
    let start_deadline = startup_deadline(
        options.start_at,
        std::time::SystemTime::now(),
        tokio::time::Instant::now(),
    )?;
    options.genesis.validate()?;
    if let Some(n) = &options.isolated_network {
        if n.genesis != options.genesis || n.index != options.index {
            return Err(eyre!("isolated identity/genesis/index mismatch"));
        }
    }
    let development_keys: std::collections::BTreeMap<_, _> = (0
        ..yami_chain::committee::MAX_VALIDATORS as u8)
        .map(|i| {
            (
                development_key("consensus", i).verifying_key().to_bytes(),
                development_key("transport", i).verifying_key().to_bytes(),
            )
        })
        .collect();
    if options.isolated_network.is_none()
        && options
            .genesis
            .epochs
            .iter()
            .flat_map(|e| e.validators.members())
            .any(|m| development_keys.get(&m.consensus_key) != Some(&m.transport_key))
    {
        return Err(eyre!(
            "local runner accepts only separate public development consensus/transport identities"
        ));
    }
    if options.index >= yami_chain::committee::MAX_VALIDATORS as u8
        || (options.isolated_network.is_none()
            && !native_devnet()
            && !options.genesis.has_consensus_key(
                &development_key("consensus", options.index)
                    .verifying_key()
                    .to_bytes(),
            ))
        || options.timeout < Duration::from_secs(1)
        || options.timeout > Duration::from_secs(3600)
        || options.linger > Duration::from_secs(60)
        || options.round_timeout < Duration::from_millis(800)
        || options.round_timeout > Duration::from_secs(60)
        || options.test_transactions > yami_chain::MAX_TXS
        || options.peer_base_port < 1024
        || options.peer_base_port > 65436
        || options.base_port < 1024
        || options.base_port > 65436
        || options.target_height == 0
        || (options.target_height > 100_000 && !native_devnet())
    {
        return Err(eyre!(
            "invalid local options: scheduled identity, ports 1024..65436, height 1..100000, timeout 1..3600s, linger 0..60s, round timeout 800..60000ms"
        ));
    }
    if storage
        .checkpoint_interval
        .is_some_and(|n| n == 0 || n > 100_000)
    {
        return Err(eyre!("checkpoint interval must be 1..100000"));
    }
    let journal = Arc::new(Journal::open_with_limits(
        &options.directory,
        yami_chain_storage::DEFAULT_ENTRY_BUDGET,
        storage.storage_limits,
    )?);
    if let Some(n) = &options.isolated_network {
        n.checkpoint(&journal)?;
    }
    if journal.is_segmented_archive() && journal.archive_stats()?.is_none() {
        return Err(eyre!(
            "archive migration incomplete; resume migrate-archive before running"
        ));
    }
    journal.bind_validator_index(options.index)?;
    if let Some(anchor) = storage.minimum_archive_anchor {
        journal.verify_archive_anchor(anchor)?;
    }
    if let Some(anchor) = storage.minimum_signing_anchor {
        journal.verify_signing_anchor(anchor)?;
    }
    if storage.checkpoint_interval.is_none() && journal.path().join("compact-base.bin").exists() {
        return Err(eyre!(
            "archive mode cannot reopen a pruned store; select pruned mode"
        ));
    }
    if native_devnet() && storage.checkpoint_interval.is_some() {
        return Err(eyre!("native devnet requires complete archive history"));
    }
    let state = crate::replay_certified(&journal, &options.genesis)?;
    #[cfg(feature = "insecure-devnet-input")]
    if native_devnet() {
        for height in 1..=state.height() {
            publish_native_record(
                &journal,
                &journal
                    .record(height)?
                    .ok_or_else(|| eyre!("missing native history"))?,
            )?;
        }
    }
    if state.height() >= options.target_height {
        return Ok(state);
    }
    // Existing idempotent records may be read at pressure, but do not start
    // network actors or potentially large WAL recovery before refusing new work.
    journal.ensure_capacity(0)?;
    if !options.directory.join("wal/consensus.wal").exists()
        && journal.has_signing_reservations()?
    {
        return Err(eyre!(
            "signing history exists but consensus WAL is missing; refusing restart"
        ));
    }
    journal.migrate_signatures()?;
    let key = options
        .isolated_network
        .as_ref()
        .map(|n| n.consensus.clone())
        .map(Ok)
        .unwrap_or_else(|| node_key("consensus", options.index))?;
    #[cfg(feature = "insecure-devnet-input")]
    if native_devnet() {
        let mut identity = key.verifying_key().to_bytes().to_vec();
        identity.extend_from_slice(
            &node_key("transport", options.index)?
                .verifying_key()
                .to_bytes(),
        );
        journal.write_once("native-validator-identity.bin", &identity)?;
    }
    let address = Address(Digest(key.verifying_key().to_bytes()));
    let signer = Arc::new(DurableSigner {
        genesis: options.genesis.clone(),
        key,
        journal: journal.clone(),
        crash: options.crash,
    });
    let keypair = if let Some(n) = &options.isolated_network {
        if n.genesis != options.genesis {
            return Err(eyre!("isolated genesis mismatch"));
        }
        Keypair::ed25519_from_bytes(n.transport.to_bytes())?
    } else {
        transport(options.index)?
    };
    use malachite_signing::Signer as _;
    let proof = signer
        .sign_validator_proof(
            address.0 .0.to_vec(),
            keypair.public().to_peer_id().to_bytes(),
        )
        .await?;
    let identity = NetworkIdentity::new_validator(
        format!("yamichain-local-{}", options.index),
        keypair,
        address.to_string(),
        WireCodec::new(&options.genesis).encode(&proof)?,
    );
    std::fs::create_dir_all(options.directory.join("wal"))?;
    let mut network_config = config(
        options.index,
        options.base_port,
        options.peer_base_port,
        &options.genesis,
        options.sparse_peers,
    )?;
    if let Some(n) = &options.isolated_network {
        network_config.consensus.p2p.listen_addr =
            format!("/ip4/{}/tcp/{}", n.listen.ip(), n.listen.port()).parse()?;
        network_config.consensus.p2p.persistent_peers = n
            .peers
            .iter()
            .map(|(a, key)| {
                let peer = libp2p_identity::PublicKey::from(
                    libp2p_identity::ed25519::PublicKey::try_from_bytes(key)?,
                )
                .to_peer_id();
                Ok(format!("/ip4/{}/tcp/{}/p2p/{peer}", a.ip(), a.port()).parse()?)
            })
            .collect::<Result<_>>()?;
        let d = &mut network_config.consensus.p2p.discovery;
        d.num_inbound_peers = 8;
        d.num_outbound_peers = 25;
        d.max_connections_per_peer = 1;
        d.max_connections_per_ip = 4;
        d.ip_throttle_duration = Duration::from_secs(2);
        d.ephemeral_connection_timeout = Duration::from_secs(5);
        d.max_peers_per_response = 16;
    }
    let (mut channels, handle) = EngineBuilder::new(Context, network_config)
        .with_default_wal(WalContext::new(
            options.directory.join("wal/consensus.wal"),
            WireCodec::new(&options.genesis),
        ))
        .with_default_network(NetworkContext::new(
            identity,
            WireCodec::new(&options.genesis),
        ))
        .with_default_consensus(ConsensusContext::new_validator(
            address,
            Box::new(SignatureVerifier(options.genesis.clone())),
            Box::new((*signer).clone()),
        ))
        .with_default_sync(SyncContext::new(WireCodec::new(&options.genesis)))
        .with_default_request(RequestContext::new(16))
        .build()
        .await?;
    // A monotonic deadline prevents wall-clock changes from extending this wait.
    if let Some(deadline) = start_deadline {
        tokio::time::sleep_until(deadline).await;
    }
    let result = tokio::select! {
        result=app_loop(state,journal.clone(),&mut channels,&options,&storage,signer.clone())=>result,
        signal=tokio::signal::ctrl_c()=>{signal.map_err(eyre::Report::from).and_then(|()|crate::replay_certified(&journal,&options.genesis))},
    };
    handle.actor.stop(None);
    let _ = tokio::time::timeout(Duration::from_secs(5), handle.handle).await;
    result
}

async fn apply_peer_update(
    current: &crate::network::IsolatedNetwork,
    updated: &crate::network::IsolatedNetwork,
    channels: &Channels<Context>,
    journal: &Journal,
) -> Result<()> {
    if updated.genesis != current.genesis || updated.listen != current.listen {
        return Err(eyre!(
            "discovery cannot change pinned genesis or active listener"
        ));
    }
    // A stale/invalid update does not replace the last verified set.
    updated.checkpoint(journal)?;
    // Removing an endpoint clears upstream identity-level persistence, so remove first.
    for peer in current.peers.iter().filter(|p| !updated.peers.contains(p)) {
        tokio::time::timeout(
            Duration::from_secs(1),
            malachite::NetworkRequest::remove_persistent_peer(
                &channels.net_requests,
                peer_multiaddr(peer)?.parse()?,
            ),
        )
        .await???;
    }
    for peer in updated.peers.iter().filter(|p| !current.peers.contains(p)) {
        tokio::time::timeout(
            Duration::from_secs(1),
            malachite::NetworkRequest::add_persistent_peer(
                &channels.net_requests,
                peer_multiaddr(peer)?.parse()?,
            ),
        )
        .await???;
    }
    Ok(())
}

fn peer_multiaddr(peer: &(std::net::SocketAddrV4, [u8; 32])) -> Result<String> {
    let (a, key) = peer;
    let id =
        libp2p_identity::PublicKey::from(libp2p_identity::ed25519::PublicKey::try_from_bytes(key)?)
            .to_peer_id();
    Ok(format!("/ip4/{}/tcp/{}/p2p/{id}", a.ip(), a.port()))
}

async fn publish(
    channels: &Channels<Context>,
    signer: &DurableSigner,
    mut part: Part,
) -> Result<()> {
    use malachite_signing::Signer as _;
    part.signature = signer.sign_proposal(part.proposal()?).await?.signature;
    publish_signed(channels, part).await
}
// Only locally signed or independently authenticated cached parts reach here.
async fn publish_signed(channels: &Channels<Context>, part: Part) -> Result<()> {
    let id = part.block.id()?;
    channels
        .network
        .send(NetworkMsg::PublishProposalPart(StreamMessage::new(
            StreamId::new(Bytes::copy_from_slice(&id.0)),
            0,
            StreamContent::Data(part),
        )))
        .await?;
    Ok(())
}
fn proposal(part: &Part) -> Result<ProposedValue<Context>> {
    Ok(ProposedValue {
        height: part.height,
        round: part.round,
        valid_round: part.valid_round,
        proposer: part.proposer,
        value: Value(part.block.id()?),
        validity: m::Validity::Valid,
    })
}
async fn app_loop(
    mut state: State,
    journal: Arc<Journal>,
    channels: &mut Channels<Context>,
    options: &NodeOptions,
    storage: &StorageOptions,
    signer: Arc<DurableSigner>,
) -> Result<State> {
    let mut candidates = Candidates::new(journal.clone());
    let mut history_min = crate::history_min_height(&journal, state.genesis())?;
    let mut storage_tick = tokio::time::interval(if native_devnet() {
        Duration::from_millis(100)
    } else {
        Duration::from_secs(2)
    });
    let mut active_round = m::Round::Nil;
    // One bounded observation, only for the explicit distinct-account lab workload.
    let mut proposal_observation: Option<(Digest, std::time::Instant, bool)> = None;
    let consensus_deadline = tokio::time::Instant::now() + options.timeout;
    let mut serve_deadline = None;
    #[cfg(feature = "insecure-devnet-input")]
    let mut pending_value: Option<AppMsg<Context>> = None;
    #[cfg(feature = "insecure-devnet-input")]
    let mut pending_deadline = tokio::time::Instant::now();
    let mut _held_final_reply = None;
    let mut discovery_tick = tokio::time::interval(Duration::from_secs(2));
    discovery_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut network_view = options.isolated_network.clone();
    let discovery_socket = if let Some(n) = &network_view {
        Some(
            tokio::net::UdpSocket::bind(std::net::SocketAddrV4::new(
                *n.listen.ip(),
                n.listen.port() + 1,
            ))
            .await?,
        )
    } else {
        None
    };
    let mut packet_budget = 4;
    let mut receive_budget = 64;
    let mut packet = [0u8; 1762];
    let mut page_cursor = 0usize;
    loop {
        let deadline = serve_deadline.unwrap_or_else(|| {
            if native_devnet() {
                tokio::time::Instant::now() + options.timeout
            } else {
                consensus_deadline
            }
        });
        let received = tokio::select! {
            result = tokio::time::timeout_at(deadline, channels.consensus.recv()) => result,
            _ = storage_tick.tick() => {
                journal.ensure_capacity(0)?;
                #[cfg(feature="insecure-devnet-input")]
                if matches!(&pending_value, Some(AppMsg::GetValue {height,..}) if options.directory.join(format!("devnet-payment-{}.json",height.0)).exists()) {
                    Ok(pending_value.take())
                } else {
                    // The channel connector serializes host RPCs. Holding GetValue
                    // indefinitely also blocks history/status and therefore catch-up.
                    // Cancel only this unavailable proposal within its time budget;
                    // never invent an empty or unexecuted value to unblock the host.
                    if tokio::time::Instant::now() >= pending_deadline { pending_value = None; }
                    continue;
                }
                #[cfg(not(feature="insecure-devnet-input"))]
                continue;
            },
            _ = discovery_tick.tick(), if network_view.is_some() => {
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
                let n = network_view.as_ref().expect("isolated profile");
                if let Ok(updated) = n.refresh(None, now) {
                    if updated.checkpoint(&journal).is_ok() {
                        apply_peer_update(n, &updated, channels, &journal).await?;
                        network_view = Some(updated);
                    }
                }
                let n = network_view.as_ref().expect("isolated profile");
                if n.hints.values().any(|h| now >= h.0) { return Err(eyre!("authenticated discovery hints expired; stopping isolated listener")); }
                let page = n.page(page_cursor); page_cursor = page_cursor.wrapping_add(16);
                for (address, _) in &n.peers {
                    let target = std::net::SocketAddrV4::new(*address.ip(), address.port()+1);
                    let _ = discovery_socket.as_ref().expect("isolated socket").send_to(&page, target).await;
                }
                packet_budget = 4;
                receive_budget = 64;
                continue;
            }
            result = async {
                match &discovery_socket { Some(socket) => socket.recv_from(&mut packet).await, None => std::future::pending().await }
            }, if network_view.is_some() && receive_budget > 0 => {
                // Drain a bounded number cheaply, but verify only four pages.
                // Otherwise a finite forged burst can occupy the kernel queue
                // for minutes and delay authenticated endpoint replacement.
                receive_budget -= 1;
                if let Ok((len, source)) = result {
                    let n = network_view.as_ref().expect("isolated profile");
                    // Bootstrap IPs are only an inexpensive ingress filter. Every
                    // supplied endpoint still needs a pinned identity's signature.
                    if packet_budget > 0 && n.peers.iter().any(|(a, _)| std::net::IpAddr::V4(*a.ip()) == source.ip()) {
                        packet_budget -= 1;
                        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
                        if let Ok(updated) = n.refresh(Some(&packet[..len]), now) {
                            if updated.checkpoint(&journal).is_ok() {
                                apply_peer_update(n, &updated, channels, &journal).await?;
                                network_view = Some(updated);
                            }
                        }
                    }
                }
                continue;
            }
        };
        let msg = match received {
            Ok(msg) => msg,
            Err(_) if state.height() >= options.target_height => return Ok(state),
            Err(_) => {
                return Err(eyre!(
                    "local consensus deadline reached; quorum/liveness not established"
                ))
            }
        };
        let Some(msg) = msg else { break };
        #[cfg(feature = "insecure-devnet-input")]
        if native_devnet()
            && matches!(&msg, AppMsg::GetValue {height,..} if !options.directory.join(format!("devnet-payment-{}.json",height.0)).exists())
        {
            if let AppMsg::GetValue { timeout, .. } = &msg {
                pending_deadline =
                    tokio::time::Instant::now() + (*timeout).min(Duration::from_millis(100));
            }
            pending_value = Some(msg);
            continue;
        }
        match msg {
            AppMsg::ConsensusReady { reply } => {
                let _ = reply.send((
                    Height(state.height() + 1),
                    params(state.genesis(), state.height() + 1, options.round_timeout)?,
                ));
            }
            AppMsg::StartedRound {
                height,
                round,
                reply_value,
                ..
            } => {
                if height.0 != state.height() + 1 {
                    return Err(eyre!("height/membership mismatch"));
                }
                active_round = round;
                #[cfg(feature = "insecure-devnet-input")]
                {
                    pending_value = None;
                }
                // Recover the exact authenticated metadata for this round, never
                // relabel arbitrary cached blocks as a fresh proposal.
                let proposals = candidates
                    .recover(&state, round)?
                    .into_iter()
                    .map(|part| proposal(&part))
                    .collect::<Result<Vec<_>>>()?;
                let _ = reply_value.send(proposals);
            }
            AppMsg::GetValue {
                height,
                round,
                reply,
                ..
            } => {
                if height.0 != state.height() + 1 {
                    return Err(eyre!("proposal height mismatch"));
                }
                let proposal_began = options.test_distinct_accounts.then(std::time::Instant::now);
                let legacy_block = || {
                    if options.test_distinct_accounts {
                        state.development_distinct_workload_block(options.test_transactions)
                    } else {
                        state.development_workload_block(options.test_transactions)
                    }
                };
                #[cfg(not(feature = "insecure-devnet-input"))]
                let block = legacy_block()?;
                #[cfg(feature = "insecure-devnet-input")]
                let block = if native_devnet() {
                    let payload = yami_chain_storage::read_bounded(
                        &options
                            .directory
                            .join(format!("devnet-payment-{}.json", height.0)),
                        30000,
                    )?;
                    let owner = yami_chain::development_key("registry", 0);
                    let tx = yami_chain::Transaction::devnet_certificate(
                        payload,
                        state.nonce(&owner.verifying_key().to_bytes()),
                        height.0,
                        &owner,
                    )?;
                    state.propose(vec![tx])?
                } else {
                    legacy_block()?
                };
                if let Some(at) = proposal_began {
                    proposal_observation = Some((block.id()?, at, true));
                }
                candidates.store_block(&block)?;
                let proposer = Context
                    .select_proposer(&Committee::at(state.genesis(), height.0)?, height, round)
                    .address;
                let part = Part {
                    height,
                    round,
                    valid_round: m::Round::Nil,
                    proposer,
                    block,
                    signature: [0; 64],
                };
                let _ = reply.send(LocallyProposedValue::new(
                    height,
                    round,
                    Value(part.block.id()?),
                ));
                publish(channels, &signer, part).await?;
            }
            AppMsg::ReceivedProposalPart { part, reply, .. } => {
                let mut result = None;
                if let Some(part) = single_part_proposal(part) {
                    if candidates.admit(&state, active_round, &part)? {
                        if options.test_distinct_accounts {
                            let id = part.block.id()?;
                            if proposal_observation
                                .as_ref()
                                .is_none_or(|(previous, _, _)| *previous != id)
                            {
                                proposal_observation = Some((id, std::time::Instant::now(), false));
                            }
                        }
                        result = Some(proposal(&part)?);
                    }
                }
                let _ = reply.send(result);
            }
            AppMsg::RestreamProposal {
                height,
                round,
                valid_round,
                address,
                value_id,
            } => {
                if height.0 != state.height() + 1 {
                    continue;
                }
                if address.0 .0 != signer.key.verifying_key().to_bytes() {
                    if let Some(part) = candidates.restream(
                        &state,
                        height,
                        round,
                        valid_round,
                        address,
                        value_id,
                    )? {
                        publish_signed(channels, part).await?;
                    }
                    continue;
                }
                if let Some(block) = candidates.block(height.0, value_id.0)? {
                    publish(
                        channels,
                        &signer,
                        Part {
                            height,
                            round,
                            valid_round,
                            proposer: address,
                            block,
                            signature: [0; 64],
                        },
                    )
                    .await?;
                }
            }
            AppMsg::ExtendVote { reply, .. } => {
                let _ = reply.send(None);
            }
            AppMsg::VerifyVoteExtensions {
                extensions, reply, ..
            } => {
                let _ = reply.send(
                    extensions
                        .into_iter()
                        .map(|(address, _)| {
                            (
                                address,
                                Err(malachite_core::VoteExtensionError::InvalidVoteExtension),
                            )
                        })
                        .collect(),
                );
            }
            AppMsg::Decided {
                certificate,
                extensions,
                reply,
            } => {
                if !extensions.extensions.is_empty() {
                    return Err(eyre!("unexpected vote extensions"));
                }
                let block = candidates
                    .block(certificate.height.0, certificate.value_id.0)?
                    .ok_or_else(|| eyre!("decided payload unavailable"))?;
                verify_certificate(state.genesis(), &block, &certificate)?;
                let next = candidates.execute(&state, &block)?;
                maybe_crash(&journal, options.crash, CrashPoint::BeforeCommit)?;
                let record = FinalizedRecord {
                    block,
                    certificate: encode_certificate(state.genesis(), &certificate)?,
                };
                journal.append(&record)?;
                #[cfg(feature = "insecure-devnet-input")]
                publish_native_record(&journal, &record)?;
                state = next;
                #[cfg(feature = "insecure-devnet-input")]
                state.install_finalized_committee()?;
                let _ = reply.send(());
            }
            AppMsg::Finalized {
                certificate, reply, ..
            } => {
                let record = journal
                    .record(certificate.height.0)?
                    .ok_or_else(|| eyre!("decision not durable"))?;
                verify_certificate(state.genesis(), &record.block, &certificate)?;
                if state.height() != certificate.height.0 || state.tip() != certificate.value_id.0 {
                    return Err(eyre!("finalized state mismatch"));
                }
                if state
                    .genesis()
                    .epochs
                    .iter()
                    .any(|e| e.first_height == state.height() + 1)
                {
                    maybe_crash(&journal, options.crash, CrashPoint::BeforeEpoch)?;
                }
                journal.check_cached_capacity()?;
                if let Some(floor) = storage.checkpoint_interval.and_then(|n| {
                    crate::compact::periodic_prune_floor(state.height(), n, history_min)
                }) {
                    let compacted =
                        crate::prune_certified_before(&journal, state.genesis(), floor)?;
                    if compacted != state {
                        return Err(eyre!("compaction changed finalized state"));
                    }
                    history_min = crate::history_min_height(&journal, state.genesis())?;
                }
                if options.test_distinct_accounts {
                    if let Some((id, at, local)) = proposal_observation.take() {
                        if id == certificate.value_id.0 {
                            eprintln!("{{\"lab_finality\":true,\"height\":{},\"proposal_to_finalized_seconds\":{},\"origin\":\"{}\"}}",state.height(),at.elapsed().as_secs_f64(),if local { "local_builder_start" } else { "authenticated_admission" });
                        }
                    }
                }
                println!("{}", status_json(&state));
                if state.height() >= options.target_height {
                    if options.linger.is_zero() {
                        return Ok(state);
                    }
                    // Keep serving certified history at the terminal demo height.
                    // Hold the finalization reply so the engine does not start a
                    // new height while lagging participants fetch the last record.
                    serve_deadline = Some(tokio::time::Instant::now() + options.linger);
                    _held_final_reply = Some(reply);
                    continue;
                }
                candidates.prune_finalized(state.height())?;
                active_round = m::Round::Nil;
                let _ = reply.send(Next::Start(
                    Height(state.height() + 1),
                    params(state.genesis(), state.height() + 1, options.round_timeout)?,
                ));
            }
            AppMsg::GetHistoryMinHeight { reply } => {
                let _ = reply.send(Height(history_min));
            }
            AppMsg::GetDecidedValues { range, reply } => {
                let mut values = Vec::new();
                let start = range.start().0;
                let end = range.end().0;
                if start > 0 && end >= start && end - start < MAX_SYNC_VALUES as u64 {
                    for height in start..=end.min(state.height()) {
                        let Some(record) = journal.record(height)? else {
                            break;
                        };
                        verify_record(state.genesis(), &record)?;
                        let certificate =
                            m::ExtendedCommitCertificate::from_commit_certificate_and_extensions(
                                decode_certificate(state.genesis(), &record.certificate)?,
                                m::VoteExtensions::default(),
                            );
                        values.push(RawDecidedValue::new(
                            Bytes::from(record.block.canonical()?),
                            certificate,
                        ));
                    }
                }
                if !values.is_empty() {
                    eprintln!(
                        "served certified history from {}: {} values",
                        start,
                        values.len()
                    );
                }
                let _ = reply.send(values);
            }
            AppMsg::ProcessSyncedValue {
                height,
                round,
                proposer,
                value_bytes,
                reply,
            } => {
                let outcome = match Block::decode(&value_bytes) {
                    Ok(block)
                        if height.0 == state.height() + 1
                            && block.header.height == height.0
                            && state.execute(&block).is_ok() =>
                    {
                        eprintln!("validated synced payload at height {}", height.0);
                        let part = Part {
                            height,
                            round,
                            valid_round: m::Round::Nil,
                            proposer,
                            block,
                            signature: [0; 64],
                        };
                        candidates.store_block(&part.block)?;
                        SyncedValueOutcome::Verdict(proposal(&part)?)
                    }
                    _ => SyncedValueOutcome::PeerFault,
                };
                let _ = reply.send(outcome);
            }
        }
    }
    Err(eyre!("Malachite channels closed before requested height"))
}
pub fn status_json(state: &State) -> String {
    format!("{{\"chain_id\":\"{}\",\"height\":{},\"block\":\"{}\",\"state_root\":\"{}\",\"test_tlds\":{},\"payments_enabled\":false,\"production_ready\":false}}",yami_chain::CHAIN_ID,state.height(),state.tip(),state.root(),state.names().len())
}

// Our transport publishes exactly one data part, keyed by its block ID. Reject
// misleading stream metadata before any candidate disk write or app admission.
fn single_part_proposal(stream: StreamMessage<Part>) -> Option<Part> {
    if stream.sequence != 0 {
        return None;
    }
    let StreamContent::Data(part) = stream.content else {
        return None;
    };
    let id = part.block.id().ok()?;
    (stream.stream_id.to_bytes().as_ref() == id.0.as_slice()).then_some(part)
}

#[cfg(test)]
mod stream_tests {
    use super::*;
    #[test]
    fn invalid_peer_streams_cannot_relabel_or_resequence_proposal_payloads() {
        let state = State::new(Genesis::local()).unwrap();
        let part = Part {
            height: Height(1),
            round: m::Round::ZERO,
            valid_round: m::Round::Nil,
            proposer: Context
                .select_proposer(&Committee::local(), Height(1), m::Round::ZERO)
                .address,
            block: state.development_block().unwrap(),
            signature: [0; 64],
        };
        let id = part.block.id().unwrap();
        let stream = |id: &[u8], sequence| {
            StreamMessage::new(
                StreamId::new(Bytes::copy_from_slice(id)),
                sequence,
                StreamContent::Data(part.clone()),
            )
        };
        assert_eq!(single_part_proposal(stream(&id.0, 0)), Some(part.clone()));
        for wrong in [vec![], vec![0; 31], vec![0; 32], vec![0; 33]] {
            assert!(single_part_proposal(stream(&wrong, 0)).is_none());
        }
        for sequence in [1, 2, u64::MAX] {
            assert!(single_part_proposal(stream(&id.0, sequence)).is_none());
        }
        assert!(single_part_proposal(StreamMessage::new(
            StreamId::new(Bytes::copy_from_slice(&id.0)),
            0,
            StreamContent::Fin
        ))
        .is_none());
    }
}

// A local rehearsal topology, not validator selection. All scheduled identities
// remain authenticated by the pinned genesis and all consensus rules are unchanged.
fn sparse_neighbors(scheduled: &[u8], index: u8) -> Vec<u8> {
    let Some(position) = scheduled.iter().position(|i| *i == index) else {
        return vec![];
    };
    let n = scheduled.len();
    let mut stride = 1;
    while (stride + 1) * (stride + 1) <= n {
        stride += 1;
    }
    let mut peers = std::collections::BTreeSet::new();
    for distance in [1, stride] {
        for target in [(position + distance) % n, (position + n - distance) % n] {
            if target != position {
                peers.insert(scheduled[target]);
            }
        }
    }
    peers.into_iter().collect()
}
#[cfg(test)]
mod topology_tests {
    use super::*;
    #[test]
    fn sparse_graph_is_symmetric_bounded_and_survives_one_crash() {
        for n in [4, 25, 50, 100] {
            let members: Vec<_> = (0..n).collect();
            for missing in 0..n {
                let start = if missing == 0 { 1 } else { 0 };
                let mut seen = std::collections::BTreeSet::from([start]);
                let mut todo = vec![start];
                while let Some(i) = todo.pop() {
                    let peers = sparse_neighbors(&members, i);
                    assert!(peers.len() <= 4);
                    for j in peers {
                        assert!(sparse_neighbors(&members, j).contains(&i));
                        if j != missing && seen.insert(j) {
                            todo.push(j);
                        }
                    }
                }
                assert_eq!(seen.len(), usize::from(n - 1));
            }
        }
    }
}

fn startup_deadline(
    start: Option<std::time::SystemTime>,
    wall: std::time::SystemTime,
    monotonic: tokio::time::Instant,
) -> Result<Option<tokio::time::Instant>> {
    let Some(delay) = start.and_then(|s| s.duration_since(wall).ok()) else {
        return Ok(None);
    };
    if delay > Duration::from_secs(60) {
        return Err(eyre!("startup barrier exceeds 60 seconds"));
    }
    Ok(Some(monotonic + delay))
}
#[cfg(test)]
mod startup_tests {
    use super::*;
    #[tokio::test]
    async fn invalid_startup_creates_no_store_or_network_resources() {
        let directory =
            std::env::temp_dir().join(format!("yami-invalid-startup-{}", std::process::id()));
        for (barrier, timeout) in [
            (
                Some(std::time::SystemTime::now() + Duration::from_secs(120)),
                Duration::from_secs(60),
            ),
            (None, Duration::from_millis(500)),
        ] {
            let options = NodeOptions {
                directory: directory.clone(),
                genesis: Genesis::local(),
                index: 0,
                base_port: 28100,
                peer_base_port: 28100,
                isolated_network: None,
                test_transactions: 0,
                test_distinct_accounts: false,
                sparse_peers: false,
                start_at: barrier,
                round_timeout: Duration::from_millis(800),
                target_height: 1,
                timeout,
                linger: Duration::ZERO,
                crash: None,
            };
            assert!(run_node(options).await.is_err());
            assert!(!directory.exists());
        }
    }

    #[tokio::test]
    async fn restart_pressure_refuses_before_wal_recovery_and_retains_safety() {
        let directory =
            std::env::temp_dir().join(format!("yami-pressure-startup-{}", std::process::id()));
        let genesis = Genesis::development_schedule(&[(1, 4)]).unwrap();
        let journal = Journal::open(&directory).unwrap();
        journal.replay(genesis.clone(), |_| Ok(())).unwrap();
        journal.write_once("validator-index.bin", &[0]).unwrap();
        journal
            .reserve_signature(1, 0, 1, b"retained preimage")
            .unwrap();
        std::fs::create_dir(directory.join("wal")).unwrap();
        let wal = directory.join("wal/consensus.wal");
        std::fs::write(&wal, b"malformed WAL must not be recovered at pressure").unwrap();
        drop(journal);
        for _ in 0..3 {
            let options = NodeOptions {
                directory: directory.clone(),
                genesis: genesis.clone(),
                index: 0,
                base_port: 28100,
                peer_base_port: 28100,
                isolated_network: None,
                test_transactions: 0,
                test_distinct_accounts: false,
                sparse_peers: false,
                start_at: None,
                round_timeout: Duration::from_millis(800),
                target_height: 1,
                timeout: Duration::from_secs(1),
                linger: Duration::ZERO,
                crash: None,
            };
            let error = run_node_with_storage(
                options,
                StorageOptions {
                    checkpoint_interval: None,
                    minimum_signing_anchor: None,
                    minimum_archive_anchor: None,
                    storage_limits: yami_chain_storage::StorageLimits {
                        max_bytes: 1,
                        reserve_bytes: 0,
                    },
                },
            )
            .await
            .unwrap_err();
            assert!(
                error.to_string().contains("quota/reserve exhausted"),
                "{error}"
            );
            assert_eq!(
                std::fs::read(&wal).unwrap(),
                b"malformed WAL must not be recovered at pressure"
            );
            let journal = Journal::open(&directory).unwrap();
            journal
                .reserve_signature(1, 0, 1, b"retained preimage")
                .unwrap();
            assert!(journal.reserve_signature(1, 0, 1, b"conflict").is_err());
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn independent_signing_anchor_is_checked_before_wal_or_new_work() {
        let directory = std::env::temp_dir().join(format!(
            "yami-signing-anchor-startup-{}",
            std::process::id()
        ));
        let genesis = Genesis::development_schedule(&[(1, 4)]).unwrap();
        let journal = Journal::open(&directory).unwrap();
        journal.bind_validator_index(0).unwrap();
        journal.replay(genesis.clone(), |_| Ok(())).unwrap();
        journal.reserve_signature(1, 0, 1, b"retained").unwrap();
        std::fs::create_dir(directory.join("wal")).unwrap();
        std::fs::write(directory.join("wal/consensus.wal"), b"unread WAL").unwrap();
        drop(journal);
        let options = NodeOptions {
            directory: directory.clone(),
            genesis,
            index: 0,
            base_port: 28100,
            peer_base_port: 28100,
            isolated_network: None,
            test_transactions: 0,
            test_distinct_accounts: false,
            sparse_peers: false,
            start_at: None,
            round_timeout: Duration::from_millis(800),
            target_height: 1,
            timeout: Duration::from_secs(1),
            linger: Duration::ZERO,
            crash: None,
        };
        let error = run_node_with_storage(
            options,
            StorageOptions {
                minimum_signing_anchor: Some(yami_chain_storage::SigningAnchor {
                    reservations: 2,
                    root: Digest([42; 32]),
                }),
                ..StorageOptions::default()
            },
        )
        .await
        .unwrap_err();
        assert!(
            error.to_string().contains("signing anchor rollback"),
            "{error}"
        );
        let journal = Journal::open(&directory).unwrap();
        assert_eq!(
            journal.signature_reservation(1, 0, 1).unwrap().unwrap(),
            b"retained"
        );
        assert_eq!(
            std::fs::read(directory.join("wal/consensus.wal")).unwrap(),
            b"unread WAL"
        );
        drop(journal);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn independent_archive_anchor_is_checked_before_wal_or_new_work() {
        let directory = std::env::temp_dir().join(format!(
            "yami-archive-anchor-startup-{}",
            std::process::id()
        ));
        let genesis = Genesis::development_schedule(&[(1, 4)]).unwrap();
        let journal = Journal::open(&directory).unwrap();
        journal.bind_validator_index(0).unwrap();
        journal.replay(genesis.clone(), |_| Ok(())).unwrap();
        journal.reserve_signature(1, 0, 1, b"retained").unwrap();
        journal
            .migrate_archive(genesis.clone(), |_| Ok(()))
            .unwrap();
        std::fs::create_dir(directory.join("wal")).unwrap();
        std::fs::write(directory.join("wal/consensus.wal"), b"unread WAL").unwrap();
        drop(journal);
        let options = NodeOptions {
            directory: directory.clone(),
            genesis,
            index: 0,
            base_port: 28100,
            peer_base_port: 28100,
            isolated_network: None,
            test_transactions: 0,
            test_distinct_accounts: false,
            sparse_peers: false,
            start_at: None,
            round_timeout: Duration::from_millis(800),
            target_height: 1,
            timeout: Duration::from_secs(1),
            linger: Duration::ZERO,
            crash: None,
        };
        let error = run_node_with_storage(
            options,
            StorageOptions {
                minimum_archive_anchor: Some(yami_chain_storage::ArchiveAnchor {
                    height: 2,
                    root: Digest([42; 32]),
                }),
                ..StorageOptions::default()
            },
        )
        .await
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("archive below independent anchor"),
            "{error}"
        );
        let journal = Journal::open(&directory).unwrap();
        assert_eq!(
            journal.signature_reservation(1, 0, 1).unwrap().unwrap(),
            b"retained"
        );
        assert_eq!(
            std::fs::read(directory.join("wal/consensus.wal")).unwrap(),
            b"unread WAL"
        );
        drop(journal);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn startup_uses_one_bounded_monotonic_deadline() {
        let wall = std::time::UNIX_EPOCH + Duration::from_secs(1000);
        let clock = tokio::time::Instant::now();
        assert!(startup_deadline(Some(wall + Duration::from_secs(61)), wall, clock).is_err());
        assert_eq!(
            startup_deadline(Some(wall + Duration::from_secs(60)), wall, clock).unwrap(),
            Some(clock + Duration::from_secs(60))
        );
        assert!(
            startup_deadline(Some(wall - Duration::from_secs(1)), wall, clock)
                .unwrap()
                .is_none()
        );
        assert!(startup_deadline(None, wall, clock).unwrap().is_none());
    }
}
