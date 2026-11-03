use super::*;
use ark_ff::{BigInteger, PrimeField};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystem};
use hmac::Mac;
use std::sync::OnceLock;
fn keys() -> &'static (Prover, Verifier, Arc<TreeParams>, usize) {
    static KEYS: OnceLock<(Prover, Verifier, Arc<TreeParams>, usize)> = OnceLock::new();
    KEYS.get_or_init(|| setup().expect("real Groth16 setup and satisfied sample"))
}
fn fixed_output(o: &Opening, rs: [Scalar; 3]) -> Output {
    Output {
        commitment: o.commitment(),
        delivery: [
            encrypt(
                o.view,
                (Group::generator() * Scalar::from(u64::from(o.amount))).into_affine(),
                rs[0],
            ),
            encrypt(o.view, o.salt, rs[1]),
            encrypt(o.view, o.owner, rs[2]),
        ],
    }
}
fn raw_nullifier(key: &[u8], c: Digest) -> Digest {
    let mut m = hmac::Hmac::<sha2::Sha256>::new_from_slice(key).unwrap();
    m.update(b"YPP2-nullifier-v1");
    m.update(&c);
    m.finalize().into_bytes().into()
}
fn sat(r: relation::Relation) -> bool {
    let cs = ConstraintSystem::<Fr>::new_ref();
    r.generate_constraints(cs.clone()).unwrap();
    cs.is_satisfied().unwrap()
}

#[test]
fn relation_rejects_bad_membership_delivery_authority_inflation_and_scalar_aliases() {
    let (_, _, params, _) = keys();
    let w = Wallet::new();
    let b = Wallet::new();
    let (g, states) = synthetic_genesis(params.clone(), &[&w, &b], &[100, 100]).unwrap();
    let ledger = Ledger::from_genesis(&g).unwrap();
    let (r, _) = statement(
        params.clone(),
        &w,
        &ledger.snapshot(),
        &states[0],
        None,
        &b.address(),
        1,
    )
    .unwrap();
    assert!(sat(r.clone()));
    let mut bad = r.clone();
    bad.witnesses[0].index ^= 1;
    assert!(!sat(bad));
    let mut bad = r.clone();
    bad.witnesses[0].siblings[0][0] ^= 1;
    assert!(!sat(bad));
    let mut bad = r.clone();
    bad.spend = scalar_bytes(&scalar());
    assert!(!sat(bad));
    let mut bad = r.clone();
    bad.new.amount += 1;
    bad.tx.outputs[0] = fixed_output(&bad.new, bad.randomness[0]);
    assert!(!sat(bad));
    let mut bad = r.clone();
    bad.credit.amount = 1;
    bad.new.amount += 1;
    bad.tx.outputs[0] = fixed_output(&bad.new, bad.randomness[0]);
    bad.tx.nullifiers[1] = nf(&w.view, bad.credit.commitment());
    assert!(!sat(bad)); // dummy cannot mint
    let mut bad = r.clone();
    bad.tx.outputs[1].delivery[0] = encrypt(
        w.address().view,
        (Group::generator() * Scalar::from(1u64)).into_affine(),
        bad.randomness[1][0],
    );
    assert!(!sat(bad));
    let mut bad = r.clone();
    bad.tx.outputs[1].delivery[1] = encrypt(b.address().view, point(), bad.randomness[1][1]);
    assert!(!sat(bad));
    let mut bad = r.clone();
    bad.tx.outputs[1].delivery[2] =
        encrypt(b.address().view, w.address().owner, bad.randomness[1][2]);
    assert!(!sat(bad));
    let mut bad = r.clone();
    bad.randomness[1][1] = bad.randomness[1][0];
    bad.tx.outputs[1].delivery[1] =
        encrypt(b.address().view, bad.payment.salt, bad.randomness[1][1]);
    assert!(!sat(bad)); // mask reuse forbidden
    let mut alias = w.view.into_bigint();
    alias.add_with_carry(&Scalar::MODULUS);
    let mut bad = r.clone();
    bad.view = alias.to_bytes_le().try_into().unwrap();
    bad.tx.nullifiers = [
        raw_nullifier(&bad.view, bad.old.commitment()),
        raw_nullifier(&bad.view, bad.credit.commitment()),
    ];
    assert!(!sat(bad)); // same EC key, different HMAC key is forbidden
    let mut bad = r;
    bad.live = true;
    assert!(!sat(bad)); // real credit requires membership
}

