//! Four OS-process end-to-end tests. Parent is wallet/router/acceptor, never execution oracle.
use ed25519_dalek::SigningKey;
use rand::{rngs::OsRng, RngCore};
use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command as Process, Stdio},
    sync::{Arc, Mutex},
};
use x25519_dalek::{PublicKey, StaticSecret};
use yamichain_tee_m20::{
    quorum::*,
    software_harness::{Command, Reply, SoftwareConfig},
    *,
};
fn random() -> Id {
    let mut x = [0; 32];
    OsRng.fill_bytes(&mut x);
    x
}
fn wallet() -> SigningKey {
    SigningKey::generate(&mut OsRng)
}
fn signed(a: &SigningKey, b: &SigningKey, nonce: u64, amount: u64) -> Signed {
    Signed::sign(
        Transfer {
            finance: None,

            sender: a.verifying_key().to_bytes(),
            recipient: b.verifying_key().to_bytes(),
            amount,
            nonce,
            chain: 21,
        },
        a,
    )
}
struct Worker {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}
impl Worker {
    fn start(config: &Path) -> Self {
        let mut child = Process::new(env!("CARGO_BIN_EXE_quorum-worker"))
            .arg(config)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            input,
            output,
        }
    }
    fn call(&mut self, c: Command) -> Reply {
        serde_json::to_writer(&mut self.input, &c).unwrap();
        writeln!(self.input).unwrap();
        self.input.flush().unwrap();
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        assert!(!line.is_empty(), "worker exited");
        serde_json::from_str(&line).unwrap()
    }
    fn head(&mut self) -> Head {
        match self.call(Command::Head) {
            Reply::Head(h) => h,
            _ => panic!("head"),
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
struct Lab {
    _dir: tempfile::TempDir,
    configs: Vec<SoftwareConfig>,
    paths: Vec<PathBuf>,
    workers: Vec<Worker>,
    committee: Committee,
    acceptor: Acceptor,
    alice: SigningKey,
    bob: SigningKey,
    genesis: State,
    senders: Vec<SigningKey>,
    root: Id,
    accept_path: PathBuf,
    accept_anchor: PathBuf,
}
impl Drop for Lab {
    fn drop(&mut self) {
        // Stop every child before TempDir removes private configs, state and anchors.
        for worker in &mut self.workers {
            let _ = worker.child.kill();
            let _ = worker.child.wait();
        }
    }
}
impl Lab {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = random();
        let alice = wallet();
        let bob = wallet();
        let secrets: Vec<_> = (0..4)
            .map(|_| (wallet(), StaticSecret::random_from_rng(OsRng), random()))
            .collect();
        let committee = Committee {
            chain: 21,
            members: std::array::from_fn(|i| Member {
                signing: secrets[i].0.verifying_key().to_bytes(),
                transport: PublicKey::from(&secrets[i].1).to_bytes(),
            }),
        };
        let senders: Vec<_> = (0..8).map(|_| wallet()).collect();
        let mut accounts = BTreeMap::from([(
            alice.verifying_key().to_bytes(),
            Account {
                balance: 1000,
                nonce: 0,
            },
        )]);
        for sender in &senders {
            accounts.insert(
                sender.verifying_key().to_bytes(),
                Account {
                    balance: 100,
                    nonce: 0,
                },
            );
        }
        let genesis = State {
            economics: None,

            chain: 21,
            epoch: 0,
            accounts,
        };
        let head = state_head(&genesis, root);
        let mut configs = Vec::new();
        let mut paths = Vec::new();
        let mut workers = Vec::new();
        let host = dir.path().join("untrusted-host");
        let trusted = dir.path().join("independent-trusted-anchors");
        fs::create_dir(&host).unwrap();
        fs::create_dir(&trusted).unwrap();
        for (i, (signing, transport, storage)) in secrets.iter().enumerate() {
            let c = SoftwareConfig {
                snapshot: host.join(format!("replica-{i}.json")),
                trusted_anchor: trusted.join(format!("replica-{i}.json")),
                committee: committee.clone(),
                index: i,
                signing_secret: signing.to_bytes(),
                transport_secret: transport.to_bytes(),
                storage_secret: *storage,
                root_secret: root,
                compromised: i == 3,
            };
            drop(
                Replica::create(
                    &c.snapshot,
                    &c.trusted_anchor,
                    committee.clone(),
                    i,
                    c.keys(),
                    &genesis,
                    &head,
                )
                .unwrap(),
            );
            let path = dir.path().join(format!("private-config-{i}.json"));
            private_file(&path, &bytes(&c));
            workers.push(Worker::start(&path));
            paths.push(path);
            configs.push(c);
        }
        let accept_path = host.join("accepted.json");
        let accept_anchor = trusted.join("accepted.json");
        let acceptor =
            Acceptor::create(&accept_path, &accept_anchor, committee.clone(), head).unwrap();
        Self {
            _dir: dir,
            configs,
            paths,
            workers,
            committee,
            acceptor,
            alice,
            bob,
            genesis,
            senders,
            root,
            accept_path,
            accept_anchor,
        }
    }
    fn packet(&self, nonce: u64, amount: u64) -> Payment {
        Payment::software(
            &self.committee,
            &self.acceptor.head(),
            signed(&self.alice, &self.bob, nonce, amount),
        )
        .unwrap()
    }
    fn votes(&mut self, p: &Payment) -> Vec<Approval> {
        std::thread::scope(|scope| {
            let jobs: Vec<_> = self
                .workers
                .iter_mut()
                .map(|worker| {
                    scope.spawn(move || worker.call(Command::Prepare(Box::new(p.clone()))))
                })
                .collect();
            jobs.into_iter()
                .filter_map(|job| match job.join().unwrap() {
                    Reply::Approval(v) => Some(v),
                    Reply::Rejected => None,
                    _ => panic!("prepare reply"),
                })
                .collect()
        })
    }

    fn accept(&mut self, p: &Payment) -> Certificate {
        let votes = self.votes(p);
        assert_eq!(votes.len(), 4);
        let qc = Certificate::collect(&self.committee, &self.acceptor.head(), votes).unwrap();
        self.acceptor.accept(&qc).unwrap();
        self.commit_all(&qc);
        qc
    }
    fn commit_all(&mut self, qc: &Certificate) {
        for w in &mut self.workers {
            assert!(matches!(
                w.call(Command::Commit(qc.clone())),
                Reply::Committed
            ));
            assert_eq!(w.head(), qc.statement.next);
        }
    }
    fn restart(&mut self, i: usize) {
        let mut old = self.workers.remove(i);
        old.child.kill().unwrap();
        old.child.wait().unwrap();
        drop(old);
        self.workers.insert(i, Worker::start(&self.paths[i]));
    }
    fn query(&mut self, who: &SigningKey) -> (Query, StaticSecret, Vec<Envelope>) {
        let secret = StaticSecret::random_from_rng(OsRng);
        let mut q = Query {
            account: who.verifying_key().to_bytes(),
            reply_key: PublicKey::from(&secret).to_bytes(),
            head: self.acceptor.head(),
            challenge: random(),
            signature: vec![],
        };
        q.sign(who);
        let replies = self
            .workers
            .iter_mut()
            .enumerate()
            .map(|(i, w)| {
                let envelope = encrypt(
                    self.committee.members[i].transport,
                    &bytes(&q),
                    QUERY_DOMAIN,
                )
                .unwrap();
                match w.call(Command::Query(envelope)) {
                    Reply::Balance(e) => e,
                    _ => panic!("query"),
                }
            })
            .collect();
        (q, secret, replies)
    }
}
fn private_file(path: &Path, data: &[u8]) {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    f.write_all(data).unwrap();
}

#[test]
fn four_processes_offline_receive_concurrency_restart_and_recovery() {
    let mut lab = Lab::new();
    let ids: std::collections::BTreeSet<_> = lab.workers.iter().map(|w| w.child.id()).collect();
    assert_eq!(ids.len(), 4);
    println!("M21 SOFTWARE: four independent OS executor PIDs: {ids:?}; distinct signing and transport keys; no TDX");
    let payment = lab.packet(0, 100);
    let qc = lab.accept(&payment);
    assert!(lab.acceptor.accept(&qc).is_err());
    // Recipient has been absent throughout payment, now authenticates confidential queries.
    let bob = lab.bob.clone();
    let (q, secret, replies) = lab.query(&bob);
    assert_eq!(
        verify_balance(&lab.committee, &q, &secret, &replies)
            .unwrap()
            .balance,
        100
    );
    assert!(verify_balance(
        &lab.committee,
        &q,
        &secret,
        &[replies[0].clone(), replies[0].clone(), replies[1].clone()]
    )
    .is_err());
    let mut changed = q.clone();
    changed.challenge = random();
    changed.sign(&bob);
    assert!(verify_balance(&lab.committee, &changed, &secret, &replies).is_err());
    for i in 0..4 {
        lab.restart(i);
        assert_eq!(lab.workers[i].head(), lab.acceptor.head());
    }
    // Concurrent callers race same wallet nonce through one serialized quorum router.
    let shared = Arc::new(Mutex::new(lab));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let l = shared.clone();
            std::thread::spawn(move || {
                let mut lab = l.lock().unwrap();
                let p = lab.packet(1, 1);
                let votes = lab.votes(&p);
                if let Ok(qc) = Certificate::collect(&lab.committee, &lab.acceptor.head(), votes) {
                    lab.acceptor.accept(&qc).unwrap();
                    lab.commit_all(&qc);
                    true
                } else {
                    false
                }
            })
        })
        .collect();
    assert_eq!(
        threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .filter(|v| *v)
            .count(),
        1
    );
    let mut lab = Arc::try_unwrap(shared).ok().unwrap().into_inner().unwrap();
    let (q, secret, replies) = lab.query(&bob);
    assert_eq!(
        verify_balance(&lab.committee, &q, &secret, &replies)
            .unwrap()
            .balance,
        101
    );
    // Coordinator persisted a certificate but died before sending Commit to replicas.
    let p = lab.packet(2, 20);
    let votes = lab.votes(&p);
    let qc = Certificate::collect(&lab.committee, &lab.acceptor.head(), votes).unwrap();
    lab.acceptor.accept(&qc).unwrap();
    let dummy_dir = tempfile::tempdir().unwrap();
    let dummy = Acceptor::create(
        &dummy_dir.path().join("s"),
        &dummy_dir.path().join("a"),
        lab.committee.clone(),
        state_head(&lab.genesis, lab.root),
    )
    .unwrap();
    drop(std::mem::replace(&mut lab.acceptor, dummy));
    lab.acceptor =
        Acceptor::load(&lab.accept_path, &lab.accept_anchor, lab.committee.clone()).unwrap();
    for i in 0..4 {
        lab.restart(i);
    }
    let recovered = lab.acceptor.certificate().unwrap();
    lab.commit_all(&recovered);
    lab.commit_all(&recovered);
    let (q, secret, replies) = lab.query(&bob);
    assert_eq!(
        verify_balance(&lab.committee, &q, &secret, &replies)
            .unwrap()
            .balance,
        121
    );
}

