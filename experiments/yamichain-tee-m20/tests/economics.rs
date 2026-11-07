use ed25519_dalek::{Signer, SigningKey};
use std::collections::BTreeMap;
use yamichain_tee_m20::{economics::*, *};
fn key(n: u8) -> SigningKey {
    SigningKey::from_bytes(&[n; 32])
}
fn id(n: u8) -> Id {
    key(n).verifying_key().to_bytes()
}
fn state() -> State {
    let mut s = State {
        chain: 24,
        epoch: 0,
        accounts: BTreeMap::from([
            (
                id(1),
                Account {
                    balance: 1000,
                    nonce: 0,
                },
            ),
            (
                id(2),
                Account {
                    balance: 1000,
                    nonce: 0,
                },
            ),
        ]),
        economics: None,
    };
    s.enable_economics(Policy {
        base_fee: 1,
        unit_fee: 1,
        max_units: 16,
        minimum_stake: 100,
        unbonding_blocks: 5,
        epoch_blocks: 10,
        active_limit: 4,
        fee_recipients: vec![id(10), id(11)],
    })
    .unwrap();
    s
}
fn tx(s: &State, owner: u8, recipient: Id, amount: u64, action: Action) -> Signed {
    Signed::sign(
        Transfer {
            sender: id(owner),
            recipient,
            amount,
            nonce: s.accounts[&id(owner)].nonce,
            chain: 24,
            finance: Some(Intent {
                fee_limit: 100,
                action,
            }),
        },
        &key(owner),
    )
}
fn registration(s: &State) -> Action {
    let msg = registration_message(24, id(1), s.accounts[&id(1)].nonce, id(3), id(4));
    Action::Register {
        consensus: id(3),
        transport: id(4),
        consensus_proof: key(3).sign(&msg).to_bytes().to_vec(),
        transport_proof: key(4).sign(&msg).to_bytes().to_vec(),
    }
}
fn run(s: &State, owner: u8, recipient: Id, amount: u64, action: Action) -> State {
    let next = transition(s, &tx(s, owner, recipient, amount, action)).unwrap();
    next.economics
        .as_ref()
        .unwrap()
        .invariant(&next.accounts)
        .unwrap();
    next
}
#[test]
fn encrypted_ledger_staking_delegation_fees_exit_and_delayed_withdrawal_conserve_supply() {
    let mut s = state();
    s = run(&s, 1, id(3), 0, registration(&s));
    s = run(&s, 1, id(3), 400, Action::Bond { validator: id(3) });
    s = run(&s, 2, id(3), 200, Action::Bond { validator: id(3) });
    assert_eq!(
        s.economics.as_ref().unwrap().eligible().unwrap(),
        vec![(id(3), id(4), 600)]
    );
    s = run(&s, 1, id(3), 300, Action::Unbond { validator: id(3) });
    let ticket = 2;
    let before = s.clone();
    assert!(transition(&s, &tx(&s, 1, id(1), 0, Action::Withdraw { ticket })).is_err());
    assert_eq!(s, before);
    for _ in 0..5 {
        s = run(&s, 2, id(1), 1, Action::Transfer);
    }
    s = run(&s, 1, id(1), 0, Action::Withdraw { ticket });
    assert!(s.economics.as_ref().unwrap().tickets.is_empty());
    let before = s.clone();
    assert!(transition(&s, &tx(&s, 1, id(1), 0, Action::Withdraw { ticket })).is_err());
    assert_eq!(s, before);
    let fee = s.economics.as_ref().unwrap().fees_collected;
    assert_eq!(
        s.accounts[&id(10)].balance + s.accounts[&id(11)].balance,
        fee
    );
    s = run(&s, 1, id(3), 0, Action::Exit { validator: id(3) });
    assert!(s.economics.as_ref().unwrap().eligible().unwrap().is_empty());
    assert!(transition(&s, &tx(&s, 2, id(3), 1, Action::Bond { validator: id(3) })).is_err());
    s = run(&s, 2, id(3), 200, Action::Unbond { validator: id(3) });
    assert_eq!(s.economics.as_ref().unwrap().supply, 2000);
}
#[test]
fn fee_evasion_unauthorized_unstaking_overflow_and_signature_tampering_are_atomic() {
    let s = state();
    let mut t = tx(&s, 1, id(2), 100, Action::Transfer);
    t.transfer.finance.as_mut().unwrap().fee_limit = 1;
    t = Signed::sign(t.transfer, &key(1));
    assert!(transition(&s, &t).is_err());
    let mut t = tx(&s, 1, id(2), 100, Action::Transfer);
    t.transfer.amount += 1;
    assert!(transition(&s, &t).is_err());
    let mut t = tx(&s, 1, id(2), 100, Action::Transfer);
    t.transfer.finance = None;
    t = Signed::sign(t.transfer, &key(1));
    assert!(transition(&s, &t).is_err());
    let t = tx(&s, 1, id(2), u64::MAX, Action::Transfer);
    assert!(transition(&s, &t).is_err());
    let r = registration(&s);
    let s = run(&s, 1, id(3), 0, r);
    let s = run(&s, 1, id(3), 100, Action::Bond { validator: id(3) });
    for action in [
        Action::Unbond { validator: id(3) },
        Action::Exit { validator: id(3) },
        Action::Withdraw { ticket: 1 },
    ] {
        assert!(transition(&s, &tx(&s, 2, id(3), 100, action)).is_err());
    }
    let mut r = registration(&s);
    if let Action::Register {
        transport_proof, ..
    } = &mut r
    {
        transport_proof[0] ^= 1;
    }
    assert!(transition(&s, &tx(&s, 1, id(3), 0, r)).is_err());
    let mut invalid = state();
    invalid.economics.as_mut().unwrap().policy.unit_fee = u64::MAX;
    assert!(transition(&invalid, &tx(&invalid, 1, id(2), 1, Action::Transfer)).is_err());
    assert_eq!(s.economics.as_ref().unwrap().supply, 2000);
}
#[test]
fn old_state_encoding_and_signed_transfer_bytes_remain_unchanged() {
    let s = State {
        chain: 24,
        epoch: 0,
        accounts: BTreeMap::from([(
            id(1),
            Account {
                balance: 1000,
                nonce: 0,
            },
        )]),
        economics: None,
    };
    let t = Signed::sign(
        Transfer {
            sender: id(1),
            recipient: id(2),
            amount: 100,
            nonce: 0,
            chain: 24,
            finance: None,
        },
        &key(1),
    );
    assert!(!String::from_utf8(bytes(&t)).unwrap().contains("finance"));
    let next = transition(&s, &t).unwrap();
    assert_eq!(next.accounts[&id(1)].balance, 900);
    assert!(next.economics.is_none());
    use hkdf::Hkdf;
    use sha2::Sha256;
    let old_bytes = serde_json::to_vec(&(
        24u64,
        0u64,
        vec![(
            id(1),
            Account {
                balance: 1000,
                nonce: 0,
            },
        )],
    ))
    .unwrap();
    let mut expected = [0; 32];
    Hkdf::<Sha256>::new(Some(b"YAMI-M21-private-state-root-v1"), &[9; 32])
        .expand(&old_bytes, &mut expected)
        .unwrap();
    assert_eq!(
        yamichain_tee_m20::quorum::state_head(&s, [9; 32]).commitment,
        expected
    );

    let again = transition(&s, &t).unwrap();
    assert_eq!(next, again);
}
