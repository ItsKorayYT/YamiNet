use super::*;
use ark_bls12_381::Fr;
use ark_groth16::Proof;
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystem};
use merkle::*;
use std::sync::{Arc, OnceLock};
fn keys() -> &'static (Prover, Verifier, Arc<Params>, usize) {
    static KEYS: OnceLock<(Prover, Verifier, Arc<Params>, usize)> = OnceLock::new();
    KEYS.get_or_init(|| setup().expect("real local setup"))
}
fn sat(r: relation::Relation) -> bool {
    let cs = ConstraintSystem::<Fr>::new_ref();
    r.generate_constraints(cs.clone()).unwrap();
    cs.is_satisfied().unwrap()
}

#[test]
fn real_hidden_writes_reproduce_lost_updates_conflicts_and_public_patch_leakage() {
    let (p, v, params, _) = keys();
    let (a, av) = Wallet::new((1u64 << 48) + 77);
    let (b, bv) = Wallet::new(u64::MAX - 10);
    let initial = Snapshot::new(params.clone(), &[av.clone(), bv.clone()]).unwrap();
    let (ta, an, sa) = p.rotate(&a, &initial, &av).unwrap();
    let (tb, bn, sb) = p.rotate(&b, &initial, &bv).unwrap();
    let (conflict, _, _) = p.rotate(&a, &initial, &av).unwrap();
    let (repeat, an2, _) = p.rotate(&a, &sa, &an).unwrap();
    let (fresh_b, _, merged_fresh) = p.rotate(&b, &sa, &bv).unwrap();
    for t in [&ta, &tb, &conflict, &repeat, &fresh_b] {
        v.verify(t).unwrap();
        assert_eq!(t.encode().len(), PACKET_BYTES);
        v.verify(&Write::decode(&t.encode()).unwrap()).unwrap();
    }
    assert_eq!(ta.before, tb.before);
    assert_eq!(ta.marker, conflict.marker);
    assert_ne!(ta.marker, repeat.marker);
    let mut strict = Node::new(initial.root(), Policy::Strict);
    strict.apply(v, ta.clone()).unwrap();
    let head = strict.head;
    assert_eq!(strict.apply(v, tb.clone()), Err(Error::Stale));
    assert_eq!(strict.head, head);
    // With a fresh witness supplied by a patch/coordinator, the safe sequential
    // update works, but this requires information absent from root-only packets.
    strict.apply(v, fresh_b.clone()).unwrap();
    assert_eq!(strict.root, merged_fresh.root());
    let mut unsafe_node = Node::new(initial.root(), Policy::UnsafeHistoricalReplacement);
    unsafe_node.apply(v, ta.clone()).unwrap();
    assert_eq!(unsafe_node.apply(v, conflict.clone()), Err(Error::Conflict));
    unsafe_node.apply(v, tb.clone()).unwrap();
    let true_merged = sa.replace(1, bn.commitment()).unwrap();
    assert_ne!(unsafe_node.root, true_merged.root());
    assert_eq!(unsafe_node.root, sb.root());
    assert!(!sb.contains(&an));
    assert!(sb.contains(&av));
    // Replaying the same accepted history gives exactly the same WRONG root:
    // deterministic agreement alone is not a proof of financial state safety.
    let mut replica = Node::new(initial.root(), Policy::UnsafeHistoricalReplacement);
    for r in &unsafe_node.records {
        replica.replay(v, r).unwrap();
    }
    assert_eq!(replica.root, unsafe_node.root);
    assert_eq!(replica.head, unsafe_node.head);
    assert_eq!(replica.apply(v, ta.clone()), Err(Error::Conflict));
    unsafe_node.apply(v, repeat.clone()).unwrap();
    assert_ne!(unsafe_node.root, true_merged.root()); // now Bob is overwritten
    let mut additive = Node::new(initial.root(), Policy::UnsafeFieldDelta);
    additive.apply(v, ta.clone()).unwrap();
    additive.apply(v, tb.clone()).unwrap();
    assert_ne!(additive.root, true_merged.root());
    // Minimal public-new-commitment repair reveals the otherwise hidden slot.
    assert_eq!(
        initial.locate_published_write(an.commitment(), ta.after),
        vec![0]
    );
    assert_eq!(
        initial.locate_published_write(bn.commitment(), tb.after),
        vec![1]
    );
    assert_eq!(
        sa.locate_published_write(an2.commitment(), repeat.after),
        vec![0]
    );
    assert!(!ta.encode().windows(32).any(|x| x == av.commitment()));
    assert!(!ta.encode().windows(32).any(|x| x == an.commitment())); // root-only original format
    assert!(matches!(
        p.rotate(&a, &initial, &bv),
        Err(Error::Authorization)
    )); // offline Bob's witness/key unavailable
    assert!(!sat({
        let (mut r, _, _) = statement(&a, &initial, &av).unwrap();
        r.old = bv.clone();
        r.path = initial.path(bv.commitment()).unwrap();
        r
    }));
    // Every public field is proof-bound; malformed packets cannot mutate state.
    for i in 0..4 {
        let mut bad = ta.clone();
        match i {
            0 => bad.before += Fr::from(1u64),
            1 => bad.after += Fr::from(1u64),
            2 => bad.marker[0] ^= 1,
            _ => bad.proof = Proof::default(),
        };
        assert!(v.verify(&bad).is_err());
    }
    let encoded = ta.encode();
    assert!(Write::decode(&encoded[..encoded.len() - 1]).is_err());
    let mut trailing = encoded;
    trailing.push(0);
    assert!(Write::decode(&trailing).is_err());
    let vk = v.encode();
    assert!(Verifier::decode(&vk).is_ok());
    let mut badvk = vk;
    badvk[336..344].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(Verifier::decode(&badvk).is_err());
    // Authenticated recovery can detect interrupted/corrupt history but cannot
    // recover a random new opening that root-only ZK packets do not deliver.
    let mut recovering = Node::new(initial.root(), Policy::Strict);
    assert!(recovering.require_head(strict.head).is_err());
    recovering.replay(v, &strict.records[0]).unwrap();
    let cursor = recovering.records.len();
    let mut bad = strict.records[1].clone();
    bad.head[0] ^= 1;
    assert_eq!(recovering.replay(v, &bad), Err(Error::Recovery));
    assert_eq!(recovering.records.len(), cursor);
    recovering.replay(v, &strict.records[1]).unwrap();
    recovering.require_head(strict.head).unwrap();
    let id = age::x25519::Identity::generate();
    let old_backup = a.backup(&av, &id.to_public()).unwrap();
    let new_backup = a.backup(&an, &id.to_public()).unwrap();
    let (restored, old_open) = Wallet::restore(&id, &old_backup).unwrap();
    assert!(!sa.contains(&old_open));
    assert!(matches!(
        p.rotate(&restored, &sa, &old_open),
        Err(Error::Membership)
    ));
    let (_, updated) = Wallet::restore(&id, &new_backup).unwrap();
    assert_eq!(updated.commitment(), an.commitment());
    assert!(sa.contains(&updated));
    assert!(Wallet::restore(&age::x25519::Identity::generate(), &new_backup).is_err());
    let mut corrupt = new_backup;
    let end = corrupt.len() - 1;
    corrupt[end] ^= 1;
    assert!(Wallet::restore(&id, &corrupt).is_err());
}