#[test]
fn one_malicious_process_cannot_inflate_steal_forge_or_equivocate() {
    let mut lab = Lab::new();
    let old = lab.acceptor.head();
    let p = lab.packet(0, 10);
    let honest: Vec<_> = lab.workers[..3]
        .iter_mut()
        .map(|w| match w.call(Command::Prepare(Box::new(p.clone()))) {
            Reply::Approval(a) => a,
            _ => panic!("honest prepare"),
        })
        .collect();
    assert!(honest.iter().all(|a| a.statement == honest[0].statement));
    let authorized = transition(&lab.genesis, &signed(&lab.alice, &lab.bob, 0, 10)).unwrap();
    let mut attacks = Vec::new();
    // Each attack computes an actual forged ledger root, then compromised OS worker signs it.
    let mut inflated = authorized.clone();
    inflated
        .accounts
        .get_mut(&lab.bob.verifying_key().to_bytes())
        .unwrap()
        .balance += 100;
    attacks.push(state_head(&inflated, lab.root));
    let mut stolen = authorized.clone();
    stolen
        .accounts
        .get_mut(&lab.alice.verifying_key().to_bytes())
        .unwrap()
        .balance -= 100;
    stolen
        .accounts
        .get_mut(&lab.bob.verifying_key().to_bytes())
        .unwrap()
        .balance += 100;
    attacks.push(state_head(&stolen, lab.root));
    let mut forged = authorized.clone();
    forged
        .accounts
        .get_mut(&lab.bob.verifying_key().to_bytes())
        .unwrap()
        .balance = 999_999;
    attacks.push(state_head(&forged, lab.root));
    let mut malicious = Vec::new();
    for next in attacks {
        let mut statement = honest[0].statement.clone();
        statement.next = next;
        let a = match lab.workers[3].call(Command::CompromisedSign(statement)) {
            Reply::Approval(a) => a,
            _ => panic!("compromised signing"),
        };
        a.verify(&lab.committee, &old).unwrap();
        assert!(
            Certificate::collect(&lab.committee, &old, vec![a.clone(), a.clone(), a.clone()])
                .is_err()
        );
        let invalid = Certificate {
            statement: a.statement.clone(),
            approvals: vec![a.clone(), honest[0].clone(), honest[1].clone()],
        };
        assert!(lab.acceptor.accept(&invalid).is_err());
        assert_eq!(lab.acceptor.head(), old);
        for w in &mut lab.workers {
            assert!(matches!(
                w.call(Command::Commit(invalid.clone())),
                Reply::Rejected
            ));
            assert_eq!(w.head(), old);
        }
        malicious.push(a);
    }
    // Same malicious signer equivocated over multiple states; only the three honest signatures form a QC.
    let qc = Certificate::collect(
        &lab.committee,
        &old,
        honest.iter().cloned().chain(malicious).collect(),
    )
    .unwrap();
    assert_eq!(qc.statement.next, state_head(&authorized, lab.root));
    lab.acceptor.accept(&qc).unwrap();
    // Faulty member need not participate for acceptance or confidential balance recovery.
    for w in &mut lab.workers[..3] {
        assert!(matches!(
            w.call(Command::Commit(qc.clone())),
            Reply::Committed
        ));
    }
    let bob = lab.bob.clone();
    let secret = StaticSecret::random_from_rng(OsRng);
    let mut q = Query {
        account: bob.verifying_key().to_bytes(),
        reply_key: PublicKey::from(&secret).to_bytes(),
        head: lab.acceptor.head(),
        challenge: random(),
        signature: vec![],
    };
    q.sign(&bob);
    let mut replies = Vec::new();
    for i in 0..3 {
        let envelope =
            encrypt(lab.committee.members[i].transport, &bytes(&q), QUERY_DOMAIN).unwrap();
        match lab.workers[i].call(Command::Query(envelope)) {
            Reply::Balance(e) => replies.push(e),
            _ => panic!("reply"),
        }
    }
    assert_eq!(
        verify_balance(&lab.committee, &q, &secret, &replies)
            .unwrap()
            .balance,
        10
    );
}

