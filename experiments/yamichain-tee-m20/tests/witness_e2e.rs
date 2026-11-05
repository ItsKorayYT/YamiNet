//! M23 SOFTWARE protocol: four actual executor processes and four TCP witness services.
//! Loopback separation is a test arrangement, not an independent-trust-domain claim.
use ed25519_dalek::SigningKey;
use rand::{rngs::OsRng, RngCore};
use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{Arc, Mutex},
};
use x25519_dalek::{PublicKey, StaticSecret};
use yamichain_tee_m20::{quorum::*, witness::*, *};
fn random() -> Id {
    let mut x = [0; 32];
    OsRng.fill_bytes(&mut x);
    x
}
fn key() -> SigningKey {
    SigningKey::generate(&mut OsRng)
}
fn private(path: &Path, data: &[u8]) {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    f.write_all(data).unwrap();
}
struct Node {
    child: Child,
    path: PathBuf,
    config: ServiceConfig,
    address: SocketAddr,
}
impl Node {
    fn start(path: PathBuf, mut config: ServiceConfig) -> Self {
        private(&path, &bytes(&config));
        let mut child = Command::new(env!("CARGO_BIN_EXE_witness-node"))
            .arg(&path)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut out = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        let address = line.trim().parse().expect("witness startup address");
        config.listen = address;
        Self {
            child,
            path,
            config,
            address,
        }
    }
    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
    fn restart(&mut self) {
        self.stop();
        let next = Self::start(self.path.clone(), self.config.clone());
        let mut next = next;
        std::mem::swap(self, &mut next);
    }
}
impl Drop for Node {
    fn drop(&mut self) {
        self.stop();
    }
}
struct Worker {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    path: PathBuf,
}
impl Worker {
    fn start(path: PathBuf) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_witnessed-worker"))
            .arg(&path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut w = Self {
            child,
            input,
            output,
            path,
        };
        assert!(matches!(w.call(WorkerCommand::Head), WorkerReply::Head(_)));
        w
    }
    fn call(&mut self, c: WorkerCommand) -> WorkerReply {
        serde_json::to_writer(&mut self.input, &c).unwrap();
        writeln!(self.input).unwrap();
        self.input.flush().unwrap();
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        assert!(!line.is_empty(), "worker exited");
        serde_json::from_str(&line).unwrap()
    }
    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
    fn restart(&mut self) {
        self.stop();
        let next = Self::start(self.path.clone());
        let mut next = next;
        std::mem::swap(self, &mut next);
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}
struct Lab {
    dir: tempfile::TempDir,
    nodes: Vec<Node>,
    workers: Vec<Worker>,
    configs: Vec<WorkerConfig>,
    observers: Vec<Client>,
    policy: WitnessPolicy,
    head: Head,
    alice: SigningKey,
    bob: SigningKey,
    senders: Vec<SigningKey>,
}
impl Drop for Lab {
    fn drop(&mut self) {
        for w in &mut self.workers {
            w.stop();
        }
        for n in &mut self.nodes {
            n.stop();
        }
    }
}
impl Lab {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let keys: Vec<_> = (0..4)
            .map(|_| ReplicaKeys {
                signing: key(),
                transport: StaticSecret::random_from_rng(OsRng),
                storage: random(),
                root_secret: [0; 32],
            })
            .collect();
        let authorities: [SigningKey; 4] = std::array::from_fn(|_| key());
        let observer = key();
        let root = random();
        let alice = key();
        let bob = key();
        let senders: Vec<_> = (0..8).map(|_| key()).collect();
        let committee = Committee {
            chain: 23,
            members: std::array::from_fn(|i| Member {
                signing: keys[i].signing.verifying_key().to_bytes(),
                transport: PublicKey::from(&keys[i].transport).to_bytes(),
            }),
        };
        let mut accounts = BTreeMap::from([(
            alice.verifying_key().to_bytes(),
            Account {
                balance: 1000,
                nonce: 0,
            },
        )]);
        for w in &senders {
            accounts.insert(
                w.verifying_key().to_bytes(),
                Account {
                    balance: 100,
                    nonce: 0,
                },
            );
        }
        let genesis = State {
            chain: 23,
            epoch: 0,
            accounts,
        };
        let head = state_head(&genesis, root);
        let policy = WitnessPolicy {
            committee,
            genesis: head.clone(),
            authorities: std::array::from_fn(|i| authorities[i].verifying_key().to_bytes()),
            observer: observer.verifying_key().to_bytes(),
        };
        let host = dir.path().join("executor-host-rollback-domain");
        let witness = dir.path().join("witness-authority-test-storage");
        fs::create_dir(&host).unwrap();
        fs::create_dir(&witness).unwrap();
        let mut nodes = Vec::new();
        let mut workers = Vec::new();
        let mut configs = Vec::new();
        let mut observers = Vec::new();
        for i in 0..4 {
            let config = ServiceConfig {
                listen: "127.0.0.1:0".parse().unwrap(),
                database: witness.join(format!("authority-{i}.sqlite")),
                policy: policy.clone(),
                index: i,
                signing_secret: authorities[i].to_bytes(),
                test_crash_after_prepare_commit: false,
            };
            let node = Node::start(dir.path().join(format!("witness-private-{i}.json")), config);
            let mut k = keys[i].clone();
            k.root_secret = root;
            let config = WorkerConfig {
                snapshot: host.join(format!("executor-{i}.json")),
                policy: policy.clone(),
                index: i,
                authority: node.address,
                signing_secret: k.signing.to_bytes(),
                transport_secret: k.transport.to_bytes(),
                storage_secret: k.storage,
                root_secret: root,
                software_protocol_only: true,
                fault_injection: true,
            };
            drop(
                WitnessedReplica::create(&config.snapshot, config.client().unwrap(), k, &genesis)
                    .unwrap(),
            );
            let path = dir.path().join(format!("executor-private-{i}.json"));
            private(&path, &bytes(&config));
            workers.push(Worker::start(path));
            observers.push(Client::new(node.address, policy.clone(), i, observer.clone()).unwrap());
            nodes.push(node);
            configs.push(config);
        }
        Self {
            dir,
            nodes,
            workers,
            configs,
            observers,
            policy,
            head,
            alice,
            bob,
            senders,
        }
    }
    fn signed(&self, w: &SigningKey, recipient: &SigningKey, nonce: u64, amount: u64) -> Signed {
        Signed::sign(
            Transfer {
                sender: w.verifying_key().to_bytes(),
                recipient: recipient.verifying_key().to_bytes(),
                amount,
                nonce,
                chain: 23,
            },
            w,
        )
    }
    fn payment(&self, nonce: u64, amount: u64, selected: [bool; 4]) -> Payment {
        Payment::software_selected(
            &self.policy.committee,
            &self.head,
            self.signed(&self.alice, &self.bob, nonce, amount),
            selected,
        )
        .unwrap()
    }
    fn votes(&mut self, p: &Payment, selected: [bool; 4]) -> Vec<WitnessedApproval> {
        std::thread::scope(|scope| {
            let jobs: Vec<_> = self
                .workers
                .iter_mut()
                .enumerate()
                .filter(|(i, _)| selected[*i])
                .map(|(_, w)| {
                    scope.spawn(move || w.call(WorkerCommand::Prepare(Box::new(p.clone()))))
                })
                .collect();
            jobs.into_iter()
                .filter_map(|j| match j.join().unwrap() {
                    WorkerReply::Approval(v) => Some(*v),
                    WorkerReply::Rejected(_) => None,
                    _ => panic!("prepare reply"),
                })
                .collect()
        })
    }
    fn commit(&mut self, qc: &WitnessedCertificate, selected: [bool; 4]) {
        for (i, w) in self.workers.iter_mut().enumerate() {
            if selected[i] {
                assert!(matches!(
                    w.call(WorkerCommand::Commit(Box::new(qc.clone()))),
                    WorkerReply::Committed
                ));
            }
        }
        confirm_durable_commit(&self.policy, &self.observers, qc).unwrap();
        self.head = qc.certificate.statement.next.clone();
    }
    fn pay(&mut self, p: &Payment, selected: [bool; 4]) -> WitnessedCertificate {
        let votes = self.votes(p, selected);
        let qc = WitnessedCertificate::collect(&self.policy, &self.head, votes).unwrap();
        self.commit(&qc, selected);
        qc
    }
    fn balance(&mut self, who: &SigningKey, selected: [bool; 4]) -> Account {
        let reply = StaticSecret::random_from_rng(OsRng);
        let mut query = Query {
            account: who.verifying_key().to_bytes(),
            reply_key: PublicKey::from(&reply).to_bytes(),
            head: self.head.clone(),
            challenge: random(),
            signature: vec![],
        };
        query.sign(who);
        let mut responses = Vec::new();
        for (i, w) in self.workers.iter_mut().enumerate() {
            if selected[i] {
                let e = encrypt(
                    self.policy.committee.members[i].transport,
                    &bytes(&query),
                    QUERY_DOMAIN,
                )
                .unwrap();
                match w.call(WorkerCommand::Query(e)) {
                    WorkerReply::Balance(e) => responses.push(e),
                    WorkerReply::Rejected(e) => panic!("query refused: {e}"),
                    _ => panic!("query"),
                }
            }
        }
        verify_balance(&self.policy.committee, &query, &reply, &responses).unwrap()
    }
}
#[test]
fn encrypted_transfer_and_offline_receiving_survive_one_unavailable_executor() {
    let mut lab = Lab::new();
    let pids: std::collections::BTreeSet<_> = lab
        .workers
        .iter()
        .map(|w| w.child.id())
        .chain(lab.nodes.iter().map(|n| n.child.id()))
        .collect();
    assert_eq!(pids.len(), 8);
    println!("M23 SOFTWARE: four executor processes and four authenticated TCP/SQLite witness services; loopback is NOT independent hardware trust");
    lab.workers[3].stop();
    let selected = [true, true, true, false];
    let payment = lab.payment(0, 100, selected);
    assert!(payment.envelopes[3].is_none());
    assert!(payment.envelopes[..3].iter().all(Option::is_some));
    let qc = lab.pay(&payment, selected);
    assert_eq!(qc.certificate.approvals.len(), 3);
    let bob = lab.bob.clone();
    assert_eq!(lab.balance(&bob, selected).balance, 100);
    assert_eq!(recover_head(&lab.policy, &lab.observers).unwrap(), lab.head);
    for i in 0..3 {
        lab.workers[i].restart();
    }
    assert_eq!(lab.balance(&bob, selected).balance, 100);
    lab.workers[3].restart();
    let bundle = match lab.workers[0].call(WorkerCommand::Export(3)) {
        WorkerReply::Recovery(b) => b,
        _ => panic!("peer export"),
    };
    assert!(matches!(
        lab.workers[3].call(WorkerCommand::Recover(bundle)),
        WorkerReply::Committed
    ));
    let payment = lab.payment(1, 20, [true; 4]);
    lab.pay(&payment, [true; 4]);
    assert_eq!(lab.balance(&bob, [true; 4]).balance, 120);
}
#[test]
fn concurrent_credits_and_coordinator_rollback_recover_from_external_authorities() {
    let lab = Lab::new();
    let senders = lab.senders.clone();
    let recipient = lab.bob.clone();
    let shared = Arc::new(Mutex::new(lab));
    let jobs: Vec<_> = senders
        .into_iter()
        .map(|s| {
            let l = shared.clone();
            let b = recipient.clone();
            std::thread::spawn(move || {
                let mut lab = l.lock().unwrap();
                let tx = lab.signed(&s, &b, 0, 10);
                let p = Payment::software_selected(
                    &lab.policy.committee,
                    &lab.head,
                    tx,
                    [true, true, true, false],
                )
                .unwrap();
                lab.pay(&p, [true, true, true, false]);
            })
        })
        .collect();
    for j in jobs {
        j.join().unwrap();
    }
    let mut lab = Arc::try_unwrap(shared).ok().unwrap().into_inner().unwrap();
    assert_eq!(lab.head.epoch, 8);
    assert_eq!(
        lab.balance(&recipient, [true, true, true, false]).balance,
        80
    );
    lab.workers[3].restart();
    let bundle = match lab.workers[0].call(WorkerCommand::Export(3)) {
        WorkerReply::Recovery(b) => b,
        _ => panic!("history export"),
    };
    assert_eq!(bundle.history.len(), 8);
    let mut incomplete = bundle.clone();
    incomplete.history.remove(0);
    assert!(matches!(
        lab.workers[3].call(WorkerCommand::Recover(incomplete)),
        WorkerReply::Rejected(_)
    ));
    let mut altered = bundle.clone();
    altered.encrypted.ciphertext[0] ^= 1;
    assert!(matches!(
        lab.workers[3].call(WorkerCommand::Recover(altered)),
        WorkerReply::Rejected(_)
    ));
    assert!(matches!(
        lab.workers[3].call(WorkerCommand::Recover(bundle)),
        WorkerReply::Committed
    ));
    assert_eq!(lab.balance(&recipient, [true; 4]).balance, 80);

    // Coordinator's disk/memory is not a trusted anchor. A fresh read quorum recovers ACKed finality.
    lab.head = lab.policy.genesis.clone();
    lab.head = recover_head(&lab.policy, &lab.observers).unwrap();
    assert_eq!(lab.head.epoch, 8);
    for i in 0..3 {
        lab.nodes[i].restart();
        lab.workers[i].restart();
    }
    assert_eq!(recover_head(&lab.policy, &lab.observers).unwrap(), lab.head);
}
#[test]
fn restoring_both_local_snapshot_and_anchor_cannot_erase_the_remote_vote_lock() {
    let mut lab = Lab::new();
    let c = lab.configs[0].clone();
    let old = fs::read(&c.snapshot).unwrap();
    let anchor = PathBuf::from(format!("{}.cache-anchor", c.snapshot.display()));
    let old_anchor = fs::read(&anchor).unwrap();
    let a = lab.payment(0, 10, [true; 4]);
    let b = lab.payment(0, 20, [true; 4]);
    let first = match lab.workers[0].call(WorkerCommand::Prepare(Box::new(a.clone()))) {
        WorkerReply::Approval(v) => v,
        _ => panic!("first vote"),
    };
    lab.workers[0].stop();
    fs::write(&c.snapshot, &old).unwrap();
    fs::write(&anchor, &old_anchor).unwrap();
    lab.workers[0].restart();
    assert!(matches!(
        lab.workers[0].call(WorkerCommand::Prepare(Box::new(b))),
        WorkerReply::Rejected(_)
    ));
    let resumed = match lab.workers[0].call(WorkerCommand::Resume) {
        WorkerReply::Approval(v) => v,
        _ => panic!("resume"),
    };
    assert_eq!(first.approval.statement, resumed.approval.statement);
    assert_eq!(first.confirmation.signature, resumed.confirmation.signature);
    let votes = lab.votes(&a, [true; 4]);
    let qc = WitnessedCertificate::collect(&lab.policy, &lab.head, votes).unwrap();
    lab.commit(&qc, [true; 4]);
    lab.workers[0].stop();
    fs::write(&c.snapshot, old).unwrap();
    fs::write(&anchor, old_anchor).unwrap();
    lab.workers[0].restart();
    assert!(matches!(
        lab.workers[0].call(WorkerCommand::Prepare(Box::new(a))),
        WorkerReply::Rejected(_)
    ));
    let bob = lab.bob.clone();
    assert_eq!(lab.balance(&bob, [true; 4]).balance, 10);
}
#[test]
fn cloned_executors_cannot_obtain_conflicting_durable_confirmations() {
    let mut lab = Lab::new();
    let mut config = lab.configs[0].clone();
    config.snapshot = lab.dir.path().join("cloned-executor.json");
    let clone_path = lab.dir.path().join("clone-private.json");
    private(&clone_path, &bytes(&config));
    let mut clone = Worker::start(clone_path);
    let a = lab.payment(0, 10, [true; 4]);
    let b = lab.payment(0, 20, [true; 4]);
    let original = &mut lab.workers[0];
    let replies = std::thread::scope(|scope| {
        let x = scope.spawn(|| original.call(WorkerCommand::Prepare(Box::new(a))));
        let y = scope.spawn(|| clone.call(WorkerCommand::Prepare(Box::new(b))));
        [x.join().unwrap(), y.join().unwrap()]
    });
    assert_eq!(
        replies
            .iter()
            .filter(|r| matches!(r, WorkerReply::Approval(_)))
            .count(),
        1
    );
    assert_eq!(
        replies
            .iter()
            .filter(|r| matches!(r, WorkerReply::Rejected(_)))
            .count(),
        1
    );
    clone.stop();
    let record = lab.observers[0].get().unwrap();
    assert!(record.pending.is_some());
    assert_eq!(record.head, lab.policy.genesis);
}
#[test]
fn witness_requests_are_authenticated_replay_protected_and_compare_and_swap() {
    let mut lab = Lab::new();
    let client = lab.configs[0].client().unwrap();
    let request = client.request(0, Operation::Get).unwrap();
    let response = client.exchange(&request).unwrap();
    response.verify(&lab.policy, 0, &request).unwrap();
    assert!(client
        .exchange(&request)
        .unwrap()
        .verify(&lab.policy, 0, &request)
        .is_err());
    lab.nodes[0].restart();
    assert!(client
        .exchange(&request)
        .unwrap()
        .verify(&lab.policy, 0, &request)
        .is_err());
    {
        let mut malformed = std::net::TcpStream::connect(lab.nodes[0].address).unwrap();
        malformed.write_all(&u32::MAX.to_be_bytes()).unwrap();
    }
    let fresh = client.request(0, Operation::Get).unwrap();
    assert!(response.verify(&lab.policy, 0, &fresh).is_err());
    let mut tampered = response.clone();
    tampered.signature[0] ^= 1;
    assert!(tampered.verify(&lab.policy, 0, &request).is_err());
    let unauthorized = Request::signed(&lab.policy, 0, &key(), 0, Operation::Get).unwrap();
    assert!(client
        .exchange(&unauthorized)
        .unwrap()
        .verify(&lab.policy, 0, &unauthorized)
        .is_err());
    let r = client.get().unwrap();
    assert!(client
        .call(
            0,
            Operation::Initialize {
                snapshot: r.snapshot.clone()
            }
        )
        .is_err());
    let statement = Statement {
        committee: lab.policy.committee.id(),
        previous: r.head.clone(),
        next: Head {
            chain: 23,
            epoch: 1,
            commitment: random(),
        },
        transaction: random(),
    };
    let op = Operation::Prepare {
        statement: statement.clone(),
        snapshot: r.snapshot.clone(),
    };
    assert!(client.call(r.revision - 1, op.clone()).is_err());
    // Observer is independently authenticated but read-only.
    assert!(lab.observers[0].call(r.revision, op.clone()).is_err());
    let locked = client.call(r.revision, op).unwrap();
    let mut conflicting = statement;
    conflicting.transaction = random();
    assert!(client
        .call(
            locked.revision,
            Operation::Prepare {
                statement: conflicting,
                snapshot: locked.snapshot.clone()
            }
        )
        .is_err());
}
#[test]
fn witness_unavailability_is_fail_closed_without_any_local_cache_fallback() {
    let mut lab = Lab::new();
    let p = lab.payment(0, 10, [true; 4]);
    lab.nodes[0].stop();
    assert!(matches!(
        lab.workers[0].call(WorkerCommand::Prepare(Box::new(p.clone()))),
        WorkerReply::Rejected(_)
    ));
    assert!(matches!(
        lab.workers[0].call(WorkerCommand::Resume),
        WorkerReply::Rejected(_)
    ));
    assert!(matches!(
        lab.workers[0].call(WorkerCommand::Head),
        WorkerReply::Rejected(_)
    ));
    let selected = [false, true, true, true];
    let p = lab.payment(0, 10, selected);
    lab.pay(&p, selected);
    let bob = lab.bob.clone();
    assert_eq!(lab.balance(&bob, selected).balance, 10);
    lab.nodes[0].restart();
    lab.workers[0].restart();
    let bundle = match lab.workers[1].call(WorkerCommand::Export(0)) {
        WorkerReply::Recovery(b) => b,
        _ => panic!("export"),
    };
    assert!(matches!(
        lab.workers[0].call(WorkerCommand::Recover(bundle)),
        WorkerReply::Committed
    ));
}
#[test]
fn witness_crash_after_sql_commit_before_reply_releases_no_vote_and_recovers() {
    let mut lab = Lab::new();
    lab.nodes[0].config.test_crash_after_prepare_commit = true;
    lab.nodes[0].restart();
    let p = lab.payment(0, 10, [true; 4]);
    assert!(matches!(
        lab.workers[0].call(WorkerCommand::Prepare(Box::new(p.clone()))),
        WorkerReply::Rejected(_)
    ));
    assert_eq!(lab.nodes[0].child.wait().unwrap().code(), Some(86));
    lab.nodes[0].config.test_crash_after_prepare_commit = false;
    lab.nodes[0].restart();
    lab.workers[0].restart();
    let resumed = match lab.workers[0].call(WorkerCommand::Resume) {
        WorkerReply::Approval(v) => *v,
        _ => panic!("durable resume"),
    };
    let mut votes = lab.votes(&p, [false, true, true, true]);
    votes.push(resumed);
    let qc = WitnessedCertificate::collect(&lab.policy, &lab.head, votes).unwrap();
    lab.commit(&qc, [true; 4]);
    let bob = lab.bob.clone();
    assert_eq!(lab.balance(&bob, [true; 4]).balance, 10);
}
#[test]
fn crash_boundaries_before_after_witness_and_between_certification_and_commit() {
    for stage in ["local_prepared", "witness_confirmed"] {
        let mut lab = Lab::new();
        let p = lab.payment(0, 10, [true; 4]);
        assert!(matches!(
            lab.workers[0].call(WorkerCommand::PrepareFault {
                payment: Box::new(p.clone()),
                stage: stage.into()
            }),
            WorkerReply::Rejected(_)
        ));
        lab.workers[0].restart();
        let votes = lab.votes(&p, [true; 4]);
        let qc = WitnessedCertificate::collect(&lab.policy, &lab.head, votes).unwrap();
        // Coordinator loses its collected signatures: executors resume the exact witnessed locks.
        for w in &mut lab.workers {
            w.restart();
        }
        let votes: Vec<_> = lab
            .workers
            .iter_mut()
            .map(|w| match w.call(WorkerCommand::Resume) {
                WorkerReply::Approval(v) => *v,
                _ => panic!("resume"),
            })
            .collect();
        let resumed = WitnessedCertificate::collect(&lab.policy, &lab.head, votes).unwrap();
        assert_eq!(resumed.certificate.statement, qc.certificate.statement);
        lab.commit(&resumed, [true; 4]);
    }
    for stage in ["local_committed", "commit_confirmed"] {
        let mut lab = Lab::new();
        let p = lab.payment(0, 10, [true; 4]);
        let votes = lab.votes(&p, [true; 4]);
        let qc = WitnessedCertificate::collect(&lab.policy, &lab.head, votes).unwrap();
        assert!(matches!(
            lab.workers[0].call(WorkerCommand::CommitFault {
                certificate: Box::new(qc.clone()),
                stage: stage.into()
            }),
            WorkerReply::Rejected(_)
        ));
        lab.workers[0].restart();
        lab.commit(&qc, [true; 4]);
        assert_eq!(recover_head(&lab.policy, &lab.observers).unwrap(), lab.head);
    }
}

