//! Parser-only fuzz entry point. This never signs, executes, imports snapshots,
//! verifies payments or mutates a store. Enabled only for tests/fuzz tooling.
use crate::{types::*, wire::*};
use bytes::Bytes;
use malachite::app::{
    streaming::{StreamContent, StreamId, StreamMessage},
    types::{core as m, sync},
};
use malachite_core::{LivenessMsg, ProposedValue, SignedConsensusMsg};
use yami_chain::{Block, Genesis, State, Transaction};
use yami_chain_storage::FinalizedRecord;

fn probe<T: Wire>(genesis: yami_chain::Digest, bytes: Bytes) {
    if let Ok(value) = super::wire::decode::<T>(genesis, bytes.clone()) {
        assert_eq!(
            super::wire::encode(genesis, &value).unwrap(),
            bytes,
            "accepted noncanonical network input"
        );
    }
}
pub fn fuzz_decode(bytes: &[u8]) {
    if bytes.len() > MAX_MESSAGE + 1 {
        return;
    }
    let genesis = Genesis::local().id();
    let owned = Bytes::copy_from_slice(bytes);
    macro_rules! probes {($($t:ty),*)=>{$(probe::<$t>(genesis,owned.clone());)*};}
    probes!(
        SignedConsensusMsg<Context>,
        LivenessMsg<Context>,
        Part,
        StreamMessage<Part>,
        m::ValidatorProof<Context>,
        ProposedValue<Context>,
        m::PolkaCertificate<Context>,
        sync::Status<Context>,
        sync::Request<Context>,
        sync::Response<Context>,
        m::CommitCertificate<Context>
    );
    if let Ok(block) = Block::decode(bytes) {
        assert_eq!(block.canonical().unwrap(), bytes);
    }
    if let Ok(tx) = Transaction::decode(bytes) {
        assert_eq!(tx.canonical().unwrap(), bytes);
    }
    if let Ok(record) = FinalizedRecord::decode(bytes) {
        assert_eq!(record.canonical().unwrap(), bytes);
    }
}
pub fn fuzz_seeds() -> Vec<Vec<u8>> {
    let genesis = Genesis::local();
    let state = State::new(genesis.clone()).unwrap();
    let block = state.development_block().unwrap();
    let proposer = Committee::local().members()[0].address;
    let vote = Vote {
        height: Height(1),
        round: m::Round::ZERO,
        value: m::NilOrVal::Nil,
        step: m::VoteType::Prevote,
        address: proposer,
        extension: None,
    };
    let part = Part {
        height: Height(1),
        round: m::Round::ZERO,
        valid_round: m::Round::Nil,
        proposer,
        block: block.clone(),
        signature: [0; 64],
    };
    let certificate = m::CommitCertificate::<Context> {
        height: Height(1),
        round: m::Round::ZERO,
        value_id: Value(block.id().unwrap()),
        commit_signatures: vec![],
    };
    let peer = malachite::app::types::Keypair::ed25519_from_bytes(
        yami_chain::development_key("transport", 0).to_bytes(),
    )
    .unwrap()
    .public()
    .to_peer_id();
    let extended = m::ExtendedCommitCertificate::from_commit_certificate_and_extensions(
        certificate.clone(),
        m::VoteExtensions::default(),
    );
    let mut seeds = vec![
        block.canonical().unwrap(),
        block.transactions[0].canonical().unwrap(),
        FinalizedRecord {
            block,
            certificate: encode_certificate(&genesis, &certificate).unwrap(),
        }
        .canonical()
        .unwrap(),
    ];
    macro_rules! seed {
        ($v:expr) => {
            seeds.push(super::wire::encode(genesis.id(), &$v).unwrap().to_vec())
        };
    }
    seed!(SignedConsensusMsg::Vote(m::SignedMessage::new(
        vote.clone(),
        [0; 64]
    )));
    seed!(SignedConsensusMsg::Proposal(m::SignedMessage::new(
        part.proposal().unwrap(),
        [0; 64]
    )));
    seed!(LivenessMsg::Vote(m::SignedMessage::new(vote, [0; 64])));
    seed!(part.clone());
    seed!(StreamMessage::new(
        StreamId::new(Bytes::from_static(b"seed")),
        0,
        StreamContent::Data(part)
    ));
    seed!(m::ValidatorProof::<Context>::new(
        proposer.0 .0.to_vec(),
        peer.to_bytes(),
        [0; 64]
    ));
    seed!(ProposedValue::<Context> {
        height: Height(1),
        round: m::Round::ZERO,
        valid_round: m::Round::Nil,
        proposer,
        value: certificate.value_id,
        validity: m::Validity::Valid
    });
    seed!(m::PolkaCertificate::<Context> {
        height: Height(1),
        round: m::Round::ZERO,
        value_id: certificate.value_id,
        polka_signatures: vec![]
    });
    seed!(sync::Status::<Context> {
        peer_id: malachite::app::types::PeerId::from_bytes(&peer.to_bytes()).unwrap(),
        tip_height: Height(1),
        history_min_height: Height(1)
    });
    seed!(sync::Request::<Context>::ValueRequest(
        malachite_sync::ValueRequest::new(Height(1)..=Height(8))
    ));
    seed!(sync::Response::<Context>::ValueResponse(
        malachite_sync::ValueResponse::new(
            Height(1),
            vec![sync::RawDecidedValue::new(Bytes::new(), extended)]
        )
    ));
    seed!(certificate);
    seeds
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seeded_mutation_fuzz_and_all_truncations_are_bounded_and_canonical() {
        let seeds = fuzz_seeds();
        let mut random = 17u64;
        for seed in &seeds {
            for length in 0..=seed.len() {
                fuzz_decode(&seed[..length]);
            }
        }
        for n in 0..20_000 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let mut bytes = seeds[n % seeds.len()].clone();
            for _ in 0..1 + random % 4 {
                random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                let index = random as usize % bytes.len();
                bytes[index] ^= (random >> 32) as u8;
            }
            if n % 7 == 0 {
                bytes.extend_from_slice(&random.to_be_bytes());
            }
            fuzz_decode(&bytes);
        }
        for length in [0, 1, 32, 1024, 65_536, MAX_MESSAGE, MAX_MESSAGE + 1] {
            fuzz_decode(&vec![0xff; length]);
        }
    }
}
