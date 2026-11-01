//! Controlled single-height Malachite core schedules, alongside the process demo.
//! This harness reorders authenticated messages; timers and I/O are modeled.
use crate::{
    signing::{vote_preimage, DurableSigner, SignatureVerifier},
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
                ConsensusMsg::Vote(v) => SignatureVerifier
                    .verify_signed_vote(&v, &msg.signature, &key)
                    .await?
                    .is_valid(),
                ConsensusMsg::Proposal(p) => SignatureVerifier
                    .verify_signed_proposal(&p, &msg.signature, &key)
                    .await?
                    .is_valid(),
            };
            r.resume_with(valid)
        }
        Effect::VerifyCommitCertificate(c, set, t, r) => r.resume_with(
            SignatureVerifier
                .verify_commit_certificate(&Context, &c, &set, t)
                .await,
        ),
        Effect::VerifyExtendedCommitCertificate(c, set, t, policy, r) => r.resume_with(
            SignatureVerifier
                .verify_extended_commit_certificate(&Context, &c, &set, t, policy)
                .await,
        ),
        Effect::VerifyPolkaCertificate(c, set, t, r) => r.resume_with(
            SignatureVerifier
                .verify_polka_certificate(&Context, &c, &set, t)
                .await,
        ),
        Effect::VerifyRoundCertificate(c, set, t, r) => r.resume_with(
            SignatureVerifier
                .verify_round_certificate(&Context, &c, &set, t)
                .await,
        ),
        Effect::CertVerifiedSyncValue(response, proposer, r) => {
            let block = yami_chain::Block::decode(&response.value_bytes).unwrap();
            State::new(Genesis::local())
                .unwrap()
                .execute(&block)
                .unwrap();
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
                net.messages.push_back((target, input));
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
                net.messages.push_back((target, input));
            }
            r.resume_with(())
        }
        Effect::RepublishVote(vote, r) => {
            for target in 0..net.active {
                net.messages.push_back((target, Input::Vote(vote.clone())));
            }
            r.resume_with(())
        }
        Effect::RepublishRoundCertificate(cert, r) => {
            for target in 0..net.active {
                net.messages
                    .push_back((target, Input::RoundCertificate(cert.clone())));
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
    mut seed: u64,
) -> Vec<Vec<m::CommitCertificate<Context>>> {
    let root = std::env::temp_dir().join(format!(
        "yami-consensus-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    let block = State::new(Genesis::local())
        .unwrap()
        .development_block()
        .unwrap();
    let value = Value(block.id().unwrap());
    let committee = Committee::local();
    let mut nodes = Vec::new();
    let mut start = VecDeque::new();
    for i in 0..active {
        let address = committee.0[i].address;
        let key = (0..4)
            .map(|i| development_key("consensus", i))
            .find(|k| k.verifying_key().to_bytes() == address.0 .0)
            .unwrap();
        let journal = Arc::new(Journal::open(&root.join(i.to_string())).unwrap());
        nodes.push(Node {
            metrics: Metrics::new(),
            state: malachite_core::State::new(
                Context,
                Height(1),
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
                Height(1),
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
    };
    // Start every node before any reordered traffic.
    while let Some((i, input)) = start.pop_front() {
        let node = &mut nodes[i];
        let result: Result<(), malachite_core::Error<Context>> = malachite_core::process!(input:input,state:&mut node.state,metrics:&node.metrics,with:e=>effect(e,i,&node.signer,&mut network,&mut node.decisions).await);
        result.unwrap();
    }
    if equivocate {
        assert_eq!(active, 3);
        let address = committee.0[3].address;
        let key = (0..4)
            .map(|i| development_key("consensus", i))
            .find(|k| k.verifying_key().to_bytes() == address.0 .0)
            .unwrap();
        // Deliberately malicious fourth key bypasses the production signing guard.
        for i in 0..active {
            for step in [m::VoteType::Prevote, m::VoteType::Precommit] {
                for v in [value, Value(Digest([0xA5; 32]))] {
                    let vote = Vote {
                        height: Height(1),
                        round: m::Round::ZERO,
                        value: m::NilOrVal::Val(v),
                        step,
                        address,
                        extension: None,
                    };
                    let sig = key.sign(&vote_preimage(&vote).unwrap()).to_bytes();
                    network
                        .messages
                        .push_back((i, Input::Vote(m::SignedMessage::new(vote, sig))));
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
            if nodes.iter().all(|n| !n.decisions.is_empty()) || timer_events >= 24 {
                break;
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
        assert!(steps < 10_000, "unbounded schedule");
    }
    let decisions = nodes.iter().map(|n| n.decisions.clone()).collect();
    for node in &nodes {
        for cert in &node.decisions {
            verify_certificate(&block, cert).unwrap();
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
            key: key.clone(),
            journal: Arc::new(Journal::open(&path).unwrap()),
            crash: None,
        };
        signer.sign_vote(vote.clone()).await.unwrap()
    };
    let signer = DurableSigner {
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
    let bytes = encode_certificate(&cert).unwrap();
    assert_eq!(decode_certificate(&bytes).unwrap(), cert);
    let mut bad = cert.clone();
    bad.commit_signatures[0].signature[0] ^= 1;
    assert!(verify_certificate(&block, &bad).is_err());
    let mut bad = cert.clone();
    bad.commit_signatures[1] = bad.commit_signatures[0].clone();
    assert!(verify_certificate(&block, &bad).is_err());
    let mut bad = cert.clone();
    bad.height = Height(2);
    assert!(verify_certificate(&block, &bad).is_err());
    let mut bad = cert;
    bad.commit_signatures.pop();
    assert!(verify_certificate(&block, &bad).is_err());
}

#[test]
fn wire_rejects_wrong_network_truncation_trailing_data_and_oversized_sync() {
    use m::Context as _;
    use malachite::app::types::codec::Codec;
    let address = Committee::local().0[0].address;
    let vote = Context.new_prevote(Height(1), m::Round::ZERO, m::NilOrVal::Nil, address);
    let message = SignedConsensusMsg::Vote(m::SignedMessage::new(vote, [0; 64]));
    let bytes = WireCodec.encode(&message).unwrap();
    let decode =
        |bytes| <WireCodec as Codec<SignedConsensusMsg<Context>>>::decode(&WireCodec, bytes);
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
    let encoded = WireCodec.encode(&request).unwrap();
    assert!(
        <WireCodec as Codec<malachite_sync::Request<Context>>>::decode(&WireCodec, encoded)
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
    assert!(WireCodec.encode(&response).is_err());
}
