use monero_wallet::{
    ed25519::{Commitment, Scalar},
    ringct::{EncryptedAmount, RctProofs, RctPrunable},
    transaction::{Input, Transaction},
};
use yamichain_privacy_lab::{
    create_payment, discover, funded_fixture, wallet::Wallet, with_decoys, Context, Error,
    LabState, Packet, MAX_PACKET_BYTES,
};

fn context(state: &LabState) -> Context {
    Context {
        intent: [7; 32],
        anchor: state.anchor(),
        expiry: 10,
        fee_epoch: 3,
        fee: 0,
    }
}

fn payment(amounts: &[u64]) -> (Wallet, Wallet, LabState, Packet) {
    let sender = Wallet::generate(0);
    let receiver = Wallet::generate(0);
    let (state, inputs) = funded_fixture(&sender, amounts).unwrap();
    let packet = create_payment(
        &sender,
        inputs,
        &receiver.address(),
        25_000,
        context(&state),
    )
    .unwrap();
    (sender, receiver, state, packet)
}

fn proofs(packet: &mut Packet) -> &mut RctProofs {
    let Transaction::V2 {
        proofs: Some(proofs),
        ..
    } = &mut packet.transaction
    else {
        panic!("expected full RingCT");
    };
    proofs
}

fn rejected_atomically(state: &LabState, packet: &Packet) {
    let mut copy = state.clone();
    let before = copy.anchor();
    let spent = copy.spent_count();
    assert!(copy.apply(packet, 1).is_err());
    assert_eq!(copy.anchor(), before);
    assert_eq!(copy.spent_count(), spent);
}

#[test]
fn real_transfer_viewing_change_and_recipient_spending() {
    for amounts in [vec![100_000], vec![60_000, 40_000]] {
        let (sender, receiver, mut state, packet) = payment(&amounts);
        let decoded = Packet::decode(&packet.encode()).unwrap();
        state.verify(&decoded, 1).unwrap();
        let first = state.apply(&decoded, 1).unwrap();
        assert_eq!(state.spent_count(), amounts.len());
        assert_eq!(state.apply(&decoded, 1), Err(Error::Spent));
        let received = discover(receiver.view_pair(), &decoded, first).unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].commitment().amount, 25_000);
        let change = discover(sender.view_pair(), &decoded, first).unwrap();
        assert_eq!(change.len(), 1);
        assert_eq!(
            change[0].commitment().amount + 25_000 + decoded.context.fee,
            amounts.iter().sum::<u64>()
        );
        assert!(discover(Wallet::generate(0).view_pair(), &decoded, first)
            .unwrap()
            .is_empty());
        let input = with_decoys(&state, &received[0]).unwrap();
        let spend = create_payment(
            &receiver,
            vec![input],
            &sender.address(),
            10_000,
            context(&state),
        )
        .unwrap();
        state.apply(&spend, 2).unwrap();
        assert_eq!(state.spent_count(), amounts.len() + 1);
    }
}

#[test]
fn encrypted_backup_restores_address_viewing_and_spend_authority() {
    let (sender, receiver, mut state, packet) = payment(&[100_000]);
    let key = age::x25519::Identity::generate();
    let archive = receiver.backup(&key.to_public()).unwrap();
    let other_archive = receiver.backup(&key.to_public()).unwrap();
    assert_ne!(archive, other_archive);
    let recovered = Wallet::restore(&archive, &key).unwrap();
    assert_eq!(receiver.address(), recovered.address());
    assert_eq!(receiver.birthday(), recovered.birthday());
    let first = state.apply(&packet, 1).unwrap();
    let original = discover(receiver.view_pair(), &packet, first).unwrap();
    let rescanned = discover(recovered.view_pair(), &packet, first).unwrap();
    assert_eq!(original, rescanned);
    let spend = create_payment(
        &recovered,
        vec![with_decoys(&state, &rescanned[0]).unwrap()],
        &sender.address(),
        10_000,
        context(&state),
    )
    .unwrap();
    state.verify(&spend, 2).unwrap();
    assert!(Wallet::restore(&archive, &age::x25519::Identity::generate()).is_err());
    let mut altered = archive.clone();
    let last = altered.len() - 1;
    altered[last] ^= 1;
    assert!(Wallet::restore(&altered, &key).is_err());
    for length in [0, 1, archive.len() / 2, archive.len() - 1] {
        assert!(Wallet::restore(&archive[..length], &key).is_err());
    }
    assert!(Wallet::restore(&vec![0; 4097], &key).is_err());
}

#[test]
fn every_context_field_and_outputs_are_bound() {
    let (_, _, state, good) = payment(&[100_000]);
    for field in 0..6 {
        let mut bad = good.clone();
        match field {
            0 => bad.context.intent[0] ^= 1,
            1 => bad.context.anchor[0] ^= 1,
            2 => bad.context.expiry += 1,
            3 => bad.context.fee_epoch += 1,
            4 => bad.context.fee += 1,
            _ => bad.transaction.prefix_mut().outputs.swap(0, 1),
        }
        rejected_atomically(&state, &bad);
    }
    assert_eq!(state.verify(&good, 11), Err(Error::Expired));
    let mut bytes = good.encode();
    bytes[0] ^= 1;
    assert!(Packet::decode(&bytes).is_err());
}