#[test]
fn honest_sign_once_locks_survive_process_restart_and_prevent_conflicting_quorums() {
    let mut lab = Lab::new();
    let old = lab.acceptor.head();
    let a = lab.packet(0, 10);
    let b = lab.packet(0, 20);
    // Host partitions deliveries. Two honest replicas lock A, one honest replica locks B.
    let mut votes_a = Vec::new();
    for i in 0..2 {
        match lab.workers[i].call(Command::Prepare(Box::new(a.clone()))) {
            Reply::Approval(v) => votes_a.push(v),
            _ => panic!("A"),
        }
    }
    let vote_b = match lab.workers[2].call(Command::Prepare(Box::new(b.clone()))) {
        Reply::Approval(v) => v,
        _ => panic!("B"),
    };
    let mut fake_a = votes_a[0].statement.clone();
    let mut fake_b = vote_b.statement.clone();
    fake_a.transaction = a.transaction;
    fake_b.transaction = b.transaction;
    let evil_a = match lab.workers[3].call(Command::CompromisedSign(fake_a)) {
        Reply::Approval(v) => v,
        _ => panic!("evil A"),
    };
    let evil_b = match lab.workers[3].call(Command::CompromisedSign(fake_b)) {
        Reply::Approval(v) => v,
        _ => panic!("evil B"),
    };
    votes_a.push(evil_a);
    let qc_a = Certificate::collect(&lab.committee, &old, votes_a).unwrap();
    assert!(Certificate::collect(&lab.committee, &old, vec![vote_b, evil_b]).is_err());
    for i in 0..2 {
        lab.restart(i);
        assert!(matches!(
            lab.workers[i].call(Command::Prepare(Box::new(b.clone()))),
            Reply::Rejected
        ));
        assert!(matches!(
            lab.workers[i].call(Command::Prepare(Box::new(a.clone()))),
            Reply::Approval(_)
        ));
    }
    lab.acceptor.accept(&qc_a).unwrap();
    assert!(lab.acceptor.accept(&qc_a).is_err());
    assert!(matches!(
        lab.workers[2].call(Command::Commit(qc_a)),
        Reply::Rejected
    ));
    // Safe but no general view-change/catch-up for the replica locked to another proposal.
    assert_eq!(lab.workers[2].head(), old);
}

