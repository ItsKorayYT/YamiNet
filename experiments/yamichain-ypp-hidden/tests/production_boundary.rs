// Production library is built without cfg(test). This experiment cannot register
// a backend and must not change rejection of any u16 proof version.
use yami_chain_privacy::*;
#[test]
fn all_65536_versions_still_rejected_for_payments_and_protocol_actions() {
    let mut t = ConfidentialTransfer {
        binding: BindingContext {
            chain_id: "yamichain-local-0.2".into(),
            protocol_version: 2,
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
            bytes: vec![0; 192],
        },
    };
    for version in 0..=u16::MAX {
        t.proof.version = version;
        assert!(
            matches!(PrivacyVerifier.verify_payment(&t),Err(VerificationError::UnsupportedProofVersion(v)) if v==version)
        );
        let action = ConfidentialProtocolAction {
            transfer: t.clone(),
            module: 1,
            public_effect: vec![0; 32],
        };
        assert!(PrivacyVerifier.verify_protocol_action(&action).is_err());
    }
}