#[test]
fn fee_inflation_commitment_and_ciphertext_tampering_fail() {
    let (_, receiver, state, good) = payment(&[100_000]);
    for field in 0..4 {
        let mut bad = good.clone();
        let p = proofs(&mut bad);
        match field {
            0 => p.base.fee += 1,
            1 => p.base.commitments[0] = Commitment::new(Scalar::ONE, u64::MAX).commit().compress(),
            2 => {
                let EncryptedAmount::Compact { amount } = &mut p.base.encrypted_amounts[0] else {
                    panic!("expected compact amount");
                };
                amount[0] ^= 1;
            }
            _ => p.base.encrypted_amounts.pop().map(|_| ()).unwrap(),
        }
        rejected_atomically(&state, &bad);
        if field == 2 {
            // Whichever output was changed cannot be discovered with an inconsistent opening.
            let outputs = discover(receiver.view_pair(), &bad, 16).unwrap();
            assert!(outputs.is_empty() || outputs[0].commitment().amount == 25_000);
        }
    }
}

#[test]
fn corrupted_recipient_ciphertext_is_not_discovered_and_overspending_cannot_be_built() {
    let (sender, receiver, state, good) = payment(&[100_000]);
    let received = discover(receiver.view_pair(), &good, 16).unwrap();
    let index = received[0].index_in_transaction() as usize;
    let mut bad = good.clone();
    let EncryptedAmount::Compact { amount } = &mut proofs(&mut bad).base.encrypted_amounts[index]
    else {
        panic!("expected compact amount");
    };
    amount[0] ^= 1;
    assert!(discover(receiver.view_pair(), &bad, 16).unwrap().is_empty());
    rejected_atomically(&state, &bad);
    let (_, inputs) = funded_fixture(&sender, &[100_000]).unwrap();
    assert!(create_payment(
        &sender,
        inputs,
        &receiver.address(),
        u64::MAX,
        context(&state)
    )
    .is_err());
}

#[test]
fn forged_signature_range_proof_and_wrong_ownership_fail() {
    let (sender, _, state, good) = payment(&[100_000]);
    let mut bad = good.clone();
    let RctPrunable::Clsag { clsags, .. } = &mut proofs(&mut bad).prunable else {
        panic!()
    };
    clsags[0].c1 = Scalar::ZERO;
    rejected_atomically(&state, &bad);
    let mut bad = good.clone();
    let RctPrunable::Clsag { bulletproof, .. } = &mut proofs(&mut bad).prunable else {
        panic!()
    };
    *bulletproof = monero_wallet::ringct::bulletproofs::Bulletproof::prove_plus(
        &mut rand_core::OsRng,
        vec![
            Commitment::new(Scalar::ONE, 5),
            Commitment::new(Scalar::ONE, 6),
        ],
    )
    .unwrap();
    rejected_atomically(&state, &bad);
    let (_, inputs) = funded_fixture(&sender, &[100_000]).unwrap();
    assert!(create_payment(
        &Wallet::generate(0),
        inputs,
        &sender.address(),
        1_000,
        context(&state)
    )
    .is_err());
}

#[test]
fn duplicate_inputs_alternate_spends_and_ring_substitution_fail() {
    let sender = Wallet::generate(0);
    let receiver = Wallet::generate(0);
    let (mut state, inputs) = funded_fixture(&sender, &[100_000]).unwrap();
    let make = |amount| {
        create_payment(
            &sender,
            inputs.clone(),
            &receiver.address(),
            amount,
            context(&state),
        )
        .unwrap()
    };
    let first = make(20_000);
    let second = make(30_000);
    let a = &first.transaction.prefix().inputs[0];
    let b = &second.transaction.prefix().inputs[0];
    let (Input::ToKey { key_image: a, .. }, Input::ToKey { key_image: b, .. }) = (a, b) else {
        panic!()
    };
    assert_eq!(
        a, b,
        "nullifier remains identical across different outputs/amounts"
    );
    assert!(create_payment(
        &sender,
        vec![inputs[0].clone(), inputs[0].clone()],
        &receiver.address(),
        150_000,
        context(&state)
    )
    .is_err());
    let mut duplicate = first.clone();
    let copy = duplicate.transaction.prefix().inputs[0].clone();
    duplicate.transaction.prefix_mut().inputs.push(copy);
    assert_eq!(state.verify(&duplicate, 1), Err(Error::Spent));
    let mut bad = first.clone();
    let Input::ToKey { key_offsets, .. } = &mut bad.transaction.prefix_mut().inputs[0] else {
        panic!()
    };
    key_offsets[0] = u64::MAX;
    rejected_atomically(&state, &bad);
    state.apply(&first, 1).unwrap();
    assert_eq!(state.apply(&second, 1), Err(Error::Spent));
}

#[test]
fn truncation_trailing_oversized_and_random_packets_fail_closed() {
    let (_, _, _, good) = payment(&[100_000]);
    let bytes = good.encode();
    for length in 0..bytes.len() {
        assert!(Packet::decode(&bytes[..length]).is_err());
    }
    let mut trailing = bytes;
    trailing.push(0);
    assert!(Packet::decode(&trailing).is_err());
    assert!(matches!(
        Packet::decode(&vec![0; MAX_PACKET_BYTES + 1]),
        Err(Error::Limit)
    ));
    // Fixed malformed inputs exercise format bounds, not cryptographic authorization.
    for length in [0, 1, 32, 128, 1024, MAX_PACKET_BYTES] {
        assert!(Packet::decode(&vec![255; length]).is_err());
    }
}