#[test]
fn real_proofs_hide_access_receive_offline_survive_concurrency_and_recover() {
    let (p, v, params, _) = keys();
    let wallets: Vec<_> = (0..5).map(|_| Wallet::new()).collect();
    let refs: Vec<_> = wallets.iter().collect();
    let (g, states) = synthetic_genesis(params.clone(), &refs, &[100; 5]).unwrap();
    let mut ledger = Ledger::from_genesis(&g).unwrap();
    let initial = ledger.snapshot();
    assert!(matches!(
        p.transfer(
            &wallets[0],
            &initial,
            &states[0],
            None,
            &wallets[1].address(),
            101
        ),
        Err(Error::Overflow)
    ));
    let d = Decoder::default();
    let mut scans: Vec<_> = wallets
        .iter()
        .map(|w| Scanner::new(&w.viewer(), &g, g.checkpoint(), &d).unwrap())
        .collect();
    let mut prepared = Vec::new();
    let mut next_states = Vec::new();
    // Four independent wallets build against one root, then ALL apply unchanged.
    for i in 0..4 {
        let (t, s) = p
            .transfer(
                &wallets[i],
                &initial,
                &states[i],
                None,
                &wallets[(i + 1) % 4].address(),
                3,
            )
            .unwrap();
        v.verify(&t).unwrap();
        assert_eq!(t.anchor, initial.root());
        assert_eq!(t.encode().len(), PACKET_BYTES);
        let decoded = Transaction::decode(&t.encode()).unwrap();
        v.verify(&decoded).unwrap();
        prepared.push(t);
        next_states.push(s);
    }
    let (conflict, _) = p
        .transfer(
            &wallets[0],
            &initial,
            &states[0],
            None,
            &wallets[2].address(),
            4,
        )
        .unwrap();
    v.verify(&conflict).unwrap();
    let (expiring, _) = p
        .transfer(
            &wallets[4],
            &initial,
            &states[4],
            None,
            &wallets[1].address(),
            1,
        )
        .unwrap();
    v.verify(&expiring).unwrap();
    for (i, t) in prepared.iter().enumerate() {
        if i == 0 {
            let head = ledger.head();
            let mut bad = t.clone();
            bad.outputs[1].delivery[0].1 = point();
            assert!(ledger.apply(v, bad).is_err());
            assert_eq!(ledger.head(), head);
        }
        ledger.apply(v, t.clone()).unwrap();
        if i == 0 {
            assert_eq!(ledger.apply(v, t.clone()), Err(Error::Spent));
            assert_eq!(ledger.apply(v, conflict.clone()), Err(Error::Spent));
        }
        for (j, scan) in scans.iter_mut().enumerate() {
            scan.advance(
                &wallets[j].viewer(),
                v,
                ledger.records().last().unwrap(),
                &d,
            )
            .unwrap();
        }
    }
    assert_eq!(ledger.apply(v, expiring), Err(Error::Expired)); // valid, unspent proof; root expired
                                                                // Bob discovers an offline payment and absorbs it while paying Carol.
    assert_eq!(scans[1].credits()[0].amount(), 3);
    let bob_credit = scans[1].credits()[0].clone();
    let (absorb, bob_state) = p
        .transfer(
            &wallets[1],
            &ledger.snapshot(),
            &next_states[1],
            Some(&bob_credit),
            &wallets[2].address(),
            2,
        )
        .unwrap();
    v.verify(&absorb).unwrap();
    ledger.apply(v, absorb.clone()).unwrap();
    for (j, scan) in scans.iter_mut().enumerate() {
        scan.advance(
            &wallets[j].viewer(),
            v,
            ledger.records().last().unwrap(),
            &d,
        )
        .unwrap();
    }
    assert_eq!(bob_state.amount(), 98);
    assert_eq!(
        scans[1].vault().unwrap().commitment(),
        bob_state.commitment()
    );
    assert!(scans[1].credits().is_empty());
    // Repeat Alice payment. Public output hashes never become public spend IDs.
    let (repeat, _) = p
        .transfer(
            &wallets[0],
            &ledger.snapshot(),
            &next_states[0],
            None,
            &wallets[1].address(),
            5,
        )
        .unwrap();
    v.verify(&repeat).unwrap();
    ledger.apply(v, repeat.clone()).unwrap();
    assert_ne!(prepared[0].nullifiers, repeat.nullifiers);
    assert!(!absorb
        .nullifiers
        .contains(&prepared[0].outputs[1].commitment));
    assert!(!repeat
        .nullifiers
        .contains(&prepared[0].outputs[0].commitment));
    let mut identifiers = BTreeSet::new();
    for record in ledger.records() {
        for n in record.transaction.nullifiers {
            assert!(identifiers.insert(n));
        }
        for o in &record.transaction.outputs {
            assert!(identifiers.insert(o.commitment));
        }
    }
    // No old input commitments or recipient public keys are serialized in packets.
    for (i, t) in prepared.iter().enumerate() {
        assert!(!t.encode().windows(32).any(|w| w == states[i].commitment()));
        for w in &wallets {
            assert!(!t
                .encode()
                .windows(32)
                .any(|b| b == bytes(&w.address().owner)));
            assert!(!t
                .encode()
                .windows(32)
                .any(|b| b == bytes(&w.address().view)));
        }
    }
    // Every output field is proof-bound, including all three encrypted messages.
    for i in 0..8 {
        let mut bad = repeat.clone();
        match i {
            0 => bad.anchor += Fr::from(1u64),
            1 => bad.nullifiers[0][0] ^= 1,
            2 => bad.outputs[1].commitment[0] ^= 1,
            3 => bad.outputs[1].delivery[0].1 = point(),
            4 => bad.outputs[1].delivery[1].1 = point(),
            5 => bad.outputs[1].delivery[2].1 = point(),
            6 => bad.outputs[0].delivery[1].1 = point(),
            _ => bad.proof = Proof::default(),
        };
        assert!(v.verify(&bad).is_err());
    }
    let packet = repeat.encode();
    for n in [0, 1, 100, PACKET_BYTES - 1] {
        assert!(Transaction::decode(&packet[..n]).is_err());
    }
    let mut trailing = packet.clone();
    trailing.push(0);
    assert!(Transaction::decode(&trailing).is_err());
    let mut wrong = packet;
    wrong[0] ^= 1;
    assert!(Transaction::decode(&wrong).is_err());
    let vk = v.encode();
    assert!(Verifier::decode(&vk).is_ok());
    let mut malformed = vk;
    malformed[336..344].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(Verifier::decode(&malformed).is_err());
    // Encrypted backup restore and interrupted authenticated replay.
    let identity = age::x25519::Identity::generate();
    let backup = wallets[1].backup(&identity.to_public()).unwrap();
    let restored = Wallet::restore(&identity, &backup).unwrap();
    assert!(Wallet::restore(&age::x25519::Identity::generate(), &backup).is_err());
    let viewer = restored.viewer();
    let mut recovering = Scanner::new(&viewer, &g, g.checkpoint(), &d).unwrap();
    recovering
        .advance(&viewer, v, &ledger.records()[0], &d)
        .unwrap();
    let cursor = recovering.cursor();
    let mut bad = ledger.records()[1].clone();
    bad.head[0] ^= 1;
    assert_eq!(
        recovering.advance(&viewer, v, &bad, &d),
        Err(Error::Recovery)
    );
    assert_eq!(recovering.cursor(), cursor);
    assert!(recovering.require_head(ledger.head()).is_err());
    for record in &ledger.records()[1..] {
        recovering.advance(&viewer, v, record, &d).unwrap();
    }
    recovering.require_head(ledger.head()).unwrap();
    assert_eq!(recovering.vault().unwrap().amount(), 98);
    assert_eq!(recovering.credits()[0].amount(), 5);
    assert!(Scanner::new(&viewer, &g, [0; 32], &d).is_err());
    let mut corrupted = backup;
    let last = corrupted.len() - 1;
    corrupted[last] ^= 1;
    assert!(Wallet::restore(&identity, &corrupted).is_err());
    // A view-only scanner knows spent markers but lacks spending authority.
    assert!(matches!(
        p.transfer(
            &wallets[4],
            &ledger.snapshot(),
            recovering.vault().unwrap(),
            None,
            &wallets[0].address(),
            1
        ),
        Err(Error::Authorization)
    ));
}