#[test]
fn malformed_delivery_invalid_wallet_and_unauthorized_queries_do_not_advance_state() {
    let mut lab = Lab::new();
    let old = lab.acceptor.head();
    for (nonce, amount) in [(0, 0), (0, 1001), (1, 1)] {
        let p = lab.packet(nonce, amount);
        assert!(lab.votes(&p).is_empty());
        assert_eq!(lab.acceptor.head(), old);
        for w in &mut lab.workers {
            assert_eq!(w.head(), old);
        }
    }
    let mut p = lab.packet(0, 1);
    for e in p.envelopes.iter_mut().flatten() {
        e.ciphertext[0] ^= 1;
    }
    assert!(lab.votes(&p).is_empty());
    // Correctly encrypted/committed payload with a forged wallet signature: every process rejects it.
    #[derive(serde::Serialize)]
    struct RawDelivery {
        transfer: Signed,
        blind: Id,
    }
    let mut transfer = signed(&lab.alice, &lab.bob, 0, 1);
    transfer.signature[0] ^= 1;
    let d = RawDelivery {
        transfer,
        blind: random(),
    };
    let transaction = hash(&bytes(&("YAMI-M21-blinded-transaction-v1", &d)));
    let context = bytes(&(PAYMENT_DOMAIN, lab.committee.id(), &old, transaction));
    let envelopes = std::array::from_fn(|i| {
        encrypt(lab.committee.members[i].transport, &bytes(&d), &context).unwrap()
    });
    let forged = Payment {
        committee: lab.committee.id(),
        previous: old.clone(),
        transaction,
        envelopes: envelopes.map(Some),
    };
    assert!(lab.votes(&forged).is_empty());
    let reply = StaticSecret::random_from_rng(OsRng);
    let mut q = Query {
        account: lab.bob.verifying_key().to_bytes(),
        reply_key: PublicKey::from(&reply).to_bytes(),
        head: old.clone(),
        challenge: random(),
        signature: vec![],
    };
    q.sign(&lab.alice);
    for i in 0..4 {
        let e = encrypt(lab.committee.members[i].transport, &bytes(&q), QUERY_DOMAIN).unwrap();
        assert!(matches!(
            lab.workers[i].call(Command::Query(e)),
            Reply::Rejected
        ));
    }
    let valid = lab.packet(0, 10);
    let qc = lab.accept(&valid);
    assert!(lab.votes(&valid).is_empty());
    assert!(lab.acceptor.accept(&qc).is_err());
    q.sign(&lab.bob);
    for i in 0..4 {
        let e = encrypt(lab.committee.members[i].transport, &bytes(&q), QUERY_DOMAIN).unwrap();
        assert!(matches!(
            lab.workers[i].call(Command::Query(e)),
            Reply::Rejected
        ));
    }
}

