//! M23: authenticated remote sign-once authorities. No local directory is an independence claim.
use crate::{quorum::*, *};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::{
    io::Read,
    net::{SocketAddr, TcpListener, TcpStream},
    time::Duration,
};
const MAX_FRAME: usize = 32 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WitnessPolicy {
    pub committee: Committee,
    pub genesis: Head,
    pub authorities: [Id; 4],
    pub observer: Id,
}
impl WitnessPolicy {
    pub fn id(&self) -> Id {
        hash(&bytes(&("YAMI-M23-witness-policy-v1", self)))
    }
    pub fn validate(&self) -> Result<()> {
        self.committee.validate()?;
        check(self.genesis.chain == self.committee.chain && self.genesis.epoch == 0)?;
        let mut seen = std::collections::BTreeSet::new();
        for key in self.authorities {
            VerifyingKey::from_bytes(&key)?;
            check(
                key != [0; 32]
                    && seen.insert(key)
                    && !self.committee.members.iter().any(|m| m.signing == key),
            )?;
        }
        VerifyingKey::from_bytes(&self.observer)?;
        check(!seen.contains(&self.observer))?;
        check(
            !self
                .committee
                .members
                .iter()
                .any(|m| m.signing == self.observer),
        )
    }
    fn member(&self, index: usize) -> Result<Id> {
        Ok(self
            .committee
            .members
            .get(index)
            .ok_or("member index")?
            .signing)
    }
    fn index(&self, member: Id) -> Result<usize> {
        self.committee
            .members
            .iter()
            .position(|m| m.signing == member)
            .ok_or("foreign member".into())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Confirmation {
    pub policy: Id,
    pub member: Id,
    pub revision: u64,
    pub statement: Statement,
    pub snapshot_digest: Id,
    pub signature: Vec<u8>,
}
impl Confirmation {
    fn message(&self) -> Vec<u8> {
        bytes(&(
            "YAMI-M23-durable-vote-v1",
            self.policy,
            self.member,
            self.revision,
            &self.statement,
            self.snapshot_digest,
        ))
    }
    pub fn verify(&self, p: &WitnessPolicy) -> Result<()> {
        p.validate()?;
        check(self.policy == p.id() && self.revision > 0)?;
        self.statement
            .validate(&p.committee, &self.statement.previous)?;
        verify(
            p.authorities[p.index(self.member)?],
            &self.message(),
            &self.signature,
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WitnessedApproval {
    pub approval: Approval,
    pub confirmation: Confirmation,
}
impl WitnessedApproval {
    pub fn verify(&self, p: &WitnessPolicy, head: &Head) -> Result<()> {
        self.approval.verify(&p.committee, head)?;
        self.confirmation.verify(p)?;
        check(
            self.confirmation.member == self.approval.member
                && self.confirmation.statement == self.approval.statement,
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WitnessedCertificate {
    pub certificate: Certificate,
    pub confirmations: Vec<Confirmation>,
}
impl WitnessedCertificate {
    pub fn collect(p: &WitnessPolicy, head: &Head, votes: Vec<WitnessedApproval>) -> Result<Self> {
        let valid: Vec<_> = votes
            .into_iter()
            .filter(|a| a.verify(p, head).is_ok())
            .collect();
        let certificate = Certificate::collect(
            &p.committee,
            head,
            valid.iter().map(|a| a.approval.clone()).collect(),
        )?;
        let confirmations = certificate
            .approvals
            .iter()
            .map(|a| {
                valid
                    .iter()
                    .find(|v| v.approval.member == a.member && v.approval.statement == a.statement)
                    .expect("selected validated vote")
                    .confirmation
                    .clone()
            })
            .collect();
        let result = Self {
            certificate,
            confirmations,
        };
        result.verify(p, head)?;
        Ok(result)
    }
    pub fn verify(&self, p: &WitnessPolicy, head: &Head) -> Result<()> {
        p.validate()?;
        self.certificate.verify(&p.committee, head)?;
        check(self.confirmations.len() == self.certificate.approvals.len())?;
        let mut seen = std::collections::BTreeSet::new();
        for receipt in &self.confirmations {
            receipt.verify(p)?;
            check(
                seen.insert(receipt.member)
                    && receipt.statement == self.certificate.statement
                    && self
                        .certificate
                        .approvals
                        .iter()
                        .any(|a| a.member == receipt.member),
            )?;
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Record {
    pub policy: Id,
    pub member: Id,
    pub revision: u64,
    pub head: Head,
    pub pending: Option<Confirmation>,
    pub certificate: Option<WitnessedCertificate>,
    pub snapshot: Vec<u8>,
    #[serde(default)]
    pub history: Vec<WitnessedCertificate>,
}
impl Record {
    pub fn validate(&self, p: &WitnessPolicy, index: usize) -> Result<()> {
        check(
            self.policy == p.id()
                && self.member == p.member(index)?
                && self.revision > 0
                && !self.snapshot.is_empty()
                && self.snapshot.len() <= MAX_FRAME / 8,
        )?;
        if let Some(qc) = &self.certificate {
            qc.verify(p, &qc.certificate.statement.previous)?;
            check(qc.certificate.statement.next == self.head)?;
        } else {
            check(self.head == p.genesis)?;
        }
        if let Some(vote) = &self.pending {
            vote.verify(p)?;
            check(
                vote.member == self.member
                    && vote.statement.previous == self.head
                    && vote.snapshot_digest == hash(&self.snapshot)
                    && vote.revision <= self.revision,
            )?;
        }
        check(self.history.len() <= 256)?;
        let mut previous = None;
        for qc in &self.history {
            qc.verify(p, &qc.certificate.statement.previous)?;
            if let Some(head) = &previous {
                check(qc.certificate.statement.previous == *head)?;
            }
            previous = Some(qc.certificate.statement.next.clone());
        }
        if let Some(head) = previous {
            check(head == self.head)?;
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub enum Operation {
    Get,
    History {
        after: u64,
    },
    CatchUp {
        certificates: Vec<WitnessedCertificate>,
        snapshot: Vec<u8>,
    },
    Initialize {
        snapshot: Vec<u8>,
    },
    Prepare {
        statement: Statement,
        snapshot: Vec<u8>,
    },
    Commit {
        certificate: WitnessedCertificate,
        snapshot: Vec<u8>,
    },
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Request {
    pub policy: Id,
    pub member: Id,
    pub actor: Id,
    pub nonce: Id,
    pub expected_revision: u64,
    pub operation: Operation,
    pub signature: Vec<u8>,
}
impl Request {
    fn message(&self) -> Vec<u8> {
        bytes(&(
            "YAMI-M23-witness-request-v1",
            self.policy,
            self.member,
            self.actor,
            self.nonce,
            self.expected_revision,
            hash(&bytes(&self.operation)),
        ))
    }
    pub fn signed(
        p: &WitnessPolicy,
        index: usize,
        signer: &SigningKey,
        expected_revision: u64,
        operation: Operation,
    ) -> Result<Self> {
        let mut nonce = [0; 32];
        OsRng.fill_bytes(&mut nonce);
        let mut r = Self {
            policy: p.id(),
            member: p.member(index)?,
            actor: signer.verifying_key().to_bytes(),
            nonce,
            expected_revision,
            operation,
            signature: vec![],
        };
        r.signature = signer.sign(&r.message()).to_bytes().to_vec();
        Ok(r)
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub enum Outcome {
    Record(Box<Record>),
    Rejected(String),
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Response {
    pub request_hash: Id,
    pub outcome: Outcome,
    pub signature: Vec<u8>,
}
impl Response {
    fn message(&self) -> Vec<u8> {
        bytes(&(
            "YAMI-M23-witness-response-v1",
            self.request_hash,
            hash(&bytes(&self.outcome)),
        ))
    }
    pub fn verify(&self, p: &WitnessPolicy, index: usize, request: &Request) -> Result<Record> {
        p.validate()?;
        check(index < 4)?;
        check(self.request_hash == hash(&bytes(request)))?;
        verify(p.authorities[index], &self.message(), &self.signature)?;
        match &self.outcome {
            Outcome::Record(r) => {
                r.validate(p, index)?;
                Ok((**r).clone())
            }
            Outcome::Rejected(reason) => Err(format!("witness refused: {reason}").into()),
        }
    }
}
fn read_frame(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut length = [0; 4];
    stream.read_exact(&mut length)?;
    let n = u32::from_be_bytes(length) as usize;
    check(n > 0 && n <= MAX_FRAME)?;
    let mut data = vec![0; n];
    stream.read_exact(&mut data)?;
    Ok(data)
}
fn write_frame(stream: &mut TcpStream, data: &[u8]) -> Result<()> {
    check(!data.is_empty() && data.len() <= MAX_FRAME)?;
    stream.write_all(&(data.len() as u32).to_be_bytes())?;
    stream.write_all(data)?;
    stream.flush()?;
    Ok(())
}
#[derive(Clone)]
pub struct Client {
    pub address: SocketAddr,
    pub policy: WitnessPolicy,
    pub index: usize,
    actor: SigningKey,
}
impl Client {
    pub fn new(
        address: SocketAddr,
        policy: WitnessPolicy,
        index: usize,
        actor: SigningKey,
    ) -> Result<Self> {
        policy.validate()?;
        policy.member(index)?;
        Ok(Self {
            address,
            policy,
            index,
            actor,
        })
    }
    pub fn request(&self, revision: u64, operation: Operation) -> Result<Request> {
        Request::signed(&self.policy, self.index, &self.actor, revision, operation)
    }
    pub fn exchange(&self, request: &Request) -> Result<Response> {
        let mut stream = TcpStream::connect_timeout(&self.address, Duration::from_secs(2))?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        write_frame(&mut stream, &bytes(request))?;
        let response: Response = serde_json::from_slice(&read_frame(&mut stream)?)?;
        Ok(response)
    }
    pub fn call(&self, revision: u64, operation: Operation) -> Result<Record> {
        let request = self.request(revision, operation)?;
        self.exchange(&request)?
            .verify(&self.policy, self.index, &request)
    }
    pub fn get(&self) -> Result<Record> {
        self.call(0, Operation::Get)
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub listen: SocketAddr,
    pub database: PathBuf,
    pub policy: WitnessPolicy,
    pub index: usize,
    pub signing_secret: Id,
    #[serde(default)]
    pub test_crash_after_prepare_commit: bool,
}
pub struct Service {
    config: ServiceConfig,
    signer: SigningKey,
    connection: Connection,
    _lock: File,
}
impl Service {
    pub fn open(config: ServiceConfig) -> Result<Self> {
        config.policy.validate()?;
        let signer = SigningKey::from_bytes(&config.signing_secret);
        check(
            config.index < 4
                && signer.verifying_key().to_bytes() == config.policy.authorities[config.index],
        )?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(config.database.with_extension("lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock)?;
        let connection = Connection::open(&config.database)?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS metadata(id INTEGER PRIMARY KEY, policy BLOB NOT NULL, member BLOB NOT NULL); CREATE TABLE IF NOT EXISTS state(id INTEGER PRIMARY KEY, payload BLOB NOT NULL); CREATE TABLE IF NOT EXISTS nonces(nonce BLOB PRIMARY KEY); CREATE TABLE IF NOT EXISTS votes(sequence INTEGER PRIMARY KEY, statement BLOB NOT NULL, confirmation BLOB NOT NULL); CREATE TABLE IF NOT EXISTS certificates(sequence INTEGER PRIMARY KEY, certificate BLOB NOT NULL);")?;
        let member = config.policy.member(config.index)?;
        let policy = config.policy.id();
        connection.execute(
            "INSERT OR IGNORE INTO metadata VALUES(1,?1,?2)",
            params![policy.as_slice(), member.as_slice()],
        )?;
        let stored: (Vec<u8>, Vec<u8>) =
            connection.query_row("SELECT policy, member FROM metadata WHERE id=1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        check(stored == (policy.to_vec(), member.to_vec()))?;
        File::open(config.database.parent().ok_or("database parent")?)?.sync_all()?;
        Ok(Self {
            config,
            signer,
            connection,
            _lock: lock,
        })
    }
    fn apply(&mut self, request: &Request) -> Result<Record> {
        let policy = &self.config.policy;
        let index = self.config.index;
        let member = policy.member(index)?;
        check(
            request.policy == policy.id() && request.member == member && request.nonce != [0; 32],
        )?;
        verify(request.actor, &request.message(), &request.signature)?;
        let read_only = matches!(
            request.operation,
            Operation::Get | Operation::History { .. }
        );
        check(request.actor == member || (read_only && request.actor == policy.observer))?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let used: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM nonces WHERE nonce=?1)",
            params![request.nonce.as_slice()],
            |r| r.get(0),
        )?;
        check(!used)?;
        let old: Option<Vec<u8>> = tx
            .query_row("SELECT payload FROM state WHERE id=1", [], |r| r.get(0))
            .optional()?;
        let mut current: Option<Record> = old.map(|b| serde_json::from_slice(&b)).transpose()?;
        if let Some(r) = &current {
            r.validate(policy, index)?;
        }
        let record = match &request.operation {
            Operation::Get => current.ok_or("witness not initialized")?,
            Operation::History { after } => {
                let mut record = current.ok_or("witness not initialized")?;
                let mut statement=tx.prepare("SELECT certificate FROM certificates WHERE sequence>?1 ORDER BY sequence LIMIT 257")?;
                let rows = statement
                    .query_map(params![i64::try_from(*after)?], |r| r.get::<_, Vec<u8>>(0))?;
                for row in rows {
                    record.history.push(serde_json::from_slice(&row?)?);
                }
                record
            }
            Operation::CatchUp {
                certificates,
                snapshot,
            } => {
                let mut r = current.take().ok_or("witness not initialized")?;
                check(
                    request.expected_revision == r.revision
                        && !certificates.is_empty()
                        && certificates.len() <= 256,
                )?;
                for qc in certificates {
                    qc.verify(policy, &r.head)?;
                    r.head = qc.certificate.statement.next.clone();
                    tx.execute(
                        "INSERT INTO certificates VALUES(?1,?2)",
                        params![i64::try_from(r.head.epoch)?, bytes(qc)],
                    )?;
                }
                r.revision = r.revision.checked_add(1).ok_or("revision exhausted")?;
                r.pending = None;
                r.certificate = certificates.last().cloned();
                r.snapshot = snapshot.clone();
                r
            }
            Operation::Initialize { snapshot } => {
                check(current.is_none() && request.expected_revision == 0)?;
                Record {
                    policy: policy.id(),
                    member,
                    revision: 1,
                    head: policy.genesis.clone(),
                    pending: None,
                    certificate: None,
                    snapshot: snapshot.clone(),
                    history: vec![],
                }
            }
            Operation::Prepare {
                statement,
                snapshot,
            } => {
                let mut r = current.take().ok_or("witness not initialized")?;
                check(request.expected_revision == r.revision)?;
                statement.validate(&policy.committee, &r.head)?;
                if let Some(locked) = &r.pending {
                    check(
                        locked.statement == *statement && locked.snapshot_digest == hash(snapshot),
                    )?;
                    r
                } else {
                    let existing: Option<Vec<u8>> = tx
                        .query_row(
                            "SELECT statement FROM votes WHERE sequence=?1",
                            params![i64::try_from(statement.next.epoch)?],
                            |r| r.get(0),
                        )
                        .optional()?;
                    check(existing.is_none())?;
                    r.revision = r
                        .revision
                        .checked_add(1)
                        .ok_or("witness revision exhausted")?;
                    let mut proof = Confirmation {
                        policy: policy.id(),
                        member,
                        revision: r.revision,
                        statement: statement.clone(),
                        snapshot_digest: hash(snapshot),
                        signature: vec![],
                    };
                    proof.signature = self.signer.sign(&proof.message()).to_bytes().to_vec();
                    tx.execute(
                        "INSERT INTO votes VALUES(?1,?2,?3)",
                        params![
                            i64::try_from(statement.next.epoch)?,
                            bytes(statement),
                            bytes(&proof)
                        ],
                    )?;
                    r.pending = Some(proof);
                    r.snapshot = snapshot.clone();
                    r
                }
            }
            Operation::Commit {
                certificate,
                snapshot,
            } => {
                let mut r = current.take().ok_or("witness not initialized")?;
                check(request.expected_revision == r.revision)?;
                if r.certificate.as_ref().is_some_and(|old| {
                    old.certificate.statement == certificate.certificate.statement
                }) {
                    certificate.verify(policy, &certificate.certificate.statement.previous)?;
                    check(hash(snapshot) == hash(&r.snapshot))?;
                    r
                } else {
                    certificate.verify(policy, &r.head)?;
                    r.head = certificate.certificate.statement.next.clone();
                    tx.execute(
                        "INSERT INTO certificates VALUES(?1,?2)",
                        params![i64::try_from(r.head.epoch)?, bytes(certificate)],
                    )?;
                    r.revision = r.revision.checked_add(1).ok_or("revision exhausted")?;
                    r.pending = None;
                    r.certificate = Some(certificate.clone());
                    r.snapshot = snapshot.clone();
                    r
                }
            }
        };
        record.validate(policy, index)?;
        tx.execute(
            "INSERT INTO nonces VALUES(?1)",
            params![request.nonce.as_slice()],
        )?;
        if !read_only {
            tx.execute(
                "INSERT OR REPLACE INTO state VALUES(1,?1)",
                params![bytes(&record)],
            )?;
        }
        // SQLite WAL synchronous=FULL: durable state, nonce and sign-once record precede ANY reply.
        tx.commit()?;
        Ok(record)
    }
    pub fn handle(&mut self, request: &Request) -> Response {
        let result = self.apply(request);
        let outcome = match result {
            Ok(r) => Outcome::Record(Box::new(r)),
            Err(e) => Outcome::Rejected(e.to_string()),
        };
        let mut response = Response {
            request_hash: hash(&bytes(request)),
            outcome,
            signature: vec![],
        };
        response.signature = self.signer.sign(&response.message()).to_bytes().to_vec();
        response
    }
    pub fn serve(mut self) -> Result<()> {
        let socket = socket2::Socket::new(
            socket2::Domain::for_address(self.config.listen),
            socket2::Type::STREAM,
            Some(socket2::Protocol::TCP),
        )?;
        socket.set_reuse_address(true)?;
        socket.bind(&self.config.listen.into())?;
        socket.listen(128)?;
        let listener: TcpListener = socket.into();
        println!("{}", listener.local_addr()?);
        std::io::stdout().flush()?;
        for incoming in listener.incoming() {
            let mut stream = incoming?;
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            stream.set_write_timeout(Some(Duration::from_secs(2)))?;
            let attempt = (|| -> Result<()> {
                let request: Request = serde_json::from_slice(&read_frame(&mut stream)?)?;
                let response = self.handle(&request);
                if self.config.test_crash_after_prepare_commit
                    && matches!(request.operation, Operation::Prepare { .. })
                    && matches!(response.outcome, Outcome::Record(_))
                {
                    std::process::exit(86);
                }
                write_frame(&mut stream, &bytes(&response))
            })();
            // A malformed connection must not terminate the independently deployable authority.
            if attempt.is_err() {
                continue;
            }
        }
        Ok(())
    }
}

/// M23 execution profile. Legacy local anchors are DISPOSABLE cache checks only;
/// authority and the complete encrypted recovery snapshot live at the authenticated service.
pub struct WitnessedReplica {
    inner: Replica,
    client: Client,
    record: Record,
    poisoned: bool,
}
impl WitnessedReplica {
    pub fn create(path: &Path, client: Client, keys: ReplicaKeys, state: &State) -> Result<Self> {
        check(client.policy.member(client.index)? == keys.signing.verifying_key().to_bytes())?;
        let anchor = Replica::cache_anchor(path);
        let inner = Replica::create(
            path,
            &anchor,
            client.policy.committee.clone(),
            client.index,
            keys,
            state,
            &client.policy.genesis,
        )?;
        let snapshot = inner.checkpoint()?;
        let record = client.call(0, Operation::Initialize { snapshot })?;
        let result = Self {
            inner,
            client,
            record,
            poisoned: false,
        };
        result.consistent()?;
        Ok(result)
    }
    pub fn load(path: &Path, client: Client, keys: ReplicaKeys) -> Result<Self> {
        let record = client.get()?;
        let inner = Replica::restore_cache(
            path,
            client.policy.committee.clone(),
            client.index,
            keys,
            &record.snapshot,
        )?;
        let result = Self {
            inner,
            client,
            record,
            poisoned: false,
        };
        result.consistent()?;
        Ok(result)
    }
    fn consistent(&self) -> Result<()> {
        let (head, pending, cert) = self.inner.metadata()?;
        check(
            head == self.record.head
                && pending == self.record.pending.as_ref().map(|p| p.statement.clone())
                && cert.as_ref().map(|c| &c.statement)
                    == self
                        .record
                        .certificate
                        .as_ref()
                        .map(|c| &c.certificate.statement)
                && hash(&self.inner.checkpoint()?) == hash(&self.record.snapshot),
        )
    }
    fn live(&self) -> Result<()> {
        check(!self.poisoned)?;
        self.consistent()?;
        let current = self.client.get()?;
        check(
            current.revision == self.record.revision
                && hash(&current.snapshot) == hash(&self.record.snapshot),
        )
    }
    pub fn head(&self) -> Result<Head> {
        self.live()?;
        self.inner.head()
    }
    pub fn prepare(&mut self, p: &Payment) -> Result<WitnessedApproval> {
        self.prepare_with_hook(p, |_| Ok(()))
    }
    pub fn prepare_with_hook(
        &mut self,
        p: &Payment,
        mut hook: impl FnMut(&str) -> Result<()>,
    ) -> Result<WitnessedApproval> {
        self.live()?;
        self.poisoned = true;
        // The inner signature remains private to this wrapper; it is never released before witness ACK.
        let approval = match self.inner.prepare(p) {
            Ok(v) => v,
            Err(e) => {
                if self.consistent().is_ok() {
                    self.poisoned = false;
                }
                return Err(e);
            }
        };
        hook("local_prepared")?;
        let snapshot = self.inner.checkpoint()?;
        let record = self.client.call(
            self.record.revision,
            Operation::Prepare {
                statement: approval.statement.clone(),
                snapshot,
            },
        )?;
        hook("witness_confirmed")?;
        self.record = record;
        self.consistent()?;
        self.poisoned = false;
        let confirmation = self.record.pending.clone().ok_or("missing durable vote")?;
        let result = WitnessedApproval {
            approval,
            confirmation,
        };
        result.verify(&self.client.policy, &self.record.head)?;
        Ok(result)
    }
    pub fn resume_vote(&mut self) -> Result<WitnessedApproval> {
        self.live()?;
        let approval = self.inner.resume_pending()?;
        let confirmation = self.record.pending.clone().ok_or("no witnessed vote")?;
        let result = WitnessedApproval {
            approval,
            confirmation,
        };
        result.verify(&self.client.policy, &self.record.head)?;
        Ok(result)
    }
    pub fn commit(&mut self, qc: &WitnessedCertificate) -> Result<()> {
        self.commit_with_hook(qc, |_| Ok(()))
    }
    pub fn commit_with_hook(
        &mut self,
        qc: &WitnessedCertificate,
        mut hook: impl FnMut(&str) -> Result<()>,
    ) -> Result<()> {
        self.live()?;
        qc.verify(&self.client.policy, &qc.certificate.statement.previous)?;
        if self
            .record
            .certificate
            .as_ref()
            .is_some_and(|old| old.certificate.statement == qc.certificate.statement)
        {
            return Ok(());
        }
        qc.verify(&self.client.policy, &self.record.head)?;
        self.poisoned = true;
        self.inner.commit(&qc.certificate)?;
        hook("local_committed")?;
        let snapshot = self.inner.checkpoint()?;
        let record = self.client.call(
            self.record.revision,
            Operation::Commit {
                certificate: qc.clone(),
                snapshot,
            },
        )?;
        hook("commit_confirmed")?;
        self.record = record;
        self.consistent()?;
        self.poisoned = false;
        Ok(())
    }
    pub fn query(&self, q: &Envelope) -> Result<Envelope> {
        self.live()?;
        self.inner.query(q)
    }
    pub fn export_recovery(&self, index: usize) -> Result<RecoveryBundle> {
        self.export_recovery_since(index, 0)
    }
    /// Supply the recipient's authenticated current sequence; each catch-up is bounded to 256 QCs.
    pub fn export_recovery_since(&self, index: usize, after: u64) -> Result<RecoveryBundle> {
        self.live()?;
        Ok(RecoveryBundle {
            encrypted: self.inner.export_recovery(index)?,
            certificate: self
                .record
                .certificate
                .clone()
                .ok_or("no witnessed certificate")?,
            history: self.client.call(0, Operation::History { after })?.history,
        })
    }
    pub fn recover(&mut self, bundle: &RecoveryBundle) -> Result<()> {
        self.live()?;
        bundle.certificate.verify(
            &self.client.policy,
            &bundle.certificate.certificate.statement.previous,
        )?;
        if self.record.head == bundle.certificate.certificate.statement.next {
            return Ok(());
        }
        let chain: Vec<_> = bundle
            .history
            .iter()
            .filter(|qc| qc.certificate.statement.next.epoch > self.record.head.epoch)
            .cloned()
            .collect();
        check(!chain.is_empty() && chain.len() <= 256)?;
        let mut head = self.record.head.clone();
        for qc in &chain {
            qc.verify(&self.client.policy, &head)?;
            head = qc.certificate.statement.next.clone();
        }
        check(
            head == bundle.certificate.certificate.statement.next
                && chain.last().map(|c| &c.certificate.statement)
                    == Some(&bundle.certificate.certificate.statement),
        )?;
        self.poisoned = true;
        let plain: Vec<_> = chain.iter().map(|c| c.certificate.clone()).collect();
        if let Err(error) = self
            .inner
            .recover_certified_chain(&bundle.encrypted, &plain)
        {
            if self.consistent().is_ok() {
                self.poisoned = false;
            }
            return Err(error);
        }
        check(
            self.inner.metadata()?.2.as_ref().map(|c| &c.statement)
                == Some(&bundle.certificate.certificate.statement),
        )?;
        let snapshot = self.inner.checkpoint()?;
        self.record = self.client.call(
            self.record.revision,
            Operation::CatchUp {
                certificates: chain,
                snapshot,
            },
        )?;
        self.consistent()?;
        self.poisoned = false;
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct RecoveryBundle {
    pub encrypted: Envelope,
    pub certificate: WitnessedCertificate,
    pub history: Vec<WitnessedCertificate>,
}
/// Recover finality without trusting a coordinator disk. ACK requires >=3 witness commits;
/// any fresh read quorum intersects those durable records. Unacknowledged partial commits
/// can require additional recovery/availability; they never authorize a conflicting vote.
pub fn recover_head(policy: &WitnessPolicy, clients: &[Client]) -> Result<Head> {
    let mut seen = std::collections::BTreeSet::new();
    let mut head = policy.genesis.clone();
    for c in clients {
        check(c.policy == *policy)?;
        if let Ok(record) = c.get() {
            check(seen.insert(c.index))?;
            if let Some(qc) = record.certificate {
                qc.verify(policy, &qc.certificate.statement.previous)?;
                let candidate = qc.certificate.statement.next;
                if candidate.epoch == head.epoch {
                    check(candidate == head)?;
                } else if candidate.epoch > head.epoch {
                    head = candidate;
                }
            }
        }
    }
    check(seen.len() >= 3)?;
    Ok(head)
}
pub fn confirm_durable_commit(
    policy: &WitnessPolicy,
    clients: &[Client],
    qc: &WitnessedCertificate,
) -> Result<()> {
    qc.verify(policy, &qc.certificate.statement.previous)?;
    let mut seen = std::collections::BTreeSet::new();
    for c in clients {
        check(c.policy == *policy)?;
        if let Ok(record) = c.get() {
            if record.head == qc.certificate.statement.next {
                seen.insert(c.index);
            }
        }
    }
    check(seen.len() >= 3)
}

#[derive(Clone, Serialize, Deserialize)]
pub struct WorkerConfig {
    pub snapshot: PathBuf,
    pub policy: WitnessPolicy,
    pub index: usize,
    pub authority: SocketAddr,
    pub signing_secret: Id,
    pub transport_secret: Id,
    pub storage_secret: Id,
    pub root_secret: Id,
    pub software_protocol_only: bool,
    #[serde(default)]
    pub fault_injection: bool,
}
impl WorkerConfig {
    pub fn keys(&self) -> ReplicaKeys {
        ReplicaKeys {
            signing: SigningKey::from_bytes(&self.signing_secret),
            transport: StaticSecret::from(self.transport_secret),
            storage: self.storage_secret,
            root_secret: self.root_secret,
        }
    }
    pub fn client(&self) -> Result<Client> {
        Client::new(
            self.authority,
            self.policy.clone(),
            self.index,
            self.keys().signing,
        )
    }
}
#[derive(Serialize, Deserialize)]
pub enum WorkerCommand {
    Prepare(Box<Payment>),
    PrepareFault {
        payment: Box<Payment>,
        stage: String,
    },
    Commit(Box<WitnessedCertificate>),
    CommitFault {
        certificate: Box<WitnessedCertificate>,
        stage: String,
    },
    Resume,
    Head,
    Query(Envelope),
    Export(usize),
    ExportSince {
        index: usize,
        after: u64,
    },
    Recover(Box<RecoveryBundle>),
}
#[derive(Serialize, Deserialize)]
pub enum WorkerReply {
    Approval(Box<WitnessedApproval>),
    Committed,
    Head(Head),
    Balance(Envelope),
    Recovery(Box<RecoveryBundle>),
    Rejected(String),
}
