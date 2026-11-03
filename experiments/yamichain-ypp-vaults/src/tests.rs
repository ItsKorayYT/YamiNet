use super::*;
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystem};
use std::sync::OnceLock;

fn keys() -> &'static (Prover, Verifier) {
    static KEYS: OnceLock<(Prover, Verifier)> = OnceLock::new();
    KEYS.get_or_init(|| setup().expect("real setup"))
}
fn satisfied(r: relation::Relation) -> bool {
    let cs = ConstraintSystem::<Fr>::new_ref();
    r.generate_constraints(cs.clone()).unwrap();
    cs.is_satisfied().unwrap()
}
#[test]
fn circuit_checks_authority_conservation_ranges_and_commitments() {
    let r = relation::blank();
    assert!(satisfied(r.clone()));
    let mut bad = r.clone();
    bad.secret[0] ^= 1;
    assert!(!satisfied(bad));
    let mut bad = r.clone();
    bad.new.balance += 2;
    bad.new_commitment = bad.new.commitment();
    assert!(!satisfied(bad));
    let mut bad = r.clone();
    bad.old_commitment[0] ^= 1;
    assert!(!satisfied(bad));
    let mut bad = r.clone();
    bad.new.view[0] ^= 1;
    bad.new_commitment = bad.new.commitment();
    assert!(!satisfied(bad));
    let mut bad = r.clone();
    bad.new.version = bad.old.version;
    bad.new_commitment = bad.new.commitment();
    assert!(!satisfied(bad));
    let mut bad = r.clone();
    bad.credit.amount = 0;
    bad.credit_commitment = bad.credit.commitment(bad.ciphertext_hash);
    assert!(!satisfied(bad));
    let mut bad = r.clone();
    bad.old.balance = 0;
    bad.new.balance = u64::MAX;
    bad.credit.amount = 1;
    bad.old_commitment = bad.old.commitment();
    bad.new_commitment = bad.new.commitment();
    bad.credit_commitment = bad.credit.commitment(bad.ciphertext_hash);
    assert!(!satisfied(bad));
    let mut bad = r.clone();
    bad.old.version = u64::MAX;
    bad.new.version = 0;
    bad.old_commitment = bad.old.commitment();
    bad.new_commitment = bad.new.commitment();
    assert!(!satisfied(bad));
    let mut bad = r.clone();
    bad.context[0] ^= 1;
    assert!(!satisfied(bad));
    let mut absorb = r.clone();
    absorb.debit = false;
    absorb.old.balance = 1;
    absorb.new.balance = 2;
    absorb.credit.owner = absorb.old.owner;
    absorb.credit.view = absorb.old.view;
    absorb.old_commitment = absorb.old.commitment();
    absorb.new_commitment = absorb.new.commitment();
    absorb.credit_commitment = absorb.credit.commitment(absorb.ciphertext_hash);
    assert!(satisfied(absorb.clone()));
    let mut bad = absorb.clone();
    bad.credit.owner[0] ^= 1;
    bad.credit_commitment = bad.credit.commitment(bad.ciphertext_hash);
    assert!(!satisfied(bad));
    let mut bad = absorb.clone();
    bad.credit.view[0] ^= 1;
    bad.credit_commitment = bad.credit.commitment(bad.ciphertext_hash);
    assert!(!satisfied(bad));
    let mut bad = absorb;
    bad.old.balance = u64::MAX;
    bad.new.balance = 0;
    bad.old_commitment = bad.old.commitment();
    bad.new_commitment = bad.new.commitment();
    assert!(!satisfied(bad));
}