#[test]
fn certificates_bind_every_field_and_require_distinct_pinned_signers() {
    let mut lab = Lab::new();
    let p = lab.packet(0, 10);
    let votes = lab.votes(&p);
    let previous = lab.acceptor.head();
    let qc = Certificate::collect(&lab.committee, &previous, votes).unwrap();
    for field in 0..7 {
        let mut bad = qc.clone();
        match field {
            0 => bad.statement.committee = random(),
            1 => bad.statement.previous.chain += 1,
            2 => bad.statement.previous.epoch += 1,
            3 => bad.statement.previous.commitment = random(),
            4 => bad.statement.next.epoch += 1,
            5 => bad.statement.next.commitment = random(),
            _ => bad.statement.transaction = random(),
        };
        for a in &mut bad.approvals {
            a.statement = bad.statement.clone();
        }
        assert!(lab.acceptor.accept(&bad).is_err());
        assert_eq!(lab.acceptor.head(), previous);
    }
    let mut duplicate = qc.clone();
    duplicate.approvals = vec![qc.approvals[0].clone(); 3];
    assert!(lab.acceptor.accept(&duplicate).is_err());
    let mut unknown = qc.clone();
    unknown.approvals[0].member = random();
    assert!(lab.acceptor.accept(&unknown).is_err());
    let mut two = qc.clone();
    two.approvals.truncate(2);
    assert!(lab.acceptor.accept(&two).is_err());
    let mut wrong_committee = lab.committee.clone();
    wrong_committee.members[1].signing = wrong_committee.members[0].signing;
    assert!(qc.verify(&wrong_committee, &previous).is_err());
    lab.acceptor.accept(&qc).unwrap();
    assert!(lab.acceptor.accept(&qc).is_err());
    lab.commit_all(&qc);
}

