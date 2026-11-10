use yami_chain::{development_key, Error, Genesis, State, Transaction, CHAIN_ID};
#[test]
fn native_action_has_an_explicit_default_closed_wire_boundary() {
    let key = development_key("registry", 0);
    let tx = Transaction::sign("test-wire", 0, 1, &key).unwrap();
    let mut raw = tx.canonical().unwrap();
    let tag = 7 + 4 + CHAIN_ID.len() + 2 + 8 + 8 + 8 + 32;
    raw[tag] = 240;
    #[cfg(not(feature = "insecure-devnet-input"))]
    assert_eq!(Transaction::decode(&raw), Err(Error::Unsupported));
    #[cfg(feature = "insecure-devnet-input")]
    {
        let tx = Transaction::devnet_certificate(b"{}".to_vec(), 0, 1, &key).unwrap();
        let mut bytes = tx.canonical().unwrap();
        assert_eq!(Transaction::decode(&bytes).unwrap(), tx);
        bytes[tag + 2] = 2;
        assert_eq!(Transaction::decode(&bytes), Err(Error::Version));
        // Merely building with the feature grants no committee authority.
        assert_eq!(
            State::new(Genesis::local()).unwrap().propose(vec![tx]),
            Err(Error::Unsupported)
        );
    }
    let _ = (Genesis::local(), std::mem::size_of::<State>());
}

#[cfg(feature = "insecure-devnet-input")]
#[test]
fn live_authority_is_network_scoped_sequential_and_rejects_unknown_future_committees() {
    let fixed = Genesis::local();
    let mut g = fixed.clone();
    g.enable_live_committees([7; 32]).unwrap();
    assert_ne!(g.id(), fixed.id());
    let clone = g.clone();
    assert!(g.committee_at(2).is_err());
    let next = Genesis::development_schedule(&[(1, 5)])
        .unwrap()
        .committee_at(1)
        .unwrap();
    assert!(g.install_finalized_committee(3, next.clone()).is_err());
    g.install_finalized_committee(2, next.clone()).unwrap();
    assert_eq!(clone.committee_at(2).unwrap(), next);
    let candidate = development_key("consensus", 4);
    assert!(g.is_reserved_key(&candidate.verifying_key().to_bytes()));
    assert!(g.is_reserved_key(&development_key("transport", 4).verifying_key().to_bytes()));
    let tx = Transaction::sign("test-isolation", 0, 1, &candidate).unwrap();
    assert_eq!(
        State::new(g.clone()).unwrap().propose(vec![tx]),
        Err(Error::Signature)
    );
    assert!(g
        .install_finalized_committee(2, fixed.committee_at(1).unwrap())
        .is_err());
    assert_eq!(g.committee_at(1).unwrap(), fixed.committee_at(1).unwrap());
    assert_eq!(g.id(), clone.id());
    let mut other = fixed.clone();
    other.enable_live_committees([8; 32]).unwrap();
    assert_ne!(g.id(), other.id());
    assert!(other.committee_at(2).is_err());
    assert_eq!(fixed.id(), Genesis::local().id());
}
