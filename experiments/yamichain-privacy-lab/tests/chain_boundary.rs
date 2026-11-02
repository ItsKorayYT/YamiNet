use yami_chain::privacy::*;
use yamichain_privacy_lab::{create_payment, funded_fixture, wallet::Wallet, Context};

#[test]
fn real_ringct_packet_cannot_authorize_any_chain_proof_version_or_decode_as_an_action() {
    let wallet = Wallet::generate(0);
    let receiver = Wallet::generate(0);
    let (state, inputs) = funded_fixture(&wallet, &[100_000]).unwrap();
    let packet = create_payment(
        &wallet,
        inputs,
        &receiver.address(),
        25_000,
        Context {
            intent: [0; 32],
            anchor: state.anchor(),
            expiry: 10,
            fee_epoch: 0,
            fee: 0,
        },
    )
    .unwrap();
    state.verify(&packet, 1).unwrap();
    assert!(yami_chain::Transaction::decode(&packet.encode()).is_err());
    assert!(yami_chain::Transaction::decode(&packet.transaction.serialize()).is_err());
    let mut action = ConfidentialProtocolAction {
        module: 1,
        public_effect: vec![],
        transfer: ConfidentialTransfer {
            binding: BindingContext {
                chain_id: yami_chain::CHAIN_ID.into(),
                protocol_version: yami_chain::PROTOCOL,
                intent_hash: packet.context.intent,
                finalized_anchor: packet.context.anchor,
                expiry_height: 10,
                fee_epoch: 0,
                public_fee: u128::from(packet.context.fee),
            },
            backend_public_inputs: vec![],
            encrypted_outputs: vec![],
            proof: OpaqueProof {
                version: 0,
                bytes: packet.encode(),
            },
        },
    };
    for version in 0..=u16::MAX {
        action.transfer.proof.version = version;
        assert_eq!(
            PrivacyVerifier
                .verify_payment(&action.transfer)
                .unwrap_err(),
            VerificationError::UnsupportedProofVersion(version)
        );
        assert_eq!(
            PrivacyVerifier.verify_protocol_action(&action).unwrap_err(),
            VerificationError::UnsupportedProofVersion(version)
        );
    }
}