#[test]
fn encrypted_peer_recovery_resolves_pending_conflict_and_refuses_wrong_state_or_replay() {
    let mut lab = Lab::new();
    let old = lab.acceptor.head();
    let p = lab.packet(0, 10);
    let competing = lab.packet(0, 20);
    assert!(matches!(
        lab.workers[3].call(Command::Prepare(Box::new(competing))),
        Reply::Approval(_)
    ));
    let votes = lab.votes(&p);
    assert_eq!(votes.len(), 3);
    let qc = Certificate::collect(&lab.committee, &old, votes).unwrap();
    lab.acceptor.accept(&qc).unwrap();
    for i in 0..3 {
        assert!(matches!(
            lab.workers[i].call(Command::Commit(qc.clone())),
            Reply::Committed
        ));
    }
    lab.restart(3);
    let recovery = match lab.workers[0].call(Command::ExportRecovery(3)) {
        Reply::Recovery(e) => e,
        _ => panic!("export"),
    };
    let mut altered = recovery.clone();
    altered.ciphertext[0] ^= 1;
    assert!(matches!(
        lab.workers[3].call(Command::Recover(altered)),
        Reply::Rejected
    ));
    assert_eq!(lab.workers[3].head(), old);
    assert!(matches!(
        lab.workers[2].call(Command::Recover(recovery.clone())),
        Reply::Rejected
    ));
    let mut forged_state = transition(&lab.genesis, &signed(&lab.alice, &lab.bob, 0, 10)).unwrap();
    forged_state
        .accounts
        .get_mut(&lab.bob.verifying_key().to_bytes())
        .unwrap()
        .balance += 500;
    let entries: Vec<_> = forged_state
        .accounts
        .iter()
        .map(|(id, a)| (*id, a.clone()))
        .collect();
    let counterfeit = serde_json::json!({"committee":lab.committee.id(),"state":(forged_state.chain,forged_state.epoch,entries),"certificate":qc});
    let counterfeit = encrypt(
        lab.committee.members[3].transport,
        &bytes(&counterfeit),
        b"YAMI-M21-recovery-v1",
    )
    .unwrap();
    assert!(matches!(
        lab.workers[3].call(Command::Recover(counterfeit)),
        Reply::Rejected
    ));
    assert_eq!(lab.workers[3].head(), old);
    assert!(matches!(
        lab.workers[3].call(Command::Recover(recovery.clone())),
        Reply::Committed
    ));
    assert_eq!(lab.workers[3].head(), lab.acceptor.head());
    let p = lab.packet(1, 1);
    lab.accept(&p);
    assert!(matches!(
        lab.workers[3].call(Command::Recover(recovery)),
        Reply::Rejected
    ));
    assert_eq!(lab.workers[3].head(), lab.acceptor.head());
}

#[test]
fn trusted_anchors_detect_snapshot_and_acceptor_rollback_after_process_death() {
    let mut lab = Lab::new();
    let genesis_snapshot = fs::read(&lab.configs[0].snapshot).unwrap();
    let genesis_acceptor = fs::read(&lab.accept_path).unwrap();
    let p = lab.packet(0, 10);
    let votes = lab.votes(&p);
    let prepared_snapshot = fs::read(&lab.configs[0].snapshot).unwrap();
    lab.workers[0].child.kill().unwrap();
    lab.workers[0].child.wait().unwrap();
    // Rewinding just a prepared vote snapshot must not remove its sign-once lock.
    fs::write(&lab.configs[0].snapshot, &genesis_snapshot).unwrap();
    let c = &lab.configs[0];
    assert!(Replica::load(
        &c.snapshot,
        &c.trusted_anchor,
        lab.committee.clone(),
        0,
        c.keys()
    )
    .is_err());
    fs::write(&c.snapshot, &prepared_snapshot).unwrap();
    lab.restart(0);
    let qc = Certificate::collect(&lab.committee, &lab.acceptor.head(), votes).unwrap();
    lab.acceptor.accept(&qc).unwrap();
    lab.commit_all(&qc);
    let current = fs::read(&lab.configs[0].snapshot).unwrap();
    lab.workers[0].child.kill().unwrap();
    lab.workers[0].child.wait().unwrap();
    fs::write(&lab.configs[0].snapshot, prepared_snapshot).unwrap();
    let c = &lab.configs[0];
    assert!(Replica::load(
        &c.snapshot,
        &c.trusted_anchor,
        lab.committee.clone(),
        0,
        c.keys()
    )
    .is_err());
    fs::write(&c.snapshot, &current).unwrap();
    let mut corrupted = current.clone();
    corrupted[5] ^= 1;
    fs::write(&c.snapshot, corrupted).unwrap();
    assert!(Replica::load(
        &c.snapshot,
        &c.trusted_anchor,
        lab.committee.clone(),
        0,
        c.keys()
    )
    .is_err());
    fs::write(&c.snapshot, current).unwrap();
    lab.restart(0);
    // Drop acceptor's exclusive handles, then rewind its untrusted file only.
    let saved = fs::read(&lab.accept_path).unwrap();
    let dummy = tempfile::tempdir().unwrap();
    let replace = Acceptor::create(
        &dummy.path().join("s"),
        &dummy.path().join("a"),
        lab.committee.clone(),
        state_head(&lab.genesis, lab.root),
    )
    .unwrap();
    drop(std::mem::replace(&mut lab.acceptor, replace));
    fs::write(&lab.accept_path, genesis_acceptor).unwrap();
    assert!(Acceptor::load(&lab.accept_path, &lab.accept_anchor, lab.committee.clone()).is_err());
    fs::write(&lab.accept_path, saved).unwrap();
    lab.acceptor =
        Acceptor::load(&lab.accept_path, &lab.accept_anchor, lab.committee.clone()).unwrap();
    assert_eq!(lab.acceptor.head(), qc.statement.next);
}

