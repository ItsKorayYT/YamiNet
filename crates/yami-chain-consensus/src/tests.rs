//! Controlled single-height Malachite core schedules, alongside the process demo.
//! This harness reorders authenticated messages; timers and I/O are modeled.
use crate::{
    signing::{DurableSigner, SignatureVerifier},
    types::*,
    wire::*,
};
use malachite::app::{metrics::Metrics, types::core as m};
use malachite_core::{ConsensusMsg, Effect, Input, Resumable, Resume, SignedConsensusMsg};
use malachite_signing::{Signer as _, Verifier as _, VerifierExt as _};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use yami_chain::{development_key, Digest, Genesis, Signer as _, State};
use yami_chain_storage::Journal;
static ID: AtomicU64 = AtomicU64::new(0);
struct Network {
    messages: VecDeque<(usize, Input<Context>)>,
    timers: VecDeque<(usize, m::Timeout)>,
    active: usize,
    value: Value,
    partitioned: bool,
}
impl Network {
    fn deliver(&mut self, source: usize, target: usize, input: Input<Context>) {
        if !self.partitioned || (source < self.active / 2) == (target < self.active / 2) {
            self.messages.push_back((target, input));
        }
    }
}
struct Node {
    metrics: Metrics,
    state: malachite_core::State<Context>,
    signer: DurableSigner,
    decisions: Vec<m::CommitCertificate<Context>>,
}

