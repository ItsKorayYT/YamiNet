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
