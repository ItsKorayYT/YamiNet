use super::*;
fn full_state() -> State {
    let mut state = State::new(Genesis::local()).unwrap();
    state.height = 1563;
    state.tip = hash(b"snapshot-test-tip", b"fixture");
    let key = development_key("registry", 0).verifying_key().to_bytes();
    for i in 0..100_000 {
        let label = format!("test-{i:058}");
        state.names.register(&label, key).unwrap();
    }
    state.nonces.insert(key, 100_000);
    state
}
#[test]
fn maximum_count_and_label_snapshot_rebuilds_caches_and_preserves_growth_refusal() {
    let state = full_state();
    let bytes = state.snapshot_bytes().unwrap();
    assert_eq!(bytes.len(), 9_900_130);
    assert_eq!(bytes.len(), state.snapshot_len().unwrap());
    let mut appended = vec![7; 20];
    state.append_snapshot(&mut appended).unwrap();
    assert_eq!(&appended[20..], bytes);
    let restored = State::restore_checkpoint(
        state.genesis().clone(),
        state.height(),
        state.tip(),
        state.root(),
        &bytes,
    )
    .unwrap();
    assert_eq!(state, restored);
    assert_eq!(restored.snapshot_bytes().unwrap(), bytes);
    assert_eq!(restored.names.len(), 100_000);
    let owner = development_key("registry", 0);
    let block = restored
        .propose(vec![Transaction::sign(
            "test-over-limit",
            100_000,
            1564,
            &owner,
        )
        .unwrap()])
        .unwrap();
    let grown = restored.execute(&block).unwrap();
    assert_eq!(grown.names.len(), 100_001);
    assert_eq!(grown.snapshot_bytes(), Err(Error::Limit));
    let mut target = vec![1, 2, 3];
    assert_eq!(grown.append_snapshot(&mut target), Err(Error::Limit));
    assert_eq!(target, [1, 2, 3]);
    assert_eq!(restored.snapshot_bytes().unwrap(), bytes);
    let mut forged = bytes.clone();
    forged[126..130].copy_from_slice(&100_001u32.to_be_bytes());
    assert!(State::restore_checkpoint(
        state.genesis().clone(),
        1563,
        state.tip(),
        state.root(),
        &forged
    )
    .is_err());
}
#[test]
fn corrupted_snapshots_never_supply_cached_roots_or_indexes() {
    let state = State::new(Genesis::local()).unwrap();
    let b = state.development_workload_block(16).unwrap();
    let state = state.execute(&b).unwrap();
    let bytes = state.snapshot_bytes().unwrap();
    for offset in (0..bytes.len()).step_by(13) {
        let mut bad = bytes.clone();
        bad[offset] ^= 1;
        assert!(
            State::restore_checkpoint(state.genesis().clone(), 1, state.tip(), state.root(), &bad)
                .is_err(),
            "offset {offset}"
        );
    }
    for n in 0..bytes.len() {
        assert!(State::restore_checkpoint(
            state.genesis().clone(),
            1,
            state.tip(),
            state.root(),
            &bytes[..n]
        )
        .is_err());
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(State::restore_checkpoint(
        state.genesis().clone(),
        1,
        state.tip(),
        state.root(),
        &trailing
    )
    .is_err());
}
#[test]
fn derived_reserved_key_index_matches_all_epoch_keys_and_unreserved_accounts() {
    let genesis = Genesis::development_schedule(&[(1, 4), (5, 7), (10, 100)]).unwrap();
    let state = State::new(genesis.clone()).unwrap();
    for epoch in &genesis.epochs {
        for member in epoch.validators.members() {
            for key in [member.consensus_key, member.transport_key] {
                assert_eq!(
                    state.reserved_keys.contains(&key),
                    genesis.is_reserved_key(&key)
                );
            }
        }
    }
    for i in 0..200 {
        let key = development_key("registry", i).verifying_key().to_bytes();
        assert_eq!(
            state.reserved_keys.contains(&key),
            genesis.is_reserved_key(&key)
        );
    }
}

#[test]
fn distinct_laboratory_accounts_execute_deterministically_without_changing_default_workloads() {
    let state = State::new(Genesis::local()).unwrap();
    assert_eq!(
        state.development_distinct_workload_block(0),
        state.development_block()
    );
    assert!(state.development_distinct_workload_block(65).is_err());
    let block = state.development_distinct_workload_block(64).unwrap();
    assert_eq!(
        block,
        state.development_distinct_workload_block(64).unwrap()
    );
    let next = state.execute(&block).unwrap();
    assert_eq!(next.names().len(), 64);
    assert_eq!(next.nonces.len(), 64);
    assert!(next.nonces.values().all(|n| *n == 1));
    let second = next
        .execute(&next.development_distinct_workload_block(64).unwrap())
        .unwrap();
    assert_eq!(second.names().len(), 128);
    assert_eq!(second.nonces.len(), 128);
    assert_eq!(state.names().len(), 0);
    assert_eq!(next.names().len(), 64);
    assert_eq!(
        state.development_workload_block(64).unwrap(),
        state.development_workload_block(64).unwrap()
    );
}