async fn effect(
    effect: Effect<Context>,
    node: usize,
    signer: &DurableSigner,
    net: &mut Network,
    decisions: &mut Vec<m::CommitCertificate<Context>>,
) -> Result<Resume<Context>, eyre::Report> {
    Ok(match effect {
        Effect::SignVote(v, r) => r.resume_with(signer.sign_vote(v).await?),
        Effect::SignProposal(p, r) => r.resume_with(signer.sign_proposal(p).await?),
        Effect::VerifySignature(msg, key, r) => {
            let valid = match msg.message {
                ConsensusMsg::Vote(v) => SignatureVerifier(signer.genesis.clone())
                    .verify_signed_vote(&v, &msg.signature, &key)
                    .await?
                    .is_valid(),
                ConsensusMsg::Proposal(p) => SignatureVerifier(signer.genesis.clone())
                    .verify_signed_proposal(&p, &msg.signature, &key)
                    .await?
                    .is_valid(),
            };
            r.resume_with(valid)
        }
        Effect::VerifyCommitCertificate(c, set, t, r) => r.resume_with(
            SignatureVerifier(signer.genesis.clone())
                .verify_commit_certificate(&Context, &c, &set, t)
                .await,
        ),
        Effect::VerifyExtendedCommitCertificate(c, set, t, policy, r) => r.resume_with(
            SignatureVerifier(signer.genesis.clone())
                .verify_extended_commit_certificate(&Context, &c, &set, t, policy)
                .await,
        ),
        Effect::VerifyPolkaCertificate(c, set, t, r) => r.resume_with(
            SignatureVerifier(signer.genesis.clone())
                .verify_polka_certificate(&Context, &c, &set, t)
                .await,
        ),
        Effect::VerifyRoundCertificate(c, set, t, r) => r.resume_with(
            SignatureVerifier(signer.genesis.clone())
                .verify_round_certificate(&Context, &c, &set, t)
                .await,
        ),
        Effect::CertVerifiedSyncValue(response, proposer, r) => {
            let block = yami_chain::Block::decode(&response.value_bytes).unwrap();
            let mut state = State::new(signer.genesis.clone()).unwrap();
            while state.height() + 1 < block.header.height {
                state = state.execute(&state.development_block().unwrap()).unwrap();
            }
            state.execute(&block).unwrap();
            net.messages.push_back((
                node,
                Input::ProposedValue(
                    malachite_core::ProposedValue {
                        height: response.certificate.height,
                        round: response.certificate.round,
                        valid_round: m::Round::Nil,
                        proposer,
                        value: Value(block.id().unwrap()),
                        validity: m::Validity::Valid,
                    },
                    m::ValueOrigin::Sync,
                ),
            ));
            r.resume_with(())
        }
        Effect::StartRound(height, round, proposer, _, r) => {
            net.messages.push_back((
                node,
                Input::ProposedValue(
                    malachite_core::ProposedValue {
                        height,
                        round,
                        valid_round: m::Round::Nil,
                        proposer,
                        value: net.value,
                        validity: m::Validity::Valid,
                    },
                    m::ValueOrigin::Consensus,
                ),
            ));
            r.resume_with(())
        }
        Effect::ScheduleTimeout(timeout, r) => {
            net.timers.push_back((node, timeout));
            r.resume_with(())
        }
        Effect::CancelTimeout(timeout, r) => {
            net.timers.retain(|(n, t)| *n != node || *t != timeout);
            r.resume_with(())
        }
        Effect::CancelAllTimeouts(r) => {
            net.timers.retain(|(n, _)| *n != node);
            r.resume_with(())
        }
        Effect::PublishLivenessMsg(msg, r) => {
            for target in 0..net.active {
                let input = match &msg {
                    malachite_core::LivenessMsg::Vote(v) => Input::Vote(v.clone()),
                    malachite_core::LivenessMsg::PolkaCertificate(c) => {
                        Input::PolkaCertificate(c.clone())
                    }
                    malachite_core::LivenessMsg::SkipRoundCertificate(c) => {
                        Input::RoundCertificate(c.clone())
                    }
                };
                net.deliver(node, target, input);
            }
            r.resume_with(())
        }
        Effect::GetValue(height, round, _, r) => {
            net.messages.push_back((
                node,
                Input::Propose(malachite_core::LocallyProposedValue::new(
                    height, round, net.value,
                )),
            ));
            r.resume_with(())
        }
        Effect::PublishConsensusMsg(msg, r) => {
            for target in 0..net.active {
                let input = match &msg {
                    SignedConsensusMsg::Vote(v) => Input::Vote(v.clone()),
                    SignedConsensusMsg::Proposal(p) => Input::Proposal(p.clone()),
                };
                net.deliver(node, target, input);
            }
            r.resume_with(())
        }
        Effect::RepublishVote(vote, r) => {
            for target in 0..net.active {
                net.deliver(node, target, Input::Vote(vote.clone()));
            }
            r.resume_with(())
        }
        Effect::RepublishRoundCertificate(cert, r) => {
            for target in 0..net.active {
                net.deliver(node, target, Input::RoundCertificate(cert.clone()));
            }
            r.resume_with(())
        }
        Effect::Decide(cert, _, r) => {
            decisions.push(cert);
            r.resume_with(())
        }
        Effect::ExtendVote(_, _, _, _, r) => r.resume_with(None),
        Effect::VerifyVoteExtension(_, _, _, _, _, _, r) => r.resume_with(Err(
            malachite_core::VoteExtensionError::InvalidVoteExtension,
        )),
        _ => Resume::Continue,
    })
}
async fn schedule(
    active: usize,
    equivocate: bool,
    seed: u64,
) -> Vec<Vec<m::CommitCertificate<Context>>> {
    schedule_at(Genesis::local(), 1, active, equivocate, seed).await
}
async fn schedule_at(
    genesis: Genesis,
    height: u64,
    active: usize,
    equivocate: bool,
    seed: u64,
) -> Vec<Vec<m::CommitCertificate<Context>>> {
    schedule_faults(genesis, height, active, equivocate, seed, false).await
}
async fn schedule_faults(
    genesis: Genesis,
    height: u64,
    active: usize,
    equivocate: bool,
    mut seed: u64,
    partitioned: bool,
) -> Vec<Vec<m::CommitCertificate<Context>>> {
    let root = std::env::temp_dir().join(format!(
        "yami-consensus-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    let mut app_state = State::new(genesis.clone()).unwrap();
    while app_state.height() + 1 < height {
        app_state = app_state
            .execute(&app_state.development_block().unwrap())
            .unwrap();
    }
    let block = app_state.development_block().unwrap();
    let value = Value(block.id().unwrap());
    let committee = Committee::at(&genesis, height).unwrap();
    let mut nodes = Vec::new();
    let mut start = VecDeque::new();
    for i in 0..active {
        let address = committee.members()[i].address;
        let key = (0..100)
            .map(|i| development_key("consensus", i))
            .find(|k| k.verifying_key().to_bytes() == address.0 .0)
            .unwrap();
        let journal = Arc::new(Journal::open(&root.join(i.to_string())).unwrap());
        nodes.push(Node {
            metrics: Metrics::new(),
            state: malachite_core::State::new(
                Context,
                Height(height),
                committee.clone(),
                malachite_core::Params {
                    address,
                    threshold_params: Default::default(),
                    value_payload: m::ValuePayload::ProposalAndParts,
                    enabled: true,
                },
                4,
                128,
            ),
            signer: DurableSigner {
                genesis: genesis.clone(),
                key,
                journal,
                crash: None,
            },
            decisions: vec![],
        });
        // Initialization is driven before delivery of any network messages below.
        start.push_back((
            i,
            Input::StartHeight(
                Height(height),
                committee.clone(),
                false,
                None,
                m::VoteExtensionPolicy::Disabled,
            ),
        ));
    }
    let mut network = Network {
        messages: VecDeque::new(),
        timers: VecDeque::new(),
        active,
        value,
        partitioned,
    };
    // Start every node before any reordered traffic.
    while let Some((i, input)) = start.pop_front() {
        let node = &mut nodes[i];
        let result: Result<(), malachite_core::Error<Context>> = malachite_core::process!(input:input,state:&mut node.state,metrics:&node.metrics,with:e=>effect(e,i,&node.signer,&mut network,&mut node.decisions).await);
        result.unwrap();
    }
    if equivocate {
        assert_eq!(active + 1, committee.members().len());
        let address = committee.members()[active].address;
        let key = (0..100)
            .map(|i| development_key("consensus", i))
            .find(|k| k.verifying_key().to_bytes() == address.0 .0)
            .unwrap();
        // Isolated malicious key bypasses only the test signing guard.
        // Competing proposals are authenticated, but a builder value for the
        // forged block is never supplied to honest application execution.
        for v in [value, Value(Digest([0xA5; 32]))] {
            let p = Proposal {
                height: Height(height),
                round: m::Round::ZERO,
                value: v,
                pol_round: m::Round::Nil,
                address,
            };
            let sig = key
                .sign(&crate::signing::proposal_preimage_for(&genesis, &p).unwrap())
                .to_bytes();
            for i in 0..active {
                network.deliver(
                    active,
                    i,
                    Input::Proposal(m::SignedMessage::new(p.clone(), sig)),
                );
            }
        }
        for i in 0..active {
            for step in [m::VoteType::Prevote, m::VoteType::Precommit] {
                for v in [value, Value(Digest([0xA5; 32]))] {
                    let vote = Vote {
                        height: Height(height),
                        round: m::Round::ZERO,
                        value: m::NilOrVal::Val(v),
                        step,
                        address,
                        extension: None,
                    };
                    let sig = key
                        .sign(&crate::signing::vote_preimage_for(&genesis, &vote).unwrap())
                        .to_bytes();
                    network.deliver(active, i, Input::Vote(m::SignedMessage::new(vote, sig)));
                }
            }
        }
    }
    let mut steps = 0;
    let mut timer_events = 0;
    let mut synced = std::collections::BTreeSet::new();
    loop {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let (i, input) = if !network.messages.is_empty() {
            let index = (seed as usize) % network.messages.len();
            network.messages.remove(index).unwrap()
        } else {
            if nodes.iter().all(|n| !n.decisions.is_empty()) || timer_events >= 24 * active.max(1) {
                break;
            }
            if network.partitioned && timer_events >= 2 * active {
                assert!(
                    nodes.iter().all(|n| n.decisions.is_empty()),
                    "partition obtained a false quorum"
                );
                network.partitioned = false;
            }
            // Eventual certified history delivery, as provided by the live sync actor.
            if let Some(cert) = nodes
                .iter()
                .flat_map(|n| n.decisions.iter())
                .next()
                .cloned()
            {
                if let Some(i) =
                    (0..active).find(|i| nodes[*i].decisions.is_empty() && !synced.contains(i))
                {
                    synced.insert(i);
                    let keypair = malachite::app::types::Keypair::ed25519_from_bytes(
                        development_key("transport", 0).to_bytes(),
                    )
                    .unwrap();
                    let cert = m::ExtendedCommitCertificate::from_commit_certificate_and_extensions(
                        cert,
                        m::VoteExtensions::default(),
                    );
                    network.messages.push_back((
                        i,
                        Input::SyncValueResponse(m::ValueResponse::new(
                            malachite_core::PeerId::from_bytes(
                                &keypair.public().to_peer_id().to_bytes(),
                            )
                            .unwrap(),
                            bytes::Bytes::from(block.canonical().unwrap()),
                            cert,
                        )),
                    ));
                    continue;
                }
            }
            let Some((i, timeout)) = network.timers.pop_front() else {
                break;
            };
            if !nodes[i].decisions.is_empty() {
                continue;
            }
            timer_events += 1;
            (i, Input::TimeoutElapsed(timeout))
        };
        let node = &mut nodes[i];
        let result: Result<(), malachite_core::Error<Context>> = malachite_core::process!(input:input,state:&mut node.state,metrics:&node.metrics,with:e=>effect(e,i,&node.signer,&mut network,&mut node.decisions).await);
        result.unwrap();
        steps += 1;
        assert!(steps < 10000 + active * active * 100, "unbounded schedule");
    }
    let decisions = nodes.iter().map(|n| n.decisions.clone()).collect();
    for node in &nodes {
        for cert in &node.decisions {
            verify_certificate(&genesis, &block, cert).unwrap();
        }
    }
    drop(nodes);
    std::fs::remove_dir_all(root).unwrap();
    decisions
}
#[tokio::test]
async fn four_member_reordered_authenticated_votes_one_offline_or_equivocating() {
    for seed in [1, 17, 42, 111] {
        for (active, byzantine) in [(4, false), (3, false), (3, true)] {
            let decisions = schedule(active, byzantine, seed).await;
            assert!(
                decisions.iter().all(|d| d.len() == 1),
                "active={active} byzantine={byzantine} seed={seed}: {:?}",
                decisions.iter().map(Vec::len).collect::<Vec<_>>()
            );
            let value = decisions[0][0].value_id;
            assert!(decisions.iter().flatten().all(|c| c.value_id == value));
        }
    }
}
#[tokio::test]
async fn two_of_four_cannot_finalize() {
    assert!(schedule(2, false, 17).await.iter().all(Vec::is_empty));
}

#[tokio::test]
async fn durable_signer_refuses_conflict_and_certificate_tampering() {
    use m::Context as _;
    let path = std::env::temp_dir().join(format!(
        "yami-signer-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    let key = development_key("consensus", 0);
    let address = Address(Digest(key.verifying_key().to_bytes()));
    let value = Value(
        State::new(Genesis::local())
            .unwrap()
            .development_block()
            .unwrap()
            .id()
            .unwrap(),
    );
    let vote = Context.new_precommit(Height(1), m::Round::ZERO, m::NilOrVal::Val(value), address);
    let first = {
        let signer = DurableSigner {
            genesis: Genesis::local(),
            key: key.clone(),
            journal: Arc::new(Journal::open(&path).unwrap()),
            crash: None,
        };
        signer.sign_vote(vote.clone()).await.unwrap()
    };
    let signer = DurableSigner {
        genesis: Genesis::local(),
        key,
        journal: Arc::new(Journal::open(&path).unwrap()),
        crash: None,
    };
    assert_eq!(first, signer.sign_vote(vote.clone()).await.unwrap());
    let mut conflict = vote;
    conflict.value = m::NilOrVal::Nil;
    assert!(signer.sign_vote(conflict).await.is_err());
    drop(signer);
    std::fs::remove_dir_all(path).unwrap();
    let mut certificates = schedule(3, false, 1).await;
    let mut cert = certificates[0].pop().unwrap();
    cert.commit_signatures.sort_by_key(|s| s.address);
    let block = State::new(Genesis::local())
        .unwrap()
        .development_block()
        .unwrap();
    let bytes = encode_certificate(&Genesis::local(), &cert).unwrap();
    assert_eq!(decode_certificate(&Genesis::local(), &bytes).unwrap(), cert);
    let mut bad = cert.clone();
    bad.commit_signatures[0].signature[0] ^= 1;
    assert!(verify_certificate(&Genesis::local(), &block, &bad).is_err());
    let mut bad = cert.clone();
    bad.commit_signatures[1] = bad.commit_signatures[0].clone();
    assert!(verify_certificate(&Genesis::local(), &block, &bad).is_err());
    let mut bad = cert.clone();
    bad.height = Height(2);
    assert!(verify_certificate(&Genesis::local(), &block, &bad).is_err());
    let mut bad = cert;
    bad.commit_signatures.pop();
    assert!(verify_certificate(&Genesis::local(), &block, &bad).is_err());
}

#[test]
fn wire_rejects_wrong_network_truncation_trailing_data_and_oversized_sync() {
    use m::Context as _;
    use malachite::app::types::codec::Codec;
    let address = Committee::local().members()[0].address;
    let vote = Context.new_prevote(Height(1), m::Round::ZERO, m::NilOrVal::Nil, address);
    let message = SignedConsensusMsg::Vote(m::SignedMessage::new(vote, [0; 64]));
    let bytes = WireCodec::new(&Genesis::local()).encode(&message).unwrap();
    let decode = |bytes| {
        <WireCodec as Codec<SignedConsensusMsg<Context>>>::decode(
            &WireCodec::new(&Genesis::local()),
            bytes,
        )
    };
    assert_eq!(decode(bytes.clone()).unwrap(), message);
    for len in 0..bytes.len() {
        assert!(decode(bytes.slice(..len)).is_err());
    }
    let mut wrong = bytes.to_vec();
    wrong[8] ^= 1;
    assert!(decode(wrong.into()).is_err());
    let mut trailing = bytes.to_vec();
    trailing.push(0);
    assert!(decode(trailing.into()).is_err());
    assert!(decode(bytes::Bytes::from(vec![0; MAX_MESSAGE + 1])).is_err());
    let request = malachite_sync::Request::<Context>::ValueRequest(
        malachite_sync::ValueRequest::new(Height(1)..=Height(9)),
    );
    let encoded = WireCodec::new(&Genesis::local()).encode(&request).unwrap();
    assert!(
        <WireCodec as Codec<malachite_sync::Request<Context>>>::decode(
            &WireCodec::new(&Genesis::local()),
            encoded
        )
        .is_err()
    );
    let cert = m::CommitCertificate::<Context> {
        height: Height(1),
        round: m::Round::ZERO,
        value_id: Value(Digest([0; 32])),
        commit_signatures: vec![],
    };
    let cert = m::ExtendedCommitCertificate::from_commit_certificate_and_extensions(
        cert,
        m::VoteExtensions::default(),
    );
    let response = malachite_sync::Response::ValueResponse(malachite_sync::ValueResponse::new(
        Height(1),
        vec![malachite_sync::RawDecidedValue::new(bytes::Bytes::new(), cert); 9],
    ));
    assert!(WireCodec::new(&Genesis::local()).encode(&response).is_err());
}

fn certify(
    genesis: &Genesis,
    block: &yami_chain::Block,
    limit: usize,
) -> m::CommitCertificate<Context> {
    let height = Height(block.header.height);
    let value_id = Value(block.id().unwrap());
    let signatures = genesis
        .committee_at(height.0)
        .unwrap()
        .members()
        .iter()
        .take(limit)
        .map(|member| {
            let key = (0..100)
                .map(|i| development_key("consensus", i))
                .find(|key| key.verifying_key().to_bytes() == member.consensus_key)
                .unwrap();
            let address = Address(Digest(member.consensus_key));
            let vote = Vote {
                height,
                round: m::Round::ZERO,
                value: m::NilOrVal::Val(value_id),
                step: m::VoteType::Precommit,
                address,
                extension: None,
            };
            m::CommitSignature::new(
                address,
                key.sign(&crate::signing::vote_preimage_for(genesis, &vote).unwrap())
                    .to_bytes(),
            )
        })
        .collect();
    m::CommitCertificate {
        height,
        round: m::Round::ZERO,
        value_id,
        commit_signatures: signatures,
    }
}
#[tokio::test]
async fn malachite_uses_different_height_specific_committees() {
    let genesis = Genesis::development_schedule(&[(1, 4), (3, 7), (5, 3)]).unwrap();
    for (height, count) in [(1, 4), (3, 7), (5, 3)] {
        let decisions = schedule_at(genesis.clone(), height, count, false, 17).await;
        assert!(decisions.iter().all(|d| d.len() == 1));
        assert!(decisions
            .iter()
            .flatten()
            .all(|c| c.value_id == decisions[0][0].value_id));
    }
}
#[test]
fn committees_certificates_cover_sizes_weights_history_and_recovery() {
    for size in [1, 2, 4, 7, 25, 100] {
        let genesis = Genesis::development_schedule(&[(1, size)]).unwrap();
        let block = State::new(genesis.clone())
            .unwrap()
            .development_block()
            .unwrap();
        let cert = certify(&genesis, &block, size);
        verify_certificate(&genesis, &block, &cert).unwrap();
        assert_eq!(
            decode_certificate(&genesis, &encode_certificate(&genesis, &cert).unwrap()).unwrap(),
            cert
        );
        let insufficient = certify(&genesis, &block, (2 * size) / 3);
        assert!(verify_certificate(&genesis, &block, &insufficient).is_err());
    }
    let mut genesis = Genesis::development_schedule(&[(1, 3)]).unwrap();
    let mut members = genesis.epochs[0].validators.members().to_vec();
    members[0].power = 5;
    genesis.epochs[0].validators = yami_chain::ValidatorSet::new(members).unwrap();
    let block = State::new(genesis.clone())
        .unwrap()
        .development_block()
        .unwrap();
    verify_certificate(&genesis, &block, &certify(&genesis, &block, 1)).unwrap();
    let mut impostor = certify(&genesis, &block, 3);
    impostor.commit_signatures.remove(0);
    assert!(verify_certificate(&genesis, &block, &impostor).is_err());

    let genesis = Genesis::development_schedule(&[(1, 4), (3, 7), (5, 3)]).unwrap();
    let path = std::env::temp_dir().join(format!("yami-epochs-{}", std::process::id()));
    let mut state = State::new(genesis.clone()).unwrap();
    for _ in 0..7 {
        // Reopen and authenticate the complete history after every durable write,
        // including immediately before and after both epoch boundaries.
        let journal = Journal::open(&path).unwrap();
        let replay = journal
            .replay(genesis.clone(), |r| verify_record(&genesis, r))
            .unwrap();
        assert_eq!(replay, state);
        let block = state.development_block().unwrap();
        let cert = certify(&genesis, &block, 100);
        journal
            .append(&yami_chain_storage::FinalizedRecord {
                block: block.clone(),
                certificate: encode_certificate(&genesis, &cert).unwrap(),
            })
            .unwrap();
        state = state.execute(&block).unwrap();
    }
    let journal = Journal::open(&path).unwrap();
    let checkpoint = crate::authenticate_checkpoint(&journal, &genesis, 3, 7).unwrap();
    assert_eq!(checkpoint.height(), 3);
    checkpoint.persist(&journal).unwrap();
    let bytes = checkpoint.canonical().unwrap();
    crate::verify_checkpoint(&journal, &genesis, 3, 7, &bytes).unwrap();
    for i in [0, 20, 60, 100, bytes.len() - 1] {
        let mut forged = bytes.clone();
        forged[i] ^= 1;
        assert!(crate::verify_checkpoint(&journal, &genesis, 3, 7, &forged).is_err());
    }
    assert!(crate::authenticate_checkpoint(&journal, &genesis, 3, 2).is_err());
    let inventory = crate::history_snapshot_inventory(&journal, &genesis, 3, 1_000_000).unwrap();
    assert_eq!(
        inventory.commitment().unwrap(),
        crate::history_snapshot_inventory(&journal, &genesis, 3, 1_000_000)
            .unwrap()
            .commitment()
            .unwrap()
    );
    assert!(crate::history_snapshot_inventory(&journal, &genesis, 3, 0).is_err());
    assert!(crate::history_snapshot_inventory(&journal, &genesis, 1025, u64::MAX).is_err());
    assert!(crate::history_snapshot_inventory(&journal, &Genesis::local(), 3, 1_000_000).is_err());
    assert_ne!(
        inventory.commitment().unwrap(),
        crate::history_snapshot_inventory(&journal, &genesis, 2, 1_000_000)
            .unwrap()
            .commitment()
            .unwrap()
    );
    let inventory_bytes = inventory.canonical().unwrap();
    crate::verify_history_inventory(&journal, &genesis, 3, 1_000_000, &inventory_bytes).unwrap();
    let mut tampered = inventory_bytes.clone();
    tampered[20] ^= 1;
    assert!(crate::verify_history_inventory(&journal, &genesis, 3, 1_000_000, &tampered).is_err());
    assert!(crate::verify_history_inventory(
        &journal,
        &genesis,
        3,
        1_000_000,
        &inventory_bytes[..20]
    )
    .is_err());
    checkpoint.prune_candidates(&journal).unwrap();
    assert!(journal.record(1).unwrap().is_some());
    let record = journal.record(3).unwrap().unwrap();
    let mut unauthorized = genesis.clone();
    unauthorized.epochs[1].first_height = 4;
    assert!(verify_record(&unauthorized, &record).is_err());
    assert!(journal
        .replay(unauthorized.clone(), |r| verify_record(&unauthorized, r))
        .is_err());
    assert_eq!(
        journal
            .replay(genesis.clone(), |r| verify_record(&genesis, r))
            .unwrap(),
        state
    );
    drop(journal);
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
async fn transport_binding_malformed_votes_and_proposal_reservations_fail_closed() {
    use m::Context as _;
    let genesis = Genesis::local();
    let path = std::env::temp_dir().join(format!("yami-transport-{}", std::process::id()));
    let key = development_key("consensus", 0);
    let signer = DurableSigner {
        key: key.clone(),
        genesis: genesis.clone(),
        journal: Arc::new(Journal::open(&path).unwrap()),
        crash: None,
    };
    let peer = |index| {
        malachite::app::types::Keypair::ed25519_from_bytes(
            development_key("transport", index).to_bytes(),
        )
        .unwrap()
        .public()
        .to_peer_id()
        .to_bytes()
    };
    let proof = signer
        .sign_validator_proof(key.verifying_key().to_bytes().to_vec(), peer(0))
        .await
        .unwrap();
    let verifier = SignatureVerifier(genesis.clone());
    assert!(verifier
        .verify_validator_proof(&proof)
        .await
        .unwrap()
        .is_valid());
    assert!(signer
        .sign_validator_proof(key.verifying_key().to_bytes().to_vec(), peer(1))
        .await
        .is_err());
    let mut forged = proof;
    forged.peer_id = peer(1);
    forged.signature = key.sign(&forged.preimage()).to_bytes();
    assert!(!verifier
        .verify_validator_proof(&forged)
        .await
        .unwrap()
        .is_valid());
    let vote = Context.new_prevote(
        Height(0),
        m::Round::ZERO,
        m::NilOrVal::Nil,
        Address(Digest(key.verifying_key().to_bytes())),
    );
    assert!(!verifier
        .verify_signed_vote(&vote, &[0; 64], &key.verifying_key().to_bytes())
        .await
        .unwrap()
        .is_valid());
    drop(signer);
    std::fs::remove_dir_all(&path).unwrap();

    let address = Context
        .select_proposer(&Committee::local(), Height(1), m::Round::ZERO)
        .address;
    let key = (0..4)
        .map(|i| development_key("consensus", i))
        .find(|k| k.verifying_key().to_bytes() == address.0 .0)
        .unwrap();
    let proposal = Context.new_proposal(
        Height(1),
        m::Round::ZERO,
        Value(Digest([1; 32])),
        m::Round::Nil,
        address,
    );
    let signature = {
        let signer = DurableSigner {
            key: key.clone(),
            genesis: genesis.clone(),
            journal: Arc::new(Journal::open(&path).unwrap()),
            crash: None,
        };
        signer.sign_proposal(proposal.clone()).await.unwrap()
    };
    let signer = DurableSigner {
        key,
        genesis,
        journal: Arc::new(Journal::open(&path).unwrap()),
        crash: None,
    };
    assert_eq!(
        signature,
        signer.sign_proposal(proposal.clone()).await.unwrap()
    );
    let mut conflict = proposal;
    conflict.value = Value(Digest([2; 32]));
    assert!(signer.sign_proposal(conflict).await.is_err());
    drop(signer);
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
#[ignore = "manual 4/25/100-validator authenticated Malachite schedules; run in release as documented"]
async fn scale_stress_authenticated_core() {
    for size in [4, 25, 50, 100] {
        let genesis = Genesis::development_schedule(&[(1, size)]).unwrap();
        for (name, active, malicious, partitioned) in [
            ("reordered", size, false, false),
            ("malicious", size - 1, true, false),
            ("partition-recovery", size, false, true),
        ] {
            let start = std::time::Instant::now();
            let result = schedule_faults(
                genesis.clone(),
                size as u64,
                active,
                malicious,
                17,
                partitioned,
            )
            .await;
            assert!(
                result.iter().all(|d| d.len() == 1),
                "size={size} case={name}: {:?}",
                result.iter().map(Vec::len).collect::<Vec<_>>()
            );
            assert!(result
                .iter()
                .flatten()
                .all(|c| c.value_id == result[0][0].value_id));
            println!(
                "scale-core size={size} case={name} elapsed_ms={} authenticated_decisions={}",
                start.elapsed().as_millis(),
                result.len()
            );
        }
    }
}
