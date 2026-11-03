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
fn transport(index: u8) -> Result<Keypair> {
    Ok(Keypair::ed25519_from_bytes(
        development_key("transport", index).to_bytes(),
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
            let peer = transport(i)?.public().to_peer_id();
            Ok(format!("/ip4/127.0.0.1/tcp/{}/p2p/{peer}", peer_base + u16::from(i)).parse()?)
        })
        .collect::<Result<_>>()?;
    consensus.p2p.persistent_peers_only = true;
    consensus.p2p.discovery.enabled = false;
    consensus.p2p.pubsub_max_size = bytesize::ByteSize::kib(128);
    consensus.p2p.rpc_max_size = bytesize::ByteSize::mib(1);
    consensus.queue_capacity = 4;
    consensus.queue_per_height_capacity = 128;
    let sync = config::ValueSyncConfig {
        status_update_interval: Duration::from_millis(500),
        batch_size: MAX_SYNC_VALUES,
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
            ..Default::default()
        },
        Some(Duration::from_millis(400)),
    ))
}

/// All defaults are local-only. No configurable public listen address or secret key input.
pub struct NodeOptions {
    pub directory: PathBuf,
    pub genesis: Genesis,
    pub index: u8,
    pub base_port: u16,
    pub peer_base_port: u16,
    pub test_transactions: usize,
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
    Ok(journal.replay(genesis.clone(), |record| verify_record(&genesis, record))?)
}
pub async fn run_node(options: NodeOptions) -> Result<State> {
    let start_deadline = startup_deadline(
        options.start_at,
        std::time::SystemTime::now(),
        tokio::time::Instant::now(),
    )?;
    options.genesis.validate()?;
    let development_keys: std::collections::BTreeMap<_, _> = (0
        ..yami_chain::committee::MAX_VALIDATORS as u8)
        .map(|i| {
            (
                development_key("consensus", i).verifying_key().to_bytes(),
                development_key("transport", i).verifying_key().to_bytes(),
            )
        })
        .collect();
    if options
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
        || !options.genesis.has_consensus_key(
            &development_key("consensus", options.index)
                .verifying_key()
                .to_bytes(),
        )
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
        || options.target_height > 100_000
    {
        return Err(eyre!(
            "invalid local options: scheduled identity, ports 1024..65436, height 1..100000, timeout 1..3600s, linger 0..60s, round timeout 800..60000ms"
        ));
    }
    let journal = Arc::new(Journal::open(&options.directory)?);
    journal.write_once("validator-index.bin", &[options.index])?;
    let state = journal.replay(options.genesis.clone(), |record| {
        verify_record(&options.genesis, record)
    })?;
    if state.height() >= options.target_height {
        return Ok(state);
    }
    if !options.directory.join("wal/consensus.wal").exists()
        && std::fs::read_dir(&options.directory)?.any(|entry| {
            entry.is_ok_and(|e| e.file_name().to_string_lossy().starts_with("signed-"))
        })
    {
        return Err(eyre!(
            "signing history exists but consensus WAL is missing; refusing restart"
        ));
    }
    let key = development_key("consensus", options.index);
    let address = Address(Digest(key.verifying_key().to_bytes()));
    let signer = Arc::new(DurableSigner {
        genesis: options.genesis.clone(),
        key,
        journal: journal.clone(),
        crash: options.crash,
    });
    let keypair = transport(options.index)?;
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
    let (mut channels, handle) = EngineBuilder::new(
        Context,
        config(
            options.index,
            options.base_port,
            options.peer_base_port,
            &options.genesis,
            options.sparse_peers,
        )?,
    )
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
        result=app_loop(state,journal.clone(),&mut channels,&options,signer.clone())=>result,
        signal=tokio::signal::ctrl_c()=>{signal.map_err(eyre::Report::from).and_then(|()|journal.replay(options.genesis.clone(),|record| verify_record(&options.genesis,record)).map_err(Into::into))},
    };
    handle.actor.stop(None);
    let _ = tokio::time::timeout(Duration::from_secs(5), handle.handle).await;
    result
}

async fn publish(
    channels: &Channels<Context>,
    signer: &DurableSigner,
    mut part: Part,
) -> Result<()> {
    use malachite_signing::Signer as _;
    part.signature = signer.sign_proposal(part.proposal()?).await?.signature;
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
    signer: Arc<DurableSigner>,
) -> Result<State> {
    let mut candidates = Candidates::new(journal.clone());
    let mut active_round = m::Round::Nil;
    let consensus_deadline = tokio::time::Instant::now() + options.timeout;
    let mut serve_deadline = None;
    let mut _held_final_reply = None;
    loop {
        let deadline = serve_deadline.unwrap_or(consensus_deadline);
        let msg = match tokio::time::timeout_at(deadline, channels.consensus.recv()).await {
            Ok(msg) => msg,
            Err(_) if state.height() >= options.target_height => return Ok(state),
            Err(_) => {
                return Err(eyre!(
                    "local consensus deadline reached; quorum/liveness not established"
                ))
            }
        };
        let Some(msg) = msg else { break };
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
                let block = state.development_workload_block(options.test_transactions)?;
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
                let next = state.execute(&block)?;
                maybe_crash(&journal, options.crash, CrashPoint::BeforeCommit)?;
                journal.append(&FinalizedRecord {
                    block,
                    certificate: encode_certificate(state.genesis(), &certificate)?,
                })?;
                state = next;
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
                let _ = reply.send(Height(1));
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
    format!("{{\"chain_id\":\"{}\",\"height\":{},\"block\":\"{}\",\"state_root\":\"{}\",\"test_tlds\":{},\"payments_enabled\":false,\"production_ready\":false}}",yami_chain::CHAIN_ID,state.height(),state.tip(),state.root(),state.names().entries().count())
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
                test_transactions: 0,
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
