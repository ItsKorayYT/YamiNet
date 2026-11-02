use yami_chain::*;
#[test]
fn schedules_and_role_isolation_reject_invalid_or_unauthorized_changes() {
    assert_eq!(
        Genesis::development_schedule(&vec![(1, 100); committee::MAX_EPOCHS + 1]),
        Err(Error::Genesis)
    );
    for plan in [
        vec![],
        vec![(0, 4)],
        vec![(2, 4)],
        vec![(1, 0)],
        vec![(1, 101)],
        vec![(1, 4), (1, 7)],
        vec![(1, 4), (3, 7), (2, 4)],
    ] {
        assert!(Genesis::development_schedule(&plan).is_err());
    }
    let genesis = Genesis::development_schedule(&[(1, 4), (3, 7)]).unwrap();
    let set = genesis.committee_at(3).unwrap();
    let mut members = set.members().to_vec();
    members[1] = members[0].clone();
    assert!(ValidatorSet::new(members).is_err());
    let mut members = set.members().to_vec();
    members[0].power = 0;
    assert!(ValidatorSet::new(members).is_err());
    let mut members = set.members().to_vec();
    members[0].power = u64::MAX;
    assert!(ValidatorSet::new(members).is_err());
    let state = State::new(genesis.clone()).unwrap();
    let future_key = development_key("consensus", 6);
    assert_eq!(
        state.propose(vec![
            Transaction::sign("test-reuse", 0, 5, &future_key).unwrap()
        ]),
        Err(Error::Signature)
    );
    let block = state.development_block().unwrap();
    let state = state.execute(&block).unwrap();
    let mut forged = state.development_block().unwrap();
    forged.header.committee = genesis.committee_at(3).unwrap().hash();
    assert_eq!(state.execute(&forged), Err(Error::Header));
    let state = state.execute(&state.development_block().unwrap()).unwrap();
    assert_eq!(
        state.development_block().unwrap().header.committee,
        genesis.committee_at(3).unwrap().hash()
    );
}
