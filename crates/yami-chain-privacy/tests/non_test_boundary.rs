// Integration test: library is compiled WITHOUT cfg(test). No fixture fallback.
use yami_chain_privacy::*;
#[test]
fn every_proof_version_and_public_value_action_fails_closed() {
    let mut intent = ConfidentialTransfer {
        binding: BindingContext {
            chain_id: "yamichain-local-0.1".into(),
            protocol_version: 1,
            intent_hash: [0; 32],
            finalized_anchor: [0; 32],
            expiry_height: 1,
            fee_epoch: 0,
            public_fee: 0,
        },
        backend_public_inputs: vec![],
        encrypted_outputs: vec![],
        proof: OpaqueProof {
            version: 0,
            bytes: vec![0; 64],
        },
    };
    for version in 0..=u16::MAX {
        intent.proof.version = version;
        assert!(
            matches!(PrivacyVerifier.verify_payment(&intent),Err(VerificationError::UnsupportedProofVersion(v)) if v==version)
        );
    }
    let action = ConfidentialProtocolAction {
        transfer: intent,
        module: 1,
        public_effect: vec![0; 32],
    };
    assert!(PrivacyVerifier.verify_protocol_action(&action).is_err());
}
