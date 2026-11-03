use yami_chain::{privacy::*, selection::*, Digest, Genesis};
#[test]
fn simulations_never_activate_production_elections_or_payments() {
    let g = Genesis::local();
    let snapshot = EligibilitySnapshot {
        epoch: 1,
        activation_height: 2,
        parent_checkpoint: Digest([1; 32]),
        accounting_root: Digest([2; 32]),
        eligibility_root: g.committee_at(1).unwrap().hash(),
        eligible_count: 100,
        total_weight: 100,
        committee_size: 4,
    };
    assert_eq!(
        ElectionsDisabled.select(&snapshot, Digest([3; 32])),
        Err(yami_chain::Error::Unsupported)
    );
    let mut transfer = ConfidentialTransfer {
        binding: BindingContext {
            chain_id: g.chain_id,
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
            bytes: vec![0; 64],
        },
    };
    for version in 0..=u16::MAX {
        transfer.proof.version = version;
        assert!(PrivacyVerifier.verify_payment(&transfer).is_err());
    }
    let mut action = ConfidentialProtocolAction {
        transfer,
        module: 1,
        public_effect: vec![],
    };
    for version in 0..=u16::MAX {
        action.transfer.proof.version = version;
        assert!(PrivacyVerifier.verify_protocol_action(&action).is_err());
    }
    assert!(yami_chain::Transaction::decode(b"YC-SIM-FROZEN-ELECTION-1").is_err());
}