#[test]
fn constraint_failures_include_fully_rebound_inflation_and_u64_wrap() {
    let (_, _, params, _) = keys();
    let (w, old) = Wallet::new(u64::MAX - 100);
    let snap = Snapshot::new(params.clone(), std::slice::from_ref(&old)).unwrap();
    let (r, _, _) = statement(&w, &snap, &old).unwrap();
    assert!(sat(r.clone()));
    let mut bad = r.clone();
    bad.new.balance += 1;
    bad.write.after = snap
        .replace(bad.path.index, bad.new.commitment())
        .unwrap()
        .root();
    assert!(!sat(bad)); // correct root, forbidden inflation
    let mut bad = r.clone();
    bad.secret = [9; 32];
    assert!(!sat(bad));
    let mut bad = r.clone();
    bad.path.index ^= 1;
    assert!(!sat(bad));
    let mut bad = r.clone();
    bad.path.siblings[0][0] ^= 1;
    assert!(!sat(bad));
    let mut exhausted = old.clone();
    exhausted.version = u64::MAX;
    let exhausted_snapshot =
        Snapshot::new(params.clone(), std::slice::from_ref(&exhausted)).unwrap();
    assert!(matches!(
        statement(&w, &exhausted_snapshot, &exhausted),
        Err(Error::Overflow)
    ));
    let mut bad = r;
    bad.old = exhausted;
    bad.new.version = 0;
    bad.path = exhausted_snapshot.path(bad.old.commitment()).unwrap();
    bad.write.before = exhausted_snapshot.root();
    bad.write.after = exhausted_snapshot
        .replace(0, bad.new.commitment())
        .unwrap()
        .root();
    bad.write.marker = marker(w.secret, bad.old.commitment());
    assert!(!sat(bad));
}