#[test]
fn unsigned_or_altered_witness_confirmations_cannot_downgrade_the_certificate() {
    let mut lab = Lab::new();
    let p = lab.payment(0, 10, [true; 4]);
    let votes = lab.votes(&p, [true; 4]);
    let qc = WitnessedCertificate::collect(&lab.policy, &lab.head, votes).unwrap();
    let mut missing = qc.clone();
    missing.confirmations.clear();
    assert!(missing.verify(&lab.policy, &lab.head).is_err());
    let mut duplicate = qc.clone();
    duplicate.confirmations = vec![qc.confirmations[0].clone(); qc.confirmations.len()];
    assert!(duplicate.verify(&lab.policy, &lab.head).is_err());
    let mut altered = qc.clone();
    altered.confirmations[0].statement.next.commitment = random();
    assert!(altered.verify(&lab.policy, &lab.head).is_err());
    let mut signed_by_wrong = qc.clone();
    signed_by_wrong.confirmations[0].signature[0] ^= 1;
    assert!(signed_by_wrong.verify(&lab.policy, &lab.head).is_err());
    for w in &mut lab.workers {
        assert!(matches!(
            w.call(WorkerCommand::Commit(Box::new(missing.clone()))),
            WorkerReply::Rejected(_)
        ));
    }
    lab.commit(&qc, [true; 4]);
    let mut replay = qc.clone();
    replay.certificate.statement.previous = lab.head.clone();
    assert!(replay.verify(&lab.policy, &lab.head).is_err());
}
#[test]
fn compromised_witness_storage_and_reused_authority_key_can_enable_equivocation() {
    // OUTSIDE the safety model: the authority administrator erases its independent durable history.
    let mut lab = Lab::new();
    let initial = lab.observers[0].get().unwrap();
    let a = lab.payment(0, 10, [true; 4]);
    let b = lab.payment(0, 20, [true; 4]);
    let first = match lab.workers[0].call(WorkerCommand::Prepare(Box::new(a))) {
        WorkerReply::Approval(v) => *v,
        _ => panic!("first approval"),
    };
    lab.workers[0].stop();
    lab.nodes[0].stop();
    for suffix in ["", "-wal", "-shm"] {
        let path = PathBuf::from(format!(
            "{}{suffix}",
            lab.nodes[0].config.database.display()
        ));
        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }
    lab.nodes[0].restart();
    let client = lab.configs[0].client().unwrap();
    client
        .call(
            0,
            Operation::Initialize {
                snapshot: initial.snapshot,
            },
        )
        .unwrap();
    lab.workers[0].restart();
    let second = match lab.workers[0].call(WorkerCommand::Prepare(Box::new(b))) {
        WorkerReply::Approval(v) => *v,
        _ => panic!("authority-reset approval"),
    };
    assert_ne!(first.approval.statement, second.approval.statement);
    first.verify(&lab.policy, &lab.policy.genesis).unwrap();
    second.verify(&lab.policy, &lab.policy.genesis).unwrap();
    assert_eq!(lab.head, lab.policy.genesis);
}