#[test]
fn durable_prepare_and_commit_fault_boundaries_fail_closed() {
    for stage in ["snapshot_durable", "anchor_durable"] {
        let mut lab = Lab::new();
        lab.workers[0].child.kill().unwrap();
        lab.workers[0].child.wait().unwrap();
        let c = &lab.configs[0];
        let old_file = fs::read(&c.snapshot).unwrap();
        let p = lab.packet(0, 10);
        let competing = lab.packet(0, 20);
        let mut replica = Replica::load(
            &c.snapshot,
            &c.trusted_anchor,
            lab.committee.clone(),
            0,
            c.keys(),
        )
        .unwrap();
        assert!(replica
            .prepare_with_hook(&p, |s| if s == stage {
                Err("injected interruption".into())
            } else {
                Ok(())
            })
            .is_err());
        assert!(replica.prepare(&competing).is_err());
        drop(replica);
        if stage == "snapshot_durable" {
            assert!(Replica::load(
                &c.snapshot,
                &c.trusted_anchor,
                lab.committee.clone(),
                0,
                c.keys()
            )
            .is_err());
            fs::write(&c.snapshot, old_file).unwrap();
        }
        let mut replica = Replica::load(
            &c.snapshot,
            &c.trusted_anchor,
            lab.committee.clone(),
            0,
            c.keys(),
        )
        .unwrap();
        let vote = replica.prepare(&p).unwrap();
        assert!(replica.prepare(&competing).is_err());
        drop(replica);
        lab.restart(0);
        let mut votes = lab.votes(&p);
        votes.push(vote);
        let qc = Certificate::collect(&lab.committee, &lab.acceptor.head(), votes).unwrap();
        lab.acceptor.accept(&qc).unwrap();
        lab.workers[0].child.kill().unwrap();
        lab.workers[0].child.wait().unwrap();
        let c = &lab.configs[0];
        let before_commit = fs::read(&c.snapshot).unwrap();
        let mut replica = Replica::load(
            &c.snapshot,
            &c.trusted_anchor,
            lab.committee.clone(),
            0,
            c.keys(),
        )
        .unwrap();
        assert!(replica
            .commit_with_hook(&qc, |s| if s == stage {
                Err("injected interruption".into())
            } else {
                Ok(())
            })
            .is_err());
        drop(replica);
        if stage == "snapshot_durable" {
            assert!(Replica::load(
                &c.snapshot,
                &c.trusted_anchor,
                lab.committee.clone(),
                0,
                c.keys()
            )
            .is_err());
            fs::write(&c.snapshot, before_commit).unwrap();
        }
        let mut replica = Replica::load(
            &c.snapshot,
            &c.trusted_anchor,
            lab.committee.clone(),
            0,
            c.keys(),
        )
        .unwrap();
        replica.commit(&qc).unwrap();
        assert_eq!(replica.head().unwrap(), qc.statement.next);
        drop(replica);
    }
}

