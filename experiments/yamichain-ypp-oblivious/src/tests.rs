use super::*;

fn fixture(n: usize) -> (Parameters, Vec<Wallet>, Vec<Cell>, StorageLab) {
    let p = Parameters::default();
    let wallets: Vec<_> = (0..n).map(Wallet::new).collect();
    let cells: Vec<_> = wallets
        .iter()
        .map(|w| {
            w.encrypt(&PlainState {
                version: 0,
                counter: 7,
            })
        })
        .collect();
    let lab = StorageLab::new(&cells);
    (p, wallets, cells, lab)
}

#[test]
fn independent_deltas_commute_without_lost_writes() {
    let (p, wallets, cells, mut lab) = fixture(4);
    let a_state = PlainState {
        version: 1,
        counter: 8,
    };
    let b_state = PlainState {
        version: 1,
        counter: 9,
    };
    let a = wallets[0].encrypt(&a_state);
    let b = wallets[3].encrypt(&b_state);
    // Both prepare from the SAME snapshot. No root replacement or retries.
    let (pa, ma) = prepare(&p, 4, 0, delta(a, cells[0])).unwrap();
    let (pb, mb) = prepare(&p, 4, 3, delta(b, cells[3])).unwrap();
    let mut reversed = lab.clone();
    lab.apply(&p, &pa, &ma).unwrap();
    lab.apply(&p, &pb, &mb).unwrap();
    reversed.apply(&p, &pb, &mb).unwrap();
    reversed.apply(&p, &pa, &ma).unwrap();
    assert_eq!(lab.head().hashes, reversed.head().hashes);
    assert_eq!(
        recover(&lab.head(), &lab.snapshots(), &wallets[0]),
        Ok(a_state)
    );
    assert_eq!(
        recover(&lab.head(), &lab.snapshots(), &wallets[3]),
        Ok(b_state)
    );
}

#[test]
fn reject_candidate_conflicting_valid_writes_both_pass_and_destroy_cell() {
    let (p, wallets, cells, mut lab) = fixture(4);
    let a = wallets[1].encrypt(&PlainState {
        version: 1,
        counter: 8,
    });
    let b = wallets[1].encrypt(&PlainState {
        version: 1,
        counter: 9,
    });
    let (pa, ma) = prepare(&p, 4, 1, delta(a, cells[1])).unwrap();
    let (pb, mb) = prepare(&p, 4, 1, delta(b, cells[1])).unwrap();
    assert_ne!(pa.operation, pb.operation);
    lab.apply(&p, &pa, &ma).unwrap();
    assert!(recover(&lab.head(), &lab.snapshots(), &wallets[1]).is_ok());
    assert!(lab.apply(&p, &pa, &ma).is_err()); // exact replay protection is insufficient
    lab.apply(&p, &pb, &mb).unwrap();
    assert!(recover(&lab.head(), &lab.snapshots(), &wallets[1]).is_err());
}

#[test]
fn reject_candidate_public_channels_reveal_repeated_write_positions() {
    let p = Parameters::default();
    let mut transcripts = Vec::new();
    for target in [2, 2, 0, 3, 0] {
        let (packet, channels) =
            prepare(&p, 4, target, [random_scalar(), random_scalar()]).unwrap();
        verify(&p, &packet).unwrap();
        // Publish both to validators (or collude): no secret input to observer.
        transcripts.push(reconstruct_support(&channels));
    }
    assert_eq!(
        transcripts,
        vec![vec![2], vec![2], vec![0], vec![3], vec![0]]
    );
}

#[test]
fn reject_candidate_public_full_retrieval_also_reveals_position() {
    let (p, wallets, cells, mut lab) = fixture(4);
    let before = lab.snapshots();
    let new = wallets[2].encrypt(&PlainState {
        version: 1,
        counter: 10,
    });
    let (packet, messages) = prepare(&p, 4, 2, delta(new, cells[2])).unwrap();
    lab.apply(&p, &packet, &messages).unwrap();
    let after = lab.snapshots();
    let support: Vec<_> = (0..4)
        .filter(|i| {
            (0..2).any(|j| {
                let old = scalar(&before[0].cells[*i][j]).unwrap()
                    + scalar(&before[1].cells[*i][j]).unwrap();
                let new = scalar(&after[0].cells[*i][j]).unwrap()
                    + scalar(&after[1].cells[*i][j]).unwrap();
                old != new
            })
        })
        .collect();
    assert_eq!(support, vec![2]);
}

#[test]
fn encrypted_snapshot_refresh_and_offline_recovery_are_authenticated() {
    let (p, wallets, cells, mut lab) = fixture(4);
    let old_snapshots = lab.snapshots();
    let old_head = lab.head();
    // Wallet 0 goes offline; wallet 2 rotates its ciphertext repeatedly.
    let mut prior = cells[2];
    for v in 1..=8 {
        let next = wallets[2].encrypt(&PlainState {
            version: v,
            counter: 7,
        });
        let (packet, messages) = prepare(&p, 4, 2, delta(next, prior)).unwrap();
        lab.apply(&p, &packet, &messages).unwrap();
        prior = next;
    }
    let head = lab.head(); // must be independently pinned; not chosen by mirrors
    let snapshots = lab.snapshots();
    assert_eq!(
        recover(&head, &snapshots, &wallets[0]),
        Ok(PlainState {
            version: 0,
            counter: 7
        })
    );
    assert_eq!(
        recover(&head, &snapshots, &wallets[2]),
        Ok(PlainState {
            version: 8,
            counter: 7
        })
    );
    assert!(recover(&head, &old_snapshots, &wallets[0]).is_err());
    let mut corrupt = snapshots.clone();
    corrupt[1].cells[0][0][0] ^= 1;
    assert!(recover(&head, &corrupt, &wallets[0]).is_err());
    let mut missing = snapshots.clone();
    missing[1].cells.clear();
    assert!(recover(&head, &missing, &wallets[0]).is_err());
    assert!(recover(&head, &snapshots, &Wallet::new(0)).is_err());
    // A valid old checkpoint permits rollback: freshness comes from finality.
    assert!(recover(&old_head, &old_snapshots, &wallets[0]).is_ok());
}

