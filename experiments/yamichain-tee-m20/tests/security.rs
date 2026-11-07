use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use x25519_dalek::{PublicKey, StaticSecret};
use yamichain_tee_m20::*;
fn wallet() -> SigningKey {
    SigningKey::generate(&mut OsRng)
}
fn state(a: &SigningKey) -> State {
    State {
        economics: None,

        chain: 20,
        epoch: 0,
        accounts: BTreeMap::from([(
            a.verifying_key().to_bytes(),
            Account {
                balance: 1000,
                nonce: 0,
            },
        )]),
    }
}
fn tx(a: &SigningKey, b: &SigningKey, nonce: u64, amount: u64) -> Signed {
    Signed::sign(
        Transfer {
            finance: None,

            sender: a.verifying_key().to_bytes(),
            recipient: b.verifying_key().to_bytes(),
            amount,
            nonce,
            chain: 20,
        },
        a,
    )
}
fn payment(e: &Engine, t: &Signed) -> Envelope {
    encrypt(e.public(), &bytes(t), b"YAMI-M20-payment-v1").unwrap()
}
#[test]
fn offline_receive_query_recovery_and_host_attacks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts");
    let a = wallet();
    let b = wallet();
    let signer = wallet();
    let transport = StaticSecret::random_from_rng(OsRng);
    let storage = [9; 32];
    let mut e =
        Engine::create(&path, state(&a), storage, signer.clone(), transport.clone()).unwrap();
    let genesis = e.head();
    let backup = std::fs::read(&path).unwrap();
    assert!(Engine::load(&path, storage, signer.clone(), transport.clone(), &genesis).is_err());
    let p = payment(&e, &tx(&a, &b, 0, 100));
    let mut malformed = p.clone();
    malformed.ciphertext[0] ^= 1;
    assert!(e.execute(&malformed).is_err());
    assert_eq!(e.head(), genesis);
    let r = e.execute(&p).unwrap();
    r.verify(e.receipt_key(), &genesis).unwrap();
    assert!(r
        .verify(wallet().verifying_key().to_bytes(), &genesis)
        .is_err());
    assert!(r.verify(e.receipt_key(), &r.next).is_err());
    assert!(e.execute(&p).is_err());
    let mut forged = r.clone();
    forged.next.commitment = [1; 32];
    assert!(forged.verify(e.receipt_key(), &genesis).is_err());
    let reply = StaticSecret::random_from_rng(OsRng);
    let mut q = Query {
        account: b.verifying_key().to_bytes(),
        reply_key: PublicKey::from(&reply).to_bytes(),
        head: e.head(),
        challenge: [7; 32],
        signature: vec![],
    };
    q.sign(&a);
    assert!(e.query(&q).is_err());
    q.sign(&b);
    let encrypted = e.query(&q).unwrap();
    let response: QueryResponse =
        serde_json::from_slice(&decrypt(&reply, &encrypted, &q.message()).unwrap()).unwrap();
    response.verify(e.receipt_key(), &q).unwrap();
    assert_eq!(response.account.balance, 100);
    let mut fake = response.clone();
    fake.account.balance += 1;
    assert!(fake.verify(e.receipt_key(), &q).is_err());
    let mut stale = q.clone();
    stale.challenge = [8; 32];
    assert!(response.verify(e.receipt_key(), &stale).is_err());
    let latest = e.head();
    drop(e);
    // Interrupted pre-rename write does not replace the authoritative snapshot.
    std::fs::write(path.with_extension("pending"), b"torn write").unwrap();
    let e = Engine::load(&path, storage, signer.clone(), transport.clone(), &latest).unwrap();
    assert_eq!(e.head(), latest);
    drop(e);
    assert!(Engine::load(&path, [8; 32], signer.clone(), transport.clone(), &latest).is_err());
    let current = std::fs::read(&path).unwrap();
    std::fs::write(&path, backup).unwrap();
    assert!(Engine::load(&path, storage, signer.clone(), transport.clone(), &latest).is_err());
    // A host that also rewinds the external anchor defeats rollback detection.
    assert!(Engine::load(&path, storage, signer.clone(), transport.clone(), &genesis).is_ok());
    std::fs::write(&path, &current).unwrap();
    let mut corrupted = current;
    let n = corrupted.len() / 2;
    corrupted[n] ^= 1;
    std::fs::write(&path, corrupted).unwrap();
    assert!(Engine::load(&path, storage, signer, transport, &latest).is_err());
}
#[test]
fn authorization_ranges_and_atomicity() {
    let a = wallet();
    let b = wallet();
    let s = state(&a);
    for amount in [0, 1001, u64::MAX] {
        assert!(transition(&s, &tx(&a, &b, 0, amount)).is_err());
    }
    let mut wrong = tx(&a, &b, 0, 1);
    wrong.transfer.chain = 21;
    assert!(transition(&s, &wrong).is_err());
    assert!(transition(&s, &tx(&a, &b, 1, 1)).is_err());
    let mut overflow = s.clone();
    overflow.accounts.insert(
        b.verifying_key().to_bytes(),
        Account {
            balance: u64::MAX,
            nonce: 0,
        },
    );
    assert!(transition(&overflow, &tx(&a, &b, 0, 1)).is_err());
    assert_eq!(
        overflow.accounts[&a.verifying_key().to_bytes()].balance,
        1000
    );
    let mut exhausted = s.clone();
    exhausted
        .accounts
        .get_mut(&a.verifying_key().to_bytes())
        .unwrap()
        .nonce = u64::MAX;
    assert!(transition(&exhausted, &tx(&a, &b, u64::MAX, 1)).is_err());
    let mut selfpay = tx(&a, &a, 0, 1);
    assert!(transition(&s, &selfpay).is_err());
    selfpay.signature[0] ^= 1;
    assert!(selfpay.verify().is_err());
}
#[test]
fn concurrent_double_spend_serializes() {
    let d = tempfile::tempdir().unwrap();
    let a = wallet();
    let b = wallet();
    let e = Engine::create(
        &d.path().join("db"),
        state(&a),
        [4; 32],
        wallet(),
        StaticSecret::random_from_rng(OsRng),
    )
    .unwrap();
    let p = payment(&e, &tx(&a, &b, 0, 700));
    let shared = Arc::new(Mutex::new(e));
    let jobs: Vec<_> = (0..8)
        .map(|_| {
            let e = shared.clone();
            let p = p.clone();
            std::thread::spawn(move || e.lock().unwrap().execute(&p).is_ok())
        })
        .collect();
    assert_eq!(
        jobs.into_iter()
            .map(|j| usize::from(j.join().unwrap()))
            .sum::<usize>(),
        1
    );
}
#[test]
fn compromised_enclave_can_forge_receipt_but_disclosed_auditor_rejects() {
    let a = wallet();
    let b = wallet();
    let s = state(&a);
    let t = tx(&a, &b, 0, 10);
    let valid = transition(&s, &t).unwrap();
    audit_disclosed(&s, &t, &valid).unwrap();
    for theft in [false, true] {
        let mut forged = valid.clone();
        if theft {
            forged
                .accounts
                .get_mut(&a.verifying_key().to_bytes())
                .unwrap()
                .balance -= 100;
            forged
                .accounts
                .get_mut(&b.verifying_key().to_bytes())
                .unwrap()
                .balance += 100;
        } else {
            forged
                .accounts
                .get_mut(&b.verifying_key().to_bytes())
                .unwrap()
                .balance += 100;
        }
        assert!(audit_disclosed(&s, &t, &forged).is_err());
    }
    // Receipt signing authority is enough to authenticate an arbitrary false commitment.
    use ed25519_dalek::Signer;
    let signer = wallet();
    let old = Head {
        chain: 20,
        epoch: 0,
        commitment: [0; 32],
    };
    let mut r = Receipt {
        previous: old.clone(),
        next: Head {
            chain: 20,
            epoch: 1,
            commitment: [99; 32],
        },
        request: [5; 32],
        signature: vec![],
    };
    r.signature = signer
        .sign(&bytes(&(
            "YAMI-M20-SOFTWARE-receipt-v1",
            &r.previous,
            &r.next,
            r.request,
        )))
        .to_bytes()
        .to_vec();
    r.verify(signer.verifying_key().to_bytes(), &old).unwrap();
}
#[test]
fn attestation_policy_is_simulation_and_tdx_fails_closed() {
    let e = SoftwareEvidence {
        measurement: [1; 32],
        identity: [2; 32],
        key: [3; 32],
        challenge: [4; 32],
        issued: 100,
        expires: 150,
    };
    assert!(verify_software(&e, [1; 32], [2; 32], [3; 32], [4; 32], 120).is_ok());
    for i in 0..6 {
        let mut bad = e.clone();
        match i {
            0 => bad.measurement = [9; 32],
            1 => bad.identity = [9; 32],
            2 => bad.key = [9; 32],
            3 => bad.challenge = [9; 32],
            4 => bad.issued = 130,
            _ => bad.expires = 110,
        };
        assert!(verify_software(&bad, [1; 32], [2; 32], [3; 32], [4; 32], 120).is_err());
    }
    assert!(verify_tdx_quote(&bytes(&e)).is_err());
    assert!(verify_tdx_quote(&[]).is_err());
}
#[test]
fn interrupted_commit_fails_closed_at_each_durability_boundary() {
    for stage in ["synced_pending", "renamed", "synced_directory"] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let a = wallet();
        let b = wallet();
        let sk = wallet();
        let transport = StaticSecret::random_from_rng(OsRng);
        let mut e =
            Engine::create(&path, state(&a), [7; 32], sk.clone(), transport.clone()).unwrap();
        let old = e.head();
        let p = payment(&e, &tx(&a, &b, 0, 10));
        assert!(e
            .execute_with_fault(&p, |point| if point == stage {
                Err("simulated crash".into())
            } else {
                Ok(())
            })
            .is_err());
        assert!(e.execute(&p).is_err());
        drop(e);
        if stage == "synced_pending" {
            assert!(Engine::load(&path, [7; 32], sk, transport, &old).is_ok());
        } else {
            assert!(Engine::load(&path, [7; 32], sk.clone(), transport.clone(), &old).is_err());
            // Harness models a separately finalized new head. Do not trust the file's head in deployment.
            let v: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let receipt: Receipt = serde_json::from_value(v["receipt"].clone()).unwrap();
            receipt.verify(sk.verifying_key().to_bytes(), &old).unwrap();
            assert!(Engine::load(&path, [7; 32], sk, transport, &receipt.next).is_ok());
        }
    }
}
#[test]
fn concurrent_independent_senders_credit_offline_recipient() {
    let d = tempfile::tempdir().unwrap();
    let senders: Vec<_> = (0..8).map(|_| wallet()).collect();
    let recipient = wallet();
    let accounts = senders
        .iter()
        .map(|w| {
            (
                w.verifying_key().to_bytes(),
                Account {
                    balance: 10,
                    nonce: 0,
                },
            )
        })
        .collect();
    let e = Engine::create(
        &d.path().join("db"),
        State {
            economics: None,

            chain: 20,
            epoch: 0,
            accounts,
        },
        [7; 32],
        wallet(),
        StaticSecret::random_from_rng(OsRng),
    )
    .unwrap();
    let payments: Vec<_> = senders
        .iter()
        .map(|w| payment(&e, &tx(w, &recipient, 0, 10)))
        .collect();
    let shared = Arc::new(Mutex::new(e));
    let jobs: Vec<_> = payments
        .into_iter()
        .map(|p| {
            let e = shared.clone();
            std::thread::spawn(move || e.lock().unwrap().execute(&p).is_ok())
        })
        .collect();
    assert!(jobs.into_iter().all(|j| j.join().unwrap()));
    let e = shared.lock().unwrap();
    assert_eq!(e.head().epoch, 8);
    let reply = StaticSecret::random_from_rng(OsRng);
    let mut q = Query {
        account: recipient.verifying_key().to_bytes(),
        reply_key: PublicKey::from(&reply).to_bytes(),
        head: e.head(),
        challenge: [5; 32],
        signature: vec![],
    };
    q.sign(&recipient);
    let response: QueryResponse =
        serde_json::from_slice(&decrypt(&reply, &e.query(&q).unwrap(), &q.message()).unwrap())
            .unwrap();
    response.verify(e.receipt_key(), &q).unwrap();
    assert_eq!(response.account.balance, 80);
}
#[test]
fn envelope_key_domain_and_payload_substitution_rejected() {
    let secret = StaticSecret::random_from_rng(OsRng);
    let public = PublicKey::from(&secret).to_bytes();
    assert!(encrypt([0; 32], b"payload", b"context").is_err());
    let mut e = encrypt(public, b"payload", b"context").unwrap();
    assert!(decrypt(&secret, &e, b"other-domain").is_err());
    e.ephemeral = [0; 32];
    assert!(decrypt(&secret, &e, b"context").is_err());
    let a = wallet();
    let b = wallet();
    let mut forged = tx(&a, &b, 0, 1);
    forged.transfer.amount = 999;
    assert!(transition(&state(&a), &forged).is_err());
}
