pub use crate::signing::CrashPoint;
use crate::{
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
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use yami_chain::{development_key, Block, Digest, Genesis, State, MAX_BLOCK_BYTES};
use yami_chain_storage::{read_bounded, FinalizedRecord, Journal};

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
fn config(index: u8, base: u16) -> Result<Config> {
    let mut consensus = config::ConsensusConfig::default();
    consensus.p2p.listen_addr =
        format!("/ip4/127.0.0.1/tcp/{}", base + u16::from(index)).parse()?;
    consensus.p2p.persistent_peers = (0..4_u8)
        .filter(|i| *i != index)
        .map(|i| {
            let peer = transport(i)?.public().to_peer_id();
            Ok(format!("/ip4/127.0.0.1/tcp/{}/p2p/{peer}", base + u16::from(i)).parse()?)
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
fn params() -> m::HeightParams<Context> {
    m::HeightParams::new(
        Committee::local(),
        m::LinearTimeouts {
            propose: Duration::from_millis(800),
            prevote: Duration::from_millis(400),
            precommit: Duration::from_millis(400),
            rebroadcast: Duration::from_millis(1600),
            ..Default::default()
        },
        Some(Duration::from_millis(400)),
    )
}

/// All defaults are local-only. No configurable public listen address or secret key input.
pub struct NodeOptions {
    pub directory: PathBuf,
    pub index: u8,
    pub base_port: u16,
    pub target_height: u64,
    pub timeout: Duration,
    pub crash: Option<CrashPoint>,
}
pub fn inspect(directory: &Path) -> Result<State> {
    if !directory.join("genesis.bin").is_file() {
        return Err(eyre!("no initialized local chain store"));
    }
    let journal = Journal::open(directory)?;
    Ok(journal.replay(Genesis::local(), verify_record)?)
}
pub async fn run_node(options: NodeOptions) -> Result<State> {
    if options.index >= 4
        || options.base_port < 1024
        || options.base_port > 65532
        || options.target_height == 0
        || options.target_height > 100_000
    {
        return Err(eyre!(
            "index 0..3, base port 1024..65532, target height 1..100000 required"
        ));
    }
    let journal = Arc::new(Journal::open(&options.directory)?);
    journal.write_once("validator-index.bin", &[options.index])?;
    let state = journal.replay(Genesis::local(), verify_record)?;
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
    let signer = DurableSigner {
        key,
        journal: journal.clone(),
        crash: options.crash,
    };
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
        WireCodec.encode(&proof)?,
    );
    std::fs::create_dir_all(options.directory.join("wal"))?;
    let (mut channels, handle) =
        EngineBuilder::new(Context, config(options.index, options.base_port)?)
            .with_default_wal(WalContext::new(
                options.directory.join("wal/consensus.wal"),
                WireCodec,
            ))
            .with_default_network(NetworkContext::new(identity, WireCodec))
            .with_default_consensus(ConsensusContext::new_validator(
                address,
                Box::new(SignatureVerifier),
                Box::new(signer),
            ))
            .with_default_sync(SyncContext::new(WireCodec))
            .with_default_request(RequestContext::new(16))
            .build()
            .await?;
    let result = tokio::select! {
        result=tokio::time::timeout(options.timeout,app_loop(state,journal.clone(),&mut channels,&options))=> {
            result.unwrap_or_else(|_|Err(eyre!("local consensus deadline reached; quorum/liveness not established")))
        },
        signal=tokio::signal::ctrl_c()=>{signal.map_err(eyre::Report::from).and_then(|()|journal.replay(Genesis::local(),verify_record).map_err(Into::into))},
    };
    handle.actor.stop(None);
    let _ = tokio::time::timeout(Duration::from_secs(5), handle.handle).await;
    result
}

fn pending_name(block: &Block) -> yami_chain::Result<String> {
    Ok(format!(
        "candidate-{:020}-{}.bin",
        block.header.height,
        block.id()?
    ))
}
fn remember(journal: &Journal, blocks: &mut BTreeMap<Digest, Block>, block: Block) -> Result<()> {
    let id = block.id()?;
    if !blocks.contains_key(&id) {
        if blocks.len() >= 64 {
            return Err(eyre!("candidate quota exhausted"));
        }
        journal.write_once(&pending_name(&block)?, &block.canonical()?)?;
        blocks.insert(id, block);
    }
    Ok(())
}
fn recover_candidates(journal: &Journal, state: &State) -> Result<BTreeMap<Digest, Block>> {
    let mut blocks = BTreeMap::new();
    let prefix = format!("candidate-{:020}-", state.height() + 1);
    for entry in std::fs::read_dir(journal.path())? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            if blocks.len() >= 64 {
                return Err(eyre!("candidate quota exhausted"));
            }
            let block = Block::decode(&read_bounded(&entry.path(), MAX_BLOCK_BYTES)?)?;
            state.execute(&block)?;
            if entry.file_name().to_string_lossy() != pending_name(&block)? {
                return Err(eyre!("candidate filename mismatch"));
            }
            blocks.insert(block.id()?, block);
        }
    }
    Ok(blocks)
}
async fn publish(channels: &Channels<Context>, part: Part) -> Result<()> {
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
) -> Result<State> {
    let mut blocks = recover_candidates(&journal, &state)?;
    while let Some(msg) = channels.consensus.recv().await {
        match msg {
            AppMsg::ConsensusReady { reply } => {
                let _ = reply.send((Height(state.height() + 1), params()));
            }
            AppMsg::StartedRound {
                height,
                round,
                proposer,
                reply_value,
                ..
            } => {
                if height.0 != state.height() + 1 {
                    return Err(eyre!("height/membership mismatch"));
                }
                // Return only previously built/received, executed and persisted
                // payloads. Fresh peers must receive and validate real block data.
                let proposals = blocks
                    .values()
                    .filter(|block| block.header.height == height.0)
                    .map(|block| {
                        proposal(&Part {
                            height,
                            round,
                            valid_round: m::Round::Nil,
                            proposer,
                            block: block.clone(),
                        })
                    })
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
                let block = state.development_block()?;
                remember(&journal, &mut blocks, block.clone())?;
                let proposer = Context
                    .select_proposer(&Committee::local(), height, round)
                    .address;
                let part = Part {
                    height,
                    round,
                    valid_round: m::Round::Nil,
                    proposer,
                    block,
                };
                let _ = reply.send(LocallyProposedValue::new(
                    height,
                    round,
                    Value(part.block.id()?),
                ));
                publish(channels, part).await?;
            }
            AppMsg::ReceivedProposalPart { part, reply, .. } => {
                let mut result = None;
                if part.sequence == 0 {
                    if let StreamContent::Data(part) = part.content {
                        if part.height.0 == state.height() + 1
                            && part.block.header.height == part.height.0
                            && part.round.is_defined()
                            && Context
                                .select_proposer(&Committee::local(), part.height, part.round)
                                .address
                                == part.proposer
                            && state.execute(&part.block).is_ok()
                        {
                            remember(&journal, &mut blocks, part.block.clone())?;
                            result = Some(proposal(&part)?);
                        }
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
                if let Some(block) = blocks.get(&value_id.0) {
                    publish(
                        channels,
                        Part {
                            height,
                            round,
                            valid_round,
                            proposer: address,
                            block: block.clone(),
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
                let block = blocks
                    .get(&certificate.value_id.0)
                    .ok_or_else(|| eyre!("decided payload unavailable"))?
                    .clone();
                verify_certificate(&block, &certificate)?;
                let next = state.execute(&block)?;
                maybe_crash(&journal, options.crash, CrashPoint::BeforeCommit)?;
                journal.append(&FinalizedRecord {
                    block,
                    certificate: encode_certificate(&certificate)?,
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
                verify_certificate(&record.block, &certificate)?;
                if state.height() != certificate.height.0 || state.tip() != certificate.value_id.0 {
                    return Err(eyre!("finalized state mismatch"));
                }
                println!("{}", status_json(&state));
                if state.height() >= options.target_height {
                    return Ok(state);
                }
                blocks = recover_candidates(&journal, &state)?;
                let _ = reply.send(Next::Start(Height(state.height() + 1), params()));
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
                        verify_record(&record)?;
                        let certificate =
                            m::ExtendedCommitCertificate::from_commit_certificate_and_extensions(
                                decode_certificate(&record.certificate)?,
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
                        };
                        remember(&journal, &mut blocks, part.block.clone())?;
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
