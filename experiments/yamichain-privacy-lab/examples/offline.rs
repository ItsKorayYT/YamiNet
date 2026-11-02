use std::time::Instant;
use yamichain_privacy_lab::{
    create_payment, discover, funded_fixture, wallet::Wallet, with_decoys, Context, Error,
};

fn main() {
    let amounts = match std::env::args().nth(1).as_deref() {
        None | Some("2") => vec![60_000, 40_000],
        Some("1") => vec![100_000],
        _ => panic!("usage: offline [1|2]"),
    };
    let sender = Wallet::generate(0);
    let recipient = Wallet::generate(0);
    let (mut state, inputs) = funded_fixture(&sender, &amounts).unwrap();
    let start = Instant::now();
    let packet = create_payment(
        &sender,
        inputs,
        &recipient.address(),
        25_000,
        Context {
            intent: [1; 32],
            anchor: state.anchor(),
            expiry: 10,
            fee_epoch: 0,
            fee: 0,
        },
    )
    .unwrap();
    let create_us = start.elapsed().as_micros();
    let start = Instant::now();
    state.verify(&packet, 1).unwrap();
    let verify_us = start.elapsed().as_micros();
    let first = state.apply(&packet, 1).unwrap();
    assert_eq!(state.apply(&packet, 1), Err(Error::Spent));
    let start = Instant::now();
    let received = discover(recipient.view_pair(), &packet, first).unwrap();
    let scan_us = start.elapsed().as_micros();
    assert_eq!(received[0].commitment().amount, 25_000);
    let backup_key = age::x25519::Identity::generate();
    let archive = recipient.backup(&backup_key.to_public()).unwrap();
    let recovered = Wallet::restore(&archive, &backup_key).unwrap();
    let restored = discover(recovered.view_pair(), &packet, first).unwrap();
    let onward = create_payment(
        &recovered,
        vec![with_decoys(&state, &restored[0]).unwrap()],
        &sender.address(),
        10_000,
        Context {
            intent: [2; 32],
            anchor: state.anchor(),
            expiry: 10,
            fee_epoch: 0,
            fee: 0,
        },
    )
    .unwrap();
    state.apply(&onward, 2).unwrap();
    let monero_wallet::transaction::Transaction::V2 {
        proofs: Some(proofs),
        ..
    } = &packet.transaction
    else {
        panic!()
    };
    println!("offline_only=true inputs={} outputs=2 ring_size=16 packet_bytes={} proof_bytes={} create_prove_sign_us={} verify_us={} scan_us={} recovered_spend=true double_spend_rejected=true",
        amounts.len(), packet.encode().len(), proofs.prunable.serialize(proofs.rct_type()).len(), create_us, verify_us, scan_us);
}
