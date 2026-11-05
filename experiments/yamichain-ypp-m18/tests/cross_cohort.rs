//! Executable accounting test of the REJECTED candidate. Not a payment API.
//! Reuses the base experiment's complete proof statement and local trusted setup.
use ark_bls12_381::Bls12_381;
use ark_ec::CurveGroup;
use ark_ed_on_bls12_381::{EdwardsAffine as Point, Fr as Scalar};
use ark_ff::{Field, Zero};
use ark_groth16::{Proof, VerifyingKey};
use rand::{rngs::StdRng, SeedableRng};
use std::time::Instant;
use yamichain_ypp_accounts::{
    bytes, decode, encrypt, fixture, g, prove, random, setup, verify, Cipher, Ledger, Relation,
    Statement,
};
use yamichain_ypp_m18::{checkpoint, hash, select, Hash, Path, Tree};
#[derive(Clone)]
struct Record {
    key: Point,
    settled: Cipher,
    pending: Cipher,
    last: u64,
}
fn add(a: Cipher, b: Cipher) -> Cipher {
    ((a.0 + b.0).into_affine(), (a.1 + b.1).into_affine())
}
fn record_hash(r: &Record) -> Hash {
    hash(
        b"record-v1",
        &[
            &bytes(&r.key),
            &bytes(&r.settled),
            &bytes(&r.pending),
            &r.last.to_le_bytes(),
        ],
    )
}
fn head(epoch: u64, account_root: Hash, tags_root: Hash) -> Hash {
    hash(
        b"lab-head-v1",
        &[&checkpoint(epoch, account_root), &tags_root],
    )
}
#[derive(Clone)]
struct Harness {
    epoch: u64,
    records: Vec<Record>,
    tags: Vec<Point>,
    tree: Tree,
}
impl Harness {
    fn new() -> (Self, Vec<Scalar>) {
        let (l, x) = fixture(64);
        let records = l
            .keys
            .iter()
            .zip(l.settled)
            .map(|(key, settled)| Record {
                key: *key,
                settled,
                pending: (Point::zero(), Point::zero()),
                last: 0,
            })
            .collect::<Vec<_>>();
        let tree = Tree::new(&records.iter().map(record_hash).collect::<Vec<_>>());
        (
            Self {
                epoch: 1,
                records,
                tags: vec![],
                tree,
            },
            x,
        )
    }
    fn tags_root(&self) -> Hash {
        let mut t = self.tags.iter().map(bytes).collect::<Vec<_>>();
        t.sort_unstable();
        hash(b"tags-v1", &[&t.concat()])
    }
    fn head(&self) -> Hash {
        head(self.epoch, self.tree.root(), self.tags_root())
    }
    fn view(&self, i: usize) -> Cipher {
        let r = &self.records[i];
        if r.last < self.epoch {
            add(r.settled, r.pending)
        } else {
            r.settled
        }
    }
    fn relation(
        &self,
        ids: &[usize],
        s: usize,
        t: usize,
        x: Scalar,
        balance: u32,
        a: u32,
    ) -> Relation {
        let p = Ledger {
            epoch: self.epoch,
            keys: ids.iter().map(|i| self.records[*i].key).collect(),
            settled: ids.iter().map(|i| self.view(*i)).collect(),
            pending: vec![(Point::zero(), Point::zero()); ids.len()],
            tags: vec![],
        };
        p.relation(
            ids.iter().position(|i| *i == s).unwrap(),
            ids.iter().position(|i| *i == t).unwrap(),
            x,
            balance,
            a,
        )
    }
    fn apply(
        &mut self,
        vk: &VerifyingKey<Bls12_381>,
        ids: &[usize],
        s: &Statement,
        p: &Proof<Bls12_381>,
    ) -> Result<(), String> {
        if ids.len() != 32
            || ids.iter().any(|i| *i >= self.records.len())
            || !ids.windows(2).all(|w| w[0] < w[1])
            || ids
                .chunks(8)
                .any(|c| c[0] % 8 != 0 || c.iter().enumerate().any(|(j, i)| *i != c[0] + j))
        {
            return Err("bad public support".into());
        }
        if s.epoch != self.epoch
            || self.tags.contains(&s.tag)
            || s.keys != ids.iter().map(|i| self.records[*i].key).collect::<Vec<_>>()
            || s.old != ids.iter().map(|i| self.view(*i)).collect::<Vec<_>>()
            || !verify(vk, s, p)
        {
            return Err("invalid proof/state or conflicting debit".into());
        }
        // All fallible checks precede mutation. Stage only selected records, O(m).
        let staged = ids
            .iter()
            .zip(&s.changes)
            .map(|(i, c)| {
                let mut r = self.records[*i].clone();
                if r.last < self.epoch {
                    r.settled = add(r.settled, r.pending);
                    r.pending = (Point::zero(), Point::zero());
                    r.last = self.epoch;
                }
                r.pending = add(r.pending, (*c, s.d));
                (*i, r)
            })
            .collect::<Vec<_>>();
        for (i, r) in staged {
            self.tree.update(i, record_hash(&r));
            self.records[i] = r;
        }
        self.tags.push(s.tag);
        Ok(())
    }
    fn next_epoch(&mut self) {
        self.epoch += 1;
        self.tags.clear();
    } // No account/tree scan.
}
fn offline_recover(
    expected: Hash,
    epoch: u64,
    account_root: Hash,
    tags_root: Hash,
    r: &Record,
    path: &Path,
    x: Scalar,
) -> Result<u32, String> {
    if expected != head(epoch, account_root, tags_root)
        || !path.verify(account_root, record_hash(r))
        || r.key != (g() * x).into_affine()
    {
        return Err("bad authenticated state".into());
    }
    let c = add(r.settled, r.pending);
    decode((c.0 - c.1 * x).into_affine(), 128_000).ok_or("bad range".into())
}
#[test]
fn real_proofs_sparse_atomic_cross_cohort_offline_and_global_debit_tag() {
    let (mut h, x) = Harness::new();
    let mut rng = StdRng::seed_from_u64(18);
    let a_ids = select(64, 8, 4, 0, 17, &mut rng);
    let b_ids = select(64, 8, 4, 32, 17, &mut rng);
    let conflict_ids = select(64, 8, 4, 0, 40, &mut rng);
    assert_ne!(a_ids, conflict_ids);
    let a = h.relation(&a_ids, 0, 17, x[0], 1000, 7);
    let b = h.relation(&b_ids, 32, 17, x[32], 1000, 9);
    let conflict = h.relation(&conflict_ids, 0, 40, x[0], 1000, 11);
    assert_eq!(a.statement.tag, conflict.statement.tag);
    let start = Instant::now();
    let pk = setup(a.clone());
    let setup_ms = start.elapsed().as_secs_f64() * 1000.;
    let start = Instant::now();
    let pa = prove(&pk, a.clone());
    let prove_ms = start.elapsed().as_secs_f64() * 1000.;
    let start = Instant::now();
    assert!(verify(&pk.vk, &a.statement, &pa));
    let verify_ms = start.elapsed().as_secs_f64() * 1000.;
    let pb = prove(&pk, b.clone());
    let pc = prove(&pk, conflict.clone());
    assert!(verify(&pk.vk, &conflict.statement, &pc));
    let before = h.head();
    let records_before = h.records.iter().map(record_hash).collect::<Vec<_>>();
    let mut altered = a.statement.clone();
    altered.changes[0] = (altered.changes[0] + g()).into_affine();
    assert!(h.apply(&pk.vk, &a_ids, &altered, &pa).is_err());
    assert_eq!(before, h.head());
    let mut relabeled = a_ids.clone();
    relabeled.swap(0, 1);
    assert!(h.apply(&pk.vk, &relabeled, &a.statement, &pa).is_err());
    assert_eq!(before, h.head());
    let mut reverse = h.clone();
    reverse.apply(&pk.vk, &b_ids, &b.statement, &pb).unwrap();
    reverse.apply(&pk.vk, &a_ids, &a.statement, &pa).unwrap();
    h.apply(&pk.vk, &a_ids, &a.statement, &pa).unwrap();
    h.apply(&pk.vk, &b_ids, &b.statement, &pb).unwrap();
    assert_eq!(h.head(), reverse.head());
    assert!(h
        .apply(&pk.vk, &conflict_ids, &conflict.statement, &pc)
        .is_err());
    assert!(h.apply(&pk.vk, &a_ids, &a.statement, &pa).is_err());
    for (i, old) in records_before.iter().enumerate() {
        if !a_ids.contains(&i) && !b_ids.contains(&i) {
            assert_eq!(*old, record_hash(&h.records[i]));
        }
    }
    let finalized = h.head();
    let path = h.tree.path(17);
    let start = Instant::now();
    assert_eq!(
        offline_recover(
            finalized,
            h.epoch,
            h.tree.root(),
            h.tags_root(),
            &h.records[17],
            &path,
            x[17]
        )
        .unwrap(),
        1016
    );
    let recovery_ms = start.elapsed().as_secs_f64() * 1000.;
    assert!(offline_recover(
        finalized,
        h.epoch + 1,
        h.tree.root(),
        h.tags_root(),
        &h.records[17],
        &path,
        x[17]
    )
    .is_err());
    assert!(offline_recover(
        finalized,
        h.epoch,
        h.tree.root(),
        h.tags_root(),
        &h.records[17],
        &h.tree.path(18),
        x[17]
    )
    .is_err());
    let mut forged = h.records[17].clone();
    forged.pending.0 = (forged.pending.0 + g()).into_affine();
    assert!(offline_recover(
        finalized,
        h.epoch,
        h.tree.root(),
        h.tags_root(),
        &forged,
        &path,
        x[17]
    )
    .is_err());
    let record_hashes = h.records.iter().map(record_hash).collect::<Vec<_>>();
    h.next_epoch();
    assert_eq!(
        record_hashes,
        h.records.iter().map(record_hash).collect::<Vec<_>>()
    );
    assert!(h.apply(&pk.vk, &a_ids, &a.statement, &pa).is_err());
    let ids = select(64, 8, 4, 17, 56, &mut rng);
    let c = h.relation(&ids, 17, 56, x[17], 1016, 1001);
    let p = prove(&pk, c.clone());
    h.apply(&pk.vk, &ids, &c.statement, &p).unwrap();
    assert_eq!(
        offline_recover(
            h.head(),
            h.epoch,
            h.tree.root(),
            h.tags_root(),
            &h.records[17],
            &h.tree.path(17),
            x[17]
        )
        .unwrap(),
        15
    );
    let memory_high_water = std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find(|s| s.starts_with("VmHWM:"))
        .unwrap()
        .to_owned();
    let metrics = serde_json::json!({"memory_high_water":memory_high_water,"registered_curve_accounts":64,"cohort_width":8,"selected_cohorts":4,"touched_accounts":32,"real_proofs_generated":4,"setup_ms":setup_ms,"first_prove_ms":prove_ms,"verify_ms":verify_ms,"offline_recovery_ms":recovery_ms,"proof_bytes":bytes(&pa).len(),"statement_proof_and_u64_ids_bytes":bytes(&a.statement).len()+bytes(&pa).len()+8*a_ids.len(),"account_record_bytes":32+64+64+8,"merkle_siblings_bytes":path.siblings.len()*32,"uses_base_groth16_trusted_setup":true,"candidate_accepted":false});
    if let Ok(path) = std::env::var("M18_CRYPTO_REPORT") {
        std::fs::write(path, format!("{metrics}\n")).unwrap();
    }
    println!("{metrics}");
}
#[test]
fn real_observer_elimination_sybil_possession_and_limb_carry_blocker() {
    let (l, x) = fixture(64);
    let r = l.relation(0, 17, x[0], 1000, 7);
    for (i, key) in x.iter().enumerate() {
        if i != 0 && i != 17 {
            assert!((r.statement.changes[i] - r.statement.d * key).is_zero());
        }
    }
    // Valid possession proves knowledge of each key; all keys can have one owner.
    let registrations = (0..128)
        .map(|_| yamichain_ypp_accounts::Registration::create(random()))
        .collect::<Vec<_>>();
    assert!(registrations.iter().all(|p| p.verify()));
    let mut forged = registrations[0].clone();
    forged.key = registrations[1].key;
    assert!(!forged.verify());
    // Standalone 64-bit limb recovery works; offline recipient carry is not solved.
    let x = random();
    let y = (g() * x).into_affine();
    let limbs = [u16::MAX; 4].map(|v| encrypt(y, Scalar::from(v), random()));
    let mut full = 0u64;
    for (i, c) in limbs.iter().enumerate() {
        let v = decode((c.0 - c.1 * x).into_affine(), u16::MAX as u32).unwrap();
        full |= (v as u64) << (16 * i);
    }
    assert_eq!(full, u64::MAX);
    // Independent receiver balance 65535 (upper limbs zero), credit 1: no u64 overflow.
    let low = encrypt(y, Scalar::from(u16::MAX), random());
    let c = add(low, encrypt(y, Scalar::from(1u64), random()));
    assert_eq!(decode((c.0 - c.1 * x).into_affine(), u16::MAX as u32), None);
    assert_eq!(decode((c.0 - c.1 * x).into_affine(), 65536), Some(65536));
}

#[test]
fn related_key_registration_requires_actual_possession() {
    let x = random();
    let y = (g() * x).into_affine();
    let a = Scalar::from(7u64);
    let rogue = (y * a).into_affine();
    let r = random();
    let target_update = encrypt(y, Scalar::from(19u64), r).0;
    let rogue_zero = encrypt(rogue, Scalar::zero(), r).0;
    let exposed = (target_update - rogue_zero * a.inverse().unwrap()).into_affine();
    assert_eq!(decode(exposed, 100), Some(19));
    let mut forged = yamichain_ypp_accounts::Registration::create(random());
    forged.key = rogue;
    assert!(!forged.verify());
}