#[test]
fn tdx_binding_covers_roster_identity_transport_chain_genesis_and_challenge() {
    let lab = Lab::new();
    let head = lab.acceptor.head();
    let challenge = random();
    let identity = random();
    let expected = tdx_report_data(&lab.committee, 0, &head, identity, challenge).unwrap();
    assert_ne!(
        expected,
        tdx_report_data(&lab.committee, 1, &head, identity, challenge).unwrap()
    );
    assert_ne!(
        expected,
        tdx_report_data(&lab.committee, 0, &head, random(), challenge).unwrap()
    );
    assert_ne!(
        expected,
        tdx_report_data(&lab.committee, 0, &head, identity, random()).unwrap()
    );
    let mut c = lab.committee.clone();
    c.members[0].transport = PublicKey::from(&StaticSecret::random_from_rng(OsRng)).to_bytes();
    assert_ne!(
        expected,
        tdx_report_data(&c, 0, &head, identity, challenge).unwrap()
    );
    let mut different = head.clone();
    different.commitment = random();
    assert_ne!(
        expected,
        tdx_report_data(&lab.committee, 0, &different, identity, challenge).unwrap()
    );
    assert!(production_attestation_gate(&bytes(&expected.to_vec())).is_err());
    assert!(production_attestation_gate(&bytes(&SoftwareEvidence {
        measurement: random(),
        identity,
        key: c.members[0].transport,
        challenge,
        issued: 1,
        expires: 2
    }))
    .is_err());
    assert!(production_attestation_gate(&[]).is_err());
}

#[test]
fn concurrent_valid_transfers_all_credit_offline_recipient_exactly_once() {
    let lab = Lab::new();
    let senders = lab.senders.clone();
    let bob = lab.bob.clone();
    let shared = Arc::new(Mutex::new(lab));
    let jobs: Vec<_> = senders
        .into_iter()
        .map(|sender| {
            let l = shared.clone();
            let recipient = bob.clone();
            std::thread::spawn(move || {
                let transfer = signed(&sender, &recipient, 0, 10);
                let mut lab = l.lock().unwrap();
                let payment =
                    Payment::software(&lab.committee, &lab.acceptor.head(), transfer).unwrap();
                lab.accept(&payment);
            })
        })
        .collect();
    for j in jobs {
        j.join().unwrap();
    }
    let mut lab = Arc::try_unwrap(shared).ok().unwrap().into_inner().unwrap();
    assert_eq!(lab.acceptor.head().epoch, 8);
    let (q, secret, replies) = lab.query(&bob);
    assert_eq!(
        verify_balance(&lab.committee, &q, &secret, &replies)
            .unwrap()
            .balance,
        80
    );
    // Three honest query replies defeat a correctly signed false response by the fourth process.
    let fake = match lab.workers[3].call(Command::CompromisedBalance(
        q.clone(),
        Account {
            balance: 999999,
            nonce: 0,
        },
    )) {
        Reply::Balance(e) => e,
        _ => panic!("malicious reply"),
    };
    let mut with_outlier = replies[..3].to_vec();
    with_outlier.push(fake.clone());
    assert_eq!(
        verify_balance(&lab.committee, &q, &secret, &with_outlier)
            .unwrap()
            .balance,
        80
    );
    assert!(verify_balance(
        &lab.committee,
        &q,
        &secret,
        &[fake.clone(), fake.clone(), fake]
    )
    .is_err());
}

#[test]
fn rewinding_the_trusted_anchor_too_voids_the_software_rollback_guarantee() {
    // Deliberate violation of the trusted-anchor assumption, not an in-model safe rollback.
    let mut lab = Lab::new();
    let c = lab.configs[0].clone();
    let old_snapshot = fs::read(&c.snapshot).unwrap();
    let old_anchor = fs::read(&c.trusted_anchor).unwrap();
    let a = lab.packet(0, 10);
    let b = lab.packet(0, 20);
    let vote_a = match lab.workers[0].call(Command::Prepare(Box::new(a))) {
        Reply::Approval(v) => v,
        _ => panic!("first vote"),
    };
    lab.workers[0].child.kill().unwrap();
    lab.workers[0].child.wait().unwrap();
    fs::write(&c.snapshot, old_snapshot).unwrap();
    fs::write(&c.trusted_anchor, old_anchor).unwrap();
    lab.restart(0);
    let vote_b = match lab.workers[0].call(Command::Prepare(Box::new(b))) {
        Reply::Approval(v) => v,
        _ => panic!("rewound vote"),
    };
    assert_eq!(vote_a.member, vote_b.member);
    assert_ne!(vote_a.statement, vote_b.statement);
    vote_a.verify(&lab.committee, &lab.acceptor.head()).unwrap();
    vote_b.verify(&lab.committee, &lab.acceptor.head()).unwrap();
}