#[test]
fn reject_candidate_one_server_withholding_prevents_recovery() {
    let (_, wallets, _, lab) = fixture(4);
    let head = lab.head();
    let mut snapshots = lab.snapshots();
    snapshots[0].cells.clear();
    assert!(recover(&head, &snapshots, &wallets[0]).is_err());
    // A checkpoint authenticates bytes but cannot recreate a missing share.
}

#[test]
fn reject_candidate_offline_receiving_has_no_authorization_relation() {
    let (p, wallets, cells, mut lab) = fixture(4);
    // A sender without recipient key can prepare a shape-valid overwrite.
    // We deliberately expose this as corruption, never as a successful credit.
    let forged = Wallet::new(1).encrypt(&PlainState {
        version: 1,
        counter: 99,
    });
    let (packet, messages) = prepare(&p, 4, 1, delta(forged, cells[1])).unwrap();
    lab.apply(&p, &packet, &messages).unwrap();
    assert!(recover(&lab.head(), &lab.snapshots(), &wallets[1]).is_err());
}

#[test]
fn malformed_proofs_or_share_messages_are_rejected_atomically() {
    let (p, _, _, mut lab) = fixture(4);
    let (packet, messages) = prepare(&p, 4, 3, [random_scalar(), random_scalar()]).unwrap();
    let before = lab.head().hashes;
    let mut bad = packet.clone();
    bad.operation[0] ^= 1;
    assert!(lab.apply(&p, &bad, &messages).is_err());
    let mut bad = packet.clone();
    bad.commitments[0][0] = [255; 32];
    assert!(verify(&p, &bad).is_err());
    let mut bad = packet.clone();
    bad.proof.responses[0].pop();
    assert!(verify(&p, &bad).is_err());
    let mut bad = packet.clone();
    bad.proof.challenges[0] = [255; 32];
    assert!(verify(&p, &bad).is_err());
    let mut bad = messages.clone();
    bad[1].delta[0][0] += Scalar::ONE;
    assert!(lab.apply(&p, &packet, &bad).is_err());
    assert_eq!(before, lab.head().hashes);
}

#[test]
fn zero_change_and_either_server_share_verify() {
    let p = Parameters::default();
    for n in [2, 4, 8] {
        for target in [0, n - 1] {
            let (packet, messages) = prepare(&p, n, target, [Scalar::ZERO; 2]).unwrap();
            verify(&p, &packet).unwrap();
            for (s, message) in messages.iter().enumerate() {
                verify_share(&p, &packet, s, message).unwrap();
            }
            assert!(reconstruct_support(&messages).is_empty());
        }
    }
    assert!(prepare(&p, 1, 0, [Scalar::ZERO; 2]).is_err());
}

#[test]
fn replicated_database_pir_is_not_pir_over_additive_storage_shares() {
    // Standard linear two-replica PIR: q0 + q1 = e_target. It requires
    // identical databases at both responders. Counterexample uses exact ints.
    let database = [10u64, 20, 30, 40].map(Scalar::from);
    let share0 = [1u64, 2, 3, 4].map(Scalar::from);
    let share1: Vec<_> = database.iter().zip(share0).map(|(d, a)| d - a).collect();
    let q0 = [5u64, 6, 7, 8].map(Scalar::from);
    let target = 2;
    let q1: Vec<_> = q0
        .iter()
        .enumerate()
        .map(|(i, q)| {
            (if i == target {
                Scalar::ONE
            } else {
                Scalar::ZERO
            }) - q
        })
        .collect();
    let dot = |a: &[Scalar], b: &[Scalar]| a.iter().zip(b).map(|(a, b)| a * b).sum::<Scalar>();
    assert_eq!(dot(&q0, &database) + dot(&q1, &database), database[target]);
    assert_ne!(dot(&q0, &share0) + dot(&q1, &share1), database[target]);
}

#[test]
fn rebinding_second_nonzero_cell_invalidates_shape_proof() {
    let p = Parameters::default();
    let (mut packet, mut messages) = prepare(&p, 4, 3, [Scalar::ONE; 2]).unwrap();
    messages[0].delta[0][0] += Scalar::ONE;
    packet.commitments[0][0] = p
        .commit(&messages[0].delta[0], messages[0].blindings[0])
        .compress()
        .to_bytes();
    // All share openings now verify; the existing point-write proof does not.
    for (s, message) in messages.iter().enumerate() {
        verify_share(&p, &packet, s, message).unwrap();
    }
    assert!(verify(&p, &packet).is_err());
}