#[test]
fn host_cannot_swap_ciphertext_between_preparation_and_remote_confirmation() {
    let mut lab = Lab::new();
    let config = lab.configs[0].clone();
    let old_snapshot = fs::read(&config.snapshot).unwrap();
    let anchor = PathBuf::from(format!("{}.cache-anchor", config.snapshot.display()));
    let old_anchor = fs::read(&anchor).unwrap();
    let p = lab.payment(0, 10, [true; 4]);
    lab.workers[0].stop();
    let mut replica =
        WitnessedReplica::load(&config.snapshot, config.client().unwrap(), config.keys()).unwrap();
    assert!(replica
        .prepare_with_hook(&p, |stage| {
            if stage == "local_prepared" {
                fs::write(&config.snapshot, &old_snapshot)?;
                fs::write(&anchor, &old_anchor)?;
            }
            Ok(())
        })
        .is_err());
    let record = lab.observers[0].get().unwrap();
    assert!(record.pending.is_none());
    assert_eq!(record.head, lab.policy.genesis);
    assert!(replica.resume_vote().is_err());
    drop(replica);
    lab.workers[0].restart();
    lab.pay(&p, [true; 4]);
    let bob = lab.bob.clone();
    assert_eq!(lab.balance(&bob, [true; 4]).balance, 10);
}