#[test]
fn recovery_decodes_every_opening_and_surfaces_misaddressing() {
    let a = Wallet::new();
    let b = Wallet::new();
    let d = Decoder::default();
    for amount in [0, 1, 255, 256, u16::MAX] {
        let o = a.opening(amount, true);
        let (ct, _) = Output::create(&o);
        let decoded = ct.recover(a.view, true, &d).unwrap().unwrap();
        assert_eq!(decoded.commitment(), o.commitment());
        assert_eq!(decoded.amount(), amount);
        assert!(ct.recover(b.view, true, &d).unwrap().is_none());
    }
    let o = Opening {
        owner: b.address().owner,
        view: a.address().view,
        amount: 7,
        salt: point(),
        credit: true,
    };
    let (out, _) = Output::create(&o);
    let recovered = out.recover(a.view, true, &d).unwrap().unwrap();
    assert_eq!(recovered.owner, b.address().owner);
    assert_ne!(recovered.owner, a.address().owner); // full plaintext, not silent loss
}

#[test]
fn stale_in_place_root_replacement_loses_an_unrelated_update() {
    // Architectural counterexample, not a claimed ZK proof or impossibility
    // theorem: two locally computed in-place branches cannot be applied by
    // replacing the root with the second branch's root after accepting the first.
    let params = Arc::new(TreeHash::setup(&mut OsRng).unwrap());
    let a = Wallet::new();
    let b = Wallet::new();
    let (g, states) = synthetic_genesis(params, &[&a, &b], &[100, 100]).unwrap();
    let initial = Ledger::from_genesis(&g).unwrap().tree;
    let mut alice = initial.clone();
    let mut bob = initial.clone();
    let mut merged = initial.clone();
    let next_a = Opening {
        salt: point(),
        amount: 99,
        ..states[0].clone()
    };
    let next_b = Opening {
        salt: point(),
        amount: 99,
        ..states[1].clone()
    };
    alice.leaves[0] = next_a.commitment();
    bob.leaves[1] = next_b.commitment();
    merged.leaves[0] = next_a.commitment();
    merged.leaves[1] = next_b.commitment();
    assert_ne!(alice.root(), initial.root());
    assert_ne!(bob.root(), initial.root());
    assert_ne!(bob.root(), merged.root());
    assert_eq!(bob.leaves[0], states[0].commitment()); // Alice's update was lost.
                                                       // In the implemented accumulator, append operations compose publicly; the
                                                       // real-proof concurrency test checks hidden inputs against retained roots.
}
