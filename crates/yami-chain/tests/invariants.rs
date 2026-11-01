use yami_chain::*;
fn state() -> State {
    State::new(Genesis::local()).unwrap()
}
#[test]
fn canonical_replay_and_versioned_vectors() {
    let initial = state();
    let mut a = initial.clone();
    let mut b = initial.clone();
    for _ in 0..20 {
        let block = a.development_block().unwrap();
        let bytes = block.canonical().unwrap();
        assert_eq!(Block::decode(&bytes).unwrap(), block);
        a = a.execute(&block).unwrap();
        b = b.execute(&Block::decode(&bytes).unwrap()).unwrap();
        assert_eq!(a, b);
    }
    assert_eq!(initial.height(), 0);
    // Independent Python hashlib/Ed25519 encoding, recorded in vectors.md.
    assert_eq!(
        initial.genesis().id().to_string(),
        "d8b06ded9d9a1159dd872bf164abdba3e9f260b13369414fc1df7d4b8f19c806"
    );
    assert_eq!(
        initial.root().to_string(),
        "ed53a6b3ac52317b8492c2e54b5c4f9af3a6938c35e73f7f7e94335b6b73e381"
    );
    assert_eq!(
        a.root().to_string(),
        "cb30e18ba81cad8f669389e40f08daa3e19be24ed257b50c557867b90198df1c"
    );
}
#[test]
fn malformed_domains_versions_lengths_and_trailing_bytes() {
    let mut genesis = Genesis::local();
    genesis.chain_id = "other".into();
    assert_eq!(State::new(genesis), Err(Error::Chain));
    let mut genesis = Genesis::local();
    genesis.protocol = 2;
    assert_eq!(State::new(genesis), Err(Error::Version));
    let mut genesis = Genesis::local();
    genesis.validators[1] = genesis.validators[0];
    assert_eq!(State::new(genesis), Err(Error::Genesis));
    let mut genesis = Genesis::local();
    genesis.features = 1;
    assert_eq!(State::new(genesis), Err(Error::Version));
    let good = state().development_block().unwrap().canonical().unwrap();
    for len in 0..good.len() {
        assert!(Block::decode(&good[..len]).is_err());
    }
    let mut trailing = good.clone();
    trailing.push(0);
    assert_eq!(Block::decode(&trailing), Err(Error::Encoding));
    let mut wrong_chain = good.clone();
    wrong_chain[14] = b'X';
    assert_eq!(Block::decode(&wrong_chain), Err(Error::Chain));
    let mut version = good.clone();
    let version_offset = 14 + CHAIN_ID.len();
    version[version_offset + 1] = 2;
    assert_eq!(Block::decode(&version), Err(Error::Version));
    let mut huge = good;
    huge[10..14].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(Block::decode(&huge), Err(Error::Limit));
    assert_eq!(
        Block::decode(&vec![0; MAX_BLOCK_BYTES + 1]),
        Err(Error::Limit)
    );
}
#[test]
fn signatures_nonce_expiry_duplicates_and_atomic_module_errors() {
    let s = state();
    let key = development_key("registry", 0);
    let tx = Transaction::sign("test-a", 0, 10, &key).unwrap();
    let mut bad = tx.clone();
    bad.signature[0] ^= 1;
    assert_eq!(s.propose(vec![bad]), Err(Error::Signature));
    let duplicate = s.propose(vec![tx.clone(), tx.clone()]);
    assert_eq!(duplicate, Err(Error::Duplicate));
    let reserved = Transaction::sign("yami", 1, 10, &key).unwrap();
    assert_eq!(s.propose(vec![tx.clone(), reserved]), Err(Error::Naming));
    assert_eq!(s.height(), 0);
    assert_eq!(s.names().entries().count(), 0);
    let mut bad = tx.clone();
    bad.chain_id = "foreign".into();
    assert_eq!(s.propose(vec![bad]), Err(Error::Chain));
    let expired = Transaction::sign("test-x", 0, 0, &key).unwrap();
    assert_eq!(s.propose(vec![expired]), Err(Error::Expired));
    let reused_consensus_key =
        Transaction::sign("test-role", 0, 10, &development_key("consensus", 0)).unwrap();
    assert_eq!(s.propose(vec![reused_consensus_key]), Err(Error::Signature));
    let next = s.execute(&s.propose(vec![tx.clone()]).unwrap()).unwrap();
    assert_eq!(next.propose(vec![tx]), Err(Error::Nonce));
    let mut block = next.development_block().unwrap();
    block.header.parent = Digest::default();
    assert_eq!(next.execute(&block), Err(Error::Header));
    let txs = (0..MAX_TXS + 1)
        .map(|_| Transaction::sign("test-a", 0, 1, &key).unwrap())
        .collect();
    assert_eq!(s.propose(txs), Err(Error::Limit));
}
#[test]
fn amount_overflow_underflow_and_privacy_types_never_decode_as_actions() {
    assert_eq!(
        Amount(u128::MAX).checked_add(Amount(1)),
        Err(Error::Overflow)
    );
    assert_eq!(Amount(0).checked_sub(Amount(1)), Err(Error::Overflow));
    assert_eq!(Amount(2).checked_sub(Amount(1)), Ok(Amount(1)));
    let tx = Transaction::sign("test-x", 0, 1, &development_key("registry", 0)).unwrap();
    let mut bytes = tx.canonical().unwrap();
    let tag = 7 + 4 + CHAIN_ID.len() + 2 + 8 + 8 + 8 + 32;
    bytes[tag] = 2;
    assert_eq!(Transaction::decode(&bytes), Err(Error::Unsupported));
}