#[test]
fn actual_proofs_offline_receive_replay_conflicts_transcript_and_recovery() {
    let (p, v) = keys();
    let encoded = v.encode().unwrap();
    assert!(Verifier::decode(&encoded).is_ok());
    assert!(Verifier::decode(&encoded[..encoded.len() - 1]).is_err());
    let mut malformed_vk = encoded;
    malformed_vk[336..344].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(Verifier::decode(&malformed_vk).is_err());
    let a = Wallet::new();
    let b = Wallet::new();
    let attacker = Wallet::new();
    assert_ne!(a.address().owner, b.address().owner);
    assert_ne!(*a.secret, *b.secret);
    assert_ne!(a.address().owner, a.address().view());
    let (g, [astate, bstate]) = synthetic_genesis(&a, &b, [100, 7]).unwrap();
    let mut ledger = Ledger::from_genesis(&g);
    assert!(matches!(
        p.debit(&attacker, &ledger, 0, &astate, &b.address(), 1),
        Err(Error::Authorization)
    ));
    assert!(matches!(
        p.debit(&a, &ledger, 0, &astate, &b.address(), 101),
        Err(Error::Overflow)
    ));
    assert!(matches!(
        p.debit(&a, &ledger, 0, &astate, &b.address(), 0),
        Err(Error::Invalid)
    ));
    // Bob supplies no secret, state witness, or network interaction for this debit.
    let (tx, astate2) = p.debit(&a, &ledger, 0, &astate, &b.address(), 13).unwrap();
    v.verify(&tx).unwrap();
    let packet = tx.encode();
    let decoded = Transaction::decode(&packet).unwrap();
    v.verify(&decoded).unwrap();
    assert_eq!(decoded.encode(), packet);
    for change in 0..9 {
        let mut forged = tx.clone();
        match change {
            0 => forged.old[0] ^= 1,
            1 => forged.new[0] ^= 1,
            2 => forged.credit[0] ^= 1,
            3 => forged.credit_ciphertext[0] ^= 1,
            4 => forged.state_ciphertext[0] ^= 1,
            5 => forged.anchor[0] ^= 1,
            6 => forged.slot = 1,
            7 => forged.debit = false,
            _ => forged.proof[0] ^= 1,
        }
        assert!(v.verify(&forged).is_err(), "unbound public field {change}");
    }
    for n in [0, 1, 10, 129, packet.len() - 1] {
        assert!(Transaction::decode(&packet[..n]).is_err());
    }
    let mut trailing = packet.clone();
    trailing.push(0);
    assert!(Transaction::decode(&trailing).is_err());
    let mut bad = packet.clone();
    bad[0] = 2;
    assert!(Transaction::decode(&bad).is_err());
    let mut bad = packet.clone();
    bad[162..166].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Transaction::decode(&bad).is_err());
    assert!(Transaction::decode(&vec![0; TX_BYTES + 1]).is_err());
    let before = ledger.head();
    let mut forged = tx.clone();
    forged.proof.fill(0);
    assert!(ledger.apply(v, forged).is_err());
    assert_eq!(ledger.head(), before);
    ledger.apply(v, tx.clone()).unwrap();
    assert_eq!(ledger.pending_count(), 1);
    let head = ledger.head();
    assert_eq!(ledger.apply(v, tx.clone()), Err(Error::StaleRoot));
    assert_eq!(ledger.head(), head);
    // A second fully valid proof against the old snapshot loses the race.
    let old_ledger = Ledger::from_genesis(&g);
    let (concurrent, _) = p
        .debit(&a, &old_ledger, 0, &astate, &attacker.address(), 1)
        .unwrap();
    v.verify(&concurrent).unwrap();
    assert_eq!(ledger.apply(v, concurrent), Err(Error::StaleRoot));
    let bobview = b.viewing_authority();
    let mut scan = Scanner::new(&bobview, &g, g.checkpoint()).unwrap();
    assert_eq!(scan.require_head(head), Err(Error::Recovery));
    let mut tampered = ledger.records()[0].clone();
    tampered.head[0] ^= 1;
    assert_eq!(scan.advance(&bobview, v, &tampered), Err(Error::Recovery));
    assert_eq!(scan.cursor(), 0);
    scan.advance(&bobview, v, &ledger.records()[0]).unwrap();
    scan.require_head(head).unwrap();
    assert_eq!(scan.incoming()[0].amount(), 13);
    assert_eq!(scan.states[1].as_ref().unwrap().balance(), 7);
    let incoming = scan.incoming()[0].clone();
    let (claim, bstate2) = p.absorb(&b, &ledger, 1, &bstate, &incoming).unwrap();
    v.verify(&claim).unwrap();
    ledger.apply(v, claim.clone()).unwrap();
    assert_eq!(bstate2.balance(), 20);
    assert_eq!(astate2.balance(), 87);
    assert_eq!(ledger.pending_count(), 0);
    assert_eq!(ledger.marker_count(), 1);
    assert!(matches!(
        p.absorb(&b, &ledger, 1, &bstate2, &incoming),
        Err(Error::Conflict)
    ));
    scan.advance(&bobview, v, &ledger.records()[1]).unwrap();
    scan.require_head(ledger.head()).unwrap();
    assert!(scan.incoming().is_empty());
    assert_eq!(scan.states[1].as_ref().unwrap().balance(), 20);
    // Observable linkability is a required negative result, not a privacy claim.
    assert_eq!(tx.credit, claim.credit); // public debit -> absorb link
    assert_eq!(tx.old, g.leaves[0]); // public genesis -> first update link
    let (repeat, _) = p.debit(&a, &ledger, 0, &astate2, &b.address(), 1).unwrap();
    assert_eq!(repeat.slot, tx.slot);
    assert_eq!(repeat.old, tx.new); // stable slot AND commitment graph
    v.verify(&repeat).unwrap();
    // Reanchoring a proof or swapping slots cannot rescue an expired witness.
    let mut refreshed = tx;
    refreshed.anchor = ledger.root();
    assert!(v.verify(&refreshed).is_err());
    // Restore from encrypted backup and rescan authenticated historical records.
    let backup_identity = x25519::Identity::generate();
    let backup = b.backup(&backup_identity.to_public()).unwrap();
    assert!(Wallet::restore(&x25519::Identity::generate(), &backup).is_err());
    let restored = Wallet::restore(&backup_identity, &backup).unwrap();
    assert_eq!(restored.address().owner, b.address().owner);
    let restored_view = restored.viewing_authority();
    let mut recovered = Scanner::new(&restored_view, &g, g.checkpoint()).unwrap();
    recovered
        .advance(&restored_view, v, &ledger.records()[0])
        .unwrap();
    // Simulate interruption by serially resuming the retained cursor.
    assert_eq!(recovered.cursor(), 1);
    assert!(recovered.require_head(ledger.head()).is_err());
    recovered
        .advance(&restored_view, v, &ledger.records()[1])
        .unwrap();
    recovered.require_head(ledger.head()).unwrap();
    assert_eq!(
        recovered.states[1].as_ref().unwrap().commitment(),
        bstate2.commitment()
    );
    let (spend, _) = p
        .debit(
            &restored,
            &ledger,
            1,
            recovered.states[1].as_ref().unwrap(),
            &a.address(),
            2,
        )
        .unwrap();
    v.verify(&spend).unwrap();
    let mut corrupt = backup;
    corrupt[40] ^= 1;
    assert!(Wallet::restore(&backup_identity, &corrupt).is_err());
    let mut corrupt = g.clone();
    corrupt.ciphertexts[0][10] ^= 1;
    assert!(Scanner::new(&restored_view, &corrupt, g.checkpoint()).is_err());
}