#[test]
fn kzg_commuting_updates_need_refresh_and_public_refresh_reveals_the_write() {
    for n in [4, 16, 64] {
        let result = polynomial::run(n).unwrap();
        assert!(result.independent_updates_commute);
        assert!(result.stale_witness_rejected);
        assert!(result.refreshed_witness_valid);
        assert_eq!(result.revealed_slots, vec![0]);
        assert!(result.repeated_hidden_write_reidentified);
        assert!(result.same_slot_blind_merge_is_not_a_version_update);
    }
    let e = polynomial::Experiment::new(4).unwrap();
    let p = e.delta(2, Fr::from(123u64)).unwrap();
    let c = e.commit(&p).unwrap();
    let hints = e.hints(&p).unwrap();
    assert_eq!(e.infer_nonzero_updates(&c, &hints).unwrap(), vec![2]);
    assert!(e.check(&c, 2, Fr::from(123u64), &hints[2]));
    assert!(!e.check(&c, 2, Fr::from(124u64), &hints[2]));
    assert!(!e.check(&c, 1, Fr::from(0u64), &hints[2]));
    assert!(e.infer_nonzero_updates(&c, &hints[..3]).is_err());
}

#[test]
fn authenticated_anchor_archive_scales_and_detects_stale_or_corrupt_membership() {
    // Independent full rebuild at awkward tree sizes catches cached-append
    // errors that a self-generated path/root round trip alone could miss.
    for count in [0usize, 1, 2, 3, 5, 17, 64, 65, 127, 129] {
        let values = (0..count)
            .map(|i| hash(&[&(i as u64).to_le_bytes()]))
            .collect::<Vec<_>>();
        let mut cached = archive::Archive::new();
        for value in &values {
            cached.append(*value);
        }
        let empty = hash(&[b"YPP3-anchor-empty"]);
        let mut layer = values
            .iter()
            .map(|v| hash(&[b"YPP3-anchor-leaf", v]))
            .collect::<Vec<_>>();
        layer.resize(count.max(1).next_power_of_two(), empty);
        while layer.len() > 1 {
            layer = layer
                .chunks_exact(2)
                .map(|p| hash(&[b"YPP3-anchor-node", &p[0], &p[1]]))
                .collect();
        }
        let rebuilt = hash(&[
            b"YPP3-anchor-checkpoint",
            &(count as u64).to_le_bytes(),
            &layer[0],
        ]);
        assert_eq!(cached.checkpoint(), rebuilt);
        for (i, value) in values.iter().enumerate() {
            assert!(archive::Archive::verify(
                rebuilt,
                &cached.proof(i, *value).unwrap()
            ));
        }
    }
    let mut a = archive::Archive::new();
    let first = hash(&[b"first"]);
    a.append(first);
    let initial = a.proof(0, first).unwrap();
    assert!(archive::Archive::verify(a.checkpoint(), &initial));
    for i in 1..100_000u64 {
        a.append(hash(&[&i.to_le_bytes()]));
    }
    assert!(!archive::Archive::verify(a.checkpoint(), &initial));
    let refreshed = a.proof(0, first).unwrap();
    assert!(archive::Archive::verify(a.checkpoint(), &refreshed));
    assert_eq!(refreshed.siblings.len(), 17);
    let mut bad = refreshed.clone();
    bad.siblings[0][0] ^= 1;
    assert!(!archive::Archive::verify(a.checkpoint(), &bad));
    let mut bad = refreshed;
    bad.count -= 1;
    assert!(!archive::Archive::verify(a.checkpoint(), &bad));
    assert!(a.proof(0, [0; 32]).is_none());
    assert!(a.proof(100_000, first).is_none());
}
#[test]
fn event_liveness_is_measured_cost_simulation_and_does_not_fix_write_safety() {
    assert_eq!(
        liveness::simulate(&[16_235.0], 8.59, 2, 1.0, Some(4)).accepted,
        0
    );
    assert_eq!(
        liveness::simulate(&[16_235.0], 8.59, 2, 1.0, Some(64)).accepted,
        2
    );
    assert_eq!(
        liveness::simulate(&[16_235.0], 8.59, 64, 4.0, None).accepted,
        64
    );
    assert_eq!(
        liveness::simulate(&[16_235.0], 8.59, 8, 0.0, Some(4)).accepted,
        4
    );
}
