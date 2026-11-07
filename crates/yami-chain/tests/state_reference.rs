//! Independent M12-style BTreeMap/flat-buffer oracle, including error ordering.
use std::collections::{BTreeMap, BTreeSet};
use yami_chain::{codec::Writer, *};
#[derive(Clone)]
struct Reference {
    genesis: Genesis,
    height: u64,
    tip: Digest,
    nonces: BTreeMap<[u8; 32], u64>,
    names: BTreeMap<String, [u8; 32]>,
}
impl Reference {
    fn new(genesis: Genesis) -> Self {
        Self {
            tip: genesis.id(),
            genesis,
            height: 0,
            nonces: BTreeMap::new(),
            names: BTreeMap::new(),
        }
    }
    fn application(&self, w: &mut Writer) {
        w.u32(self.nonces.len() as u32);
        for (k, n) in &self.nonces {
            w.raw(k);
            w.u64(*n);
        }
        w.u32(self.names.len() as u32);
        for (label, owner) in &self.names {
            w.bytes(label.as_bytes()).unwrap();
            w.raw(owner);
        }
    }
    fn root(&self) -> Digest {
        let mut w = Writer::default();
        w.raw(&self.genesis.id().0);
        w.u64(self.height);
        self.application(&mut w);
        hash(b"yamichain-state-v1", &w.0)
    }
    fn snapshot(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.raw(b"YC-STATE-1");
        w.raw(&self.genesis.id().0);
        w.u64(self.height);
        w.raw(&self.tip.0);
        self.application(&mut w);
        w.0
    }
    fn stage(&self, txs: &[Transaction]) -> Result<Self> {
        if txs.len() > MAX_TXS {
            return Err(Error::Limit);
        }
        let mut next = self.clone();
        next.height = next.height.checked_add(1).ok_or(Error::Overflow)?;
        let mut seen = BTreeSet::new();
        for tx in txs {
            let bytes = tx.canonical()?;
            if !seen.insert(hash(b"yamichain-tx-id-v1", &bytes)) {
                return Err(Error::Duplicate);
            }
            if self.genesis.is_reserved_key(&tx.registry_owner) {
                return Err(Error::Signature);
            }
            verify(&tx.registry_owner, &tx.signing_bytes()?, &tx.signature)?;
            if tx.expiry < next.height {
                return Err(Error::Expired);
            }
            let nonce = next.nonces.get(&tx.registry_owner).copied().unwrap_or(0);
            if tx.nonce != nonce {
                return Err(Error::Nonce);
            }
            match &tx.action {
                #[cfg(feature = "insecure-devnet-input")]
                Action::PrivatePaymentCertificateV1 { .. } => return Err(Error::Unsupported),
                Action::RegisterTestTld { label } => {
                    yami_chain_naming::validate_label(label).map_err(|_| Error::Naming)?;
                    if next.names.contains_key(label) {
                        return Err(Error::Naming);
                    }
                    next.names.insert(label.clone(), tx.registry_owner);
                }
            }
            next.nonces.insert(
                tx.registry_owner,
                nonce.checked_add(1).ok_or(Error::Overflow)?,
            );
        }
        Ok(next)
    }
    fn propose(&self, txs: Vec<Transaction>) -> Result<Block> {
        let next = self.stage(&txs)?;
        let mut w = Writer::default();
        w.u32(txs.len() as u32);
        for tx in &txs {
            w.bytes(&tx.canonical()?)?;
        }
        let b = Block {
            header: Header {
                height: next.height,
                parent: self.tip,
                committee: self.genesis.committee_at(next.height)?.hash(),
                transactions: hash(b"yamichain-transactions-v1", &w.0),
                state: next.root(),
            },
            transactions: txs,
        };
        b.canonical()?;
        Ok(b)
    }
    fn execute(&self, block: &Block) -> Result<Self> {
        if self.propose(block.transactions.clone())? != *block {
            return Err(Error::Header);
        }
        let mut next = self.stage(&block.transactions)?;
        next.tip = block.id()?;
        Ok(next)
    }
}
#[test]
fn cached_persistent_execution_matches_legacy_roots_bytes_errors_and_forks() {
    let genesis = Genesis::development_schedule(&[(1, 4), (20, 7), (60, 3)]).unwrap();
    let mut old = Reference::new(genesis.clone());
    let mut state = State::new(genesis).unwrap();
    let mut rng = 12345u64;
    for height in 1..=100 {
        assert_eq!(state.root(), old.root());
        assert_eq!(state.snapshot_bytes().unwrap(), old.snapshot());
        let mut transactions = Vec::new();
        let mut nonces = old.nonces.clone();
        for n in 0..4 {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            let owner = development_key("registry", (rng >> 32) as u8 % 16);
            let key = owner.verifying_key().to_bytes();
            let nonce = *nonces.get(&key).unwrap_or(&0);
            transactions.push(
                Transaction::sign(
                    &format!("test-differential-{height}-{n}"),
                    nonce,
                    height,
                    &owner,
                )
                .unwrap(),
            );
            nonces.insert(key, nonce + 1);
        }
        let baseline = old.propose(transactions.clone()).unwrap();
        let actual = state.propose(transactions.clone()).unwrap();
        assert_eq!(baseline.canonical().unwrap(), actual.canonical().unwrap());
        let saved = state.clone();
        let saved_root = saved.root();
        let saved_snapshot = saved.snapshot_bytes().unwrap();
        for mode in 0..9 {
            let mut bad = transactions.clone();
            let t = &transactions[0];
            let label = match &t.action {
                #[cfg(feature = "insecure-devnet-input")]
                Action::PrivatePaymentCertificateV1 { .. } => {
                    unreachable!("naming reference fixture")
                }
                Action::RegisterTestTld { label } => label,
            };
            let owner = development_key("registry", ((rng >> 32) as u8) % 16);
            match mode {
                0 => bad.push(bad[0].clone()),
                1 => bad[0].signature[0] ^= 1,
                2 => bad[0] = Transaction::sign(label, t.nonce + 1, height, &owner).unwrap(),
                3 => bad[0] = Transaction::sign(label, t.nonce, height - 1, &owner).unwrap(),
                4 => {
                    bad[0] = Transaction::sign("yami", 0, height, &development_key("registry", 100))
                        .unwrap()
                }
                5 => {
                    bad[0] = Transaction::sign(
                        "test-INVALID",
                        0,
                        height,
                        &development_key("registry", 100),
                    )
                    .unwrap()
                }
                6 => {
                    bad[0] = Transaction::sign(label, 0, height, &development_key("consensus", 0))
                        .unwrap()
                }
                7 => bad = vec![transactions[0].clone(); 65],
                _ => bad[0].features = 1,
            }
            assert_eq!(state.propose(bad.clone()), old.propose(bad));
            assert_eq!(state, saved);
            assert_eq!(state.root(), saved_root);
        }
        for mode in 0..4 {
            let mut forged = actual.clone();
            match mode {
                0 => forged.header.state.0[0] ^= 1,
                1 => forged.header.parent.0[0] ^= 1,
                2 => forged.header.height += 1,
                _ => forged.header.committee.0[0] ^= 1,
            };
            assert_eq!(state.execute(&forged).err(), old.execute(&forged).err());
            assert_eq!(state, saved);
        }
        old = old.execute(&baseline).unwrap();
        state = state.execute(&actual).unwrap();
        assert_eq!(state.root(), old.root());
        assert_eq!(state.snapshot_bytes().unwrap(), old.snapshot());
        assert_eq!(saved.root(), saved_root);
        assert_eq!(saved.snapshot_bytes().unwrap(), saved_snapshot);
        for (key, value) in &old.nonces {
            assert_eq!(state.nonce(key), *value);
        }
        for (label, owner) in &old.names {
            assert_eq!(state.names().get(label), Some(owner));
        }
        if height % 10 == 0 {
            let bytes = state.snapshot_bytes().unwrap();
            let restored = State::restore_checkpoint(
                state.genesis().clone(),
                height,
                state.tip(),
                state.root(),
                &bytes,
            )
            .unwrap();
            assert_eq!(state, restored);
            assert_eq!(restored.root(), old.root());
        }
    }
}
#[test]
fn empty_height_transitions_invalidate_cache_and_parallel_readers_agree() {
    let mut state = State::new(Genesis::local()).unwrap();
    let mut old = Reference::new(Genesis::local());
    for _ in 0..16 {
        let previous = state.clone();
        let root = previous.root();
        let block = state.propose(vec![]).unwrap();
        state = state.execute(&block).unwrap();
        old = old.execute(&block).unwrap();
        assert_ne!(state.root(), root);
        assert_eq!(state.root(), old.root());
        assert_eq!(previous.root(), root);
    }
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let state = &state;
            scope.spawn(move || {
                let root = state.root();
                for _ in 0..1000 {
                    assert_eq!(state.root(), root);
                    assert_eq!(state.clone(), *state);
                }
            });
        }
    });
}