#[test]
fn authenticated_encryption_detects_corrupt_incoming_and_state() {
    let g1 = Genesis {
        leaves: [[0; 32]; 2],
        ciphertexts: [vec![1], vec![2]],
    };
    let g2 = Genesis {
        leaves: [[0; 32]; 2],
        ciphertexts: [vec![1, 2], vec![]],
    };
    assert_ne!(g1.checkpoint(), g2.checkpoint());
    let w = Wallet::new();
    let a = w.address();
    let state = w.initial(42);
    let ct = encrypt(&a.recipient, &state.encode()).unwrap();
    assert_eq!(
        State::decode(&decrypt(&w.viewing, &ct).unwrap())
            .unwrap()
            .balance(),
        42
    );
    assert!(decrypt(&Wallet::new().viewing, &ct).is_err());
    let mut bad = ct.clone();
    let end = bad.len() - 1;
    bad[end] ^= 1;
    assert!(decrypt(&w.viewing, &bad).is_err());
    assert!(decrypt(&w.viewing, &ct[..ct.len() - 1]).is_err());
    assert!(State::decode(&[0; 111]).is_err());
    assert!(Credit::decode(&[0; 105]).is_err());
}

#[test]
fn valid_proof_does_not_guarantee_encrypted_delivery_or_recovery_availability() {
    let (p, v) = keys();
    let a = Wallet::new();
    let b = Wallet::new();
    let (g, [old, _]) = synthetic_genesis(&a, &b, [10, 0]).unwrap();
    let mut ledger = Ledger::from_genesis(&g);
    let new = State {
        balance: 9,
        version: 1,
        blind: random(),
        ..old.clone()
    };
    let credit = Credit {
        owner: b.address().owner,
        view: b.address().view(),
        amount: 1,
        salt: random(),
    };
    // This is a real adversarial witness, not a mock proof: ciphertext validity
    // is outside the current relation and a malicious authorized owner can burn
    // a credit and withhold its own next opening. Make that blocker executable.
    let ciphertext = vec![99; 64];
    let ct_hash = hash(&[&ciphertext]);
    let mut tx = Transaction {
        debit: true,
        slot: 0,
        anchor: ledger.root(),
        old: old.commitment(),
        new: new.commitment(),
        credit: credit.commitment(ct_hash),
        credit_ciphertext: ciphertext,
        state_ciphertext: vec![88; 64],
        binding: [0; 32],
        proof: Vec::new(),
    };
    tx.binding = hash(&[&[5], &*a.secret, &tx.context()]);
    let r = relation::Relation {
        debit: true,
        old,
        new,
        secret: *a.secret,
        credit,
        ciphertext_hash: ct_hash,
        old_commitment: tx.old,
        new_commitment: tx.new,
        credit_commitment: tx.credit,
        context: tx.context(),
        binding: tx.binding,
    };
    Groth16::<Bls12_381>::create_random_proof_with_reduction(r, &p.key, &mut OsRng)
        .unwrap()
        .serialize_compressed(&mut tx.proof)
        .unwrap();
    v.verify(&tx).unwrap();
    ledger.apply(v, tx).unwrap();
    let av = a.viewing_authority();
    let bv = b.viewing_authority();
    let mut ascan = Scanner::new(&av, &g, g.checkpoint()).unwrap();
    assert_eq!(
        ascan.advance(&av, v, &ledger.records()[0]),
        Err(Error::Recovery)
    );
    assert_eq!(ascan.cursor(), 0); // failure never commits a stale wallet cursor
    let mut bscan = Scanner::new(&bv, &g, g.checkpoint()).unwrap();
    bscan.advance(&bv, v, &ledger.records()[0]).unwrap();
    bscan.require_head(ledger.head()).unwrap();
    assert!(bscan.incoming().is_empty());
    assert_eq!(ledger.pending_count(), 1);
}
