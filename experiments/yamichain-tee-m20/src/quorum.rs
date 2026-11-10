//! M21: fixed-membership, sign-once 3-of-4 SOFTWARE quorum. No hardware claims.
//! Trusted anchors must live outside the rollback domain of the encrypted files.
use super::*;
use crate::incremental::{self, Snapshot};
use std::collections::BTreeSet;

pub const PAYMENT_DOMAIN: &[u8] = b"YAMI-M21-payment-v1";
pub const QUERY_DOMAIN: &[u8] = b"YAMI-M21-query-delivery-v1";

pub use yamichain_devnet_cert::{Member,Committee,Statement,Approval,Certificate};

#[derive(Clone, Serialize, Deserialize)]
struct Delivery {
    transfer: Signed,
    blind: Id,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Payment {
    pub committee: Id,
    pub previous: Head,
    pub transaction: Id,
    pub envelopes: [Option<Envelope>; 4],
}
fn tx_commit(d: &Delivery) -> Id {
    hash(&bytes(&("YAMI-M21-blinded-transaction-v1", d)))
}
impl Payment {
    /// Wallet API for explicitly selected SOFTWARE development membership only.
    pub fn software(c: &Committee, previous: &Head, transfer: Signed) -> Result<Self> {
        Self::software_selected(c, previous, transfer, [true; 4])
    }
    /// Explicit SOFTWARE protocol entry; this does not create hardware evidence.
    pub fn software_selected(
        c: &Committee,
        previous: &Head,
        transfer: Signed,
        selected: [bool; 4],
    ) -> Result<Self> {
        c.validate()?;
        check(selected.iter().filter(|yes| **yes).count() >= 3)?;
        transfer.verify()?;
        check(previous.chain == c.chain && transfer.transfer.chain == c.chain)?;
        let mut blind = [0; 32];
        OsRng.fill_bytes(&mut blind);
        let d = Delivery { transfer, blind };
        let transaction = tx_commit(&d);
        let context = bytes(&(PAYMENT_DOMAIN, c.id(), previous, transaction));
        let mut envelopes = Vec::new();
        for (i, m) in c.members.iter().enumerate() {
            envelopes.push(if selected[i] {
                Some(encrypt(m.transport, &bytes(&d), &context)?)
            } else {
                None
            });
        }
        Ok(Self {
            committee: c.id(),
            previous: previous.clone(),
            transaction,
            envelopes: envelopes.try_into().map_err(|_| "envelope count")?,
        })
    }
    fn context(&self) -> Vec<u8> {
        bytes(&(
            PAYMENT_DOMAIN,
            self.committee,
            &self.previous,
            self.transaction,
        ))
    }
}

/// Secret-blinded common ledger root; all four replicas know this secret and ledger.
/// Canonical ordered state encoding is inherited from M20. No public balance hash.
pub fn state_head(state: &State, root_secret: Id) -> Head {
    // HKDF/HMAC-based PRF, with a separate output key for each canonical state.
    let mut commitment = [0; 32];
    Hkdf::<Sha256>::new(Some(b"YAMI-M21-private-state-root-v1"), &root_secret)
        .expand(&encode(state), &mut commitment)
        .expect("fixed-size HKDF output");
    Head {
        chain: state.chain,
        epoch: state.epoch,
        commitment,
    }
}

fn atomic(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = path.with_extension("pending");
    let mut f = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    fs::rename(tmp, path)?;
    File::open(path.parent().ok_or("parent")?)?.sync_all()?;
    Ok(())
}
/// A stand-in for an independently durable trusted compare-and-swap service.
/// Its directory is a TRUSTED boundary, unlike the host-controlled snapshot path.
struct AnchoredFile {
    path: PathBuf,
    anchor: PathBuf,
    digest: Id,
    poisoned: bool,
    _locks: Vec<File>,
}
impl AnchoredFile {
    fn locked(path: &Path, anchor: &Path) -> Result<Self> {
        check(path != anchor && path.with_extension("lock") != anchor.with_extension("lock"))?;
        let mut locks = Vec::new();
        for p in [path, anchor] {
            let f = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(p.with_extension("lock"))?;
            fs2::FileExt::try_lock_exclusive(&f)?;
            locks.push(f);
        }
        Ok(Self {
            path: path.into(),
            anchor: anchor.into(),
            digest: [0; 32],
            poisoned: false,
            _locks: locks,
        })
    }
    fn create(path: &Path, anchor: &Path, data: &[u8]) -> Result<Self> {
        let mut s = Self::locked(path, anchor)?;
        check(!path.exists() && !anchor.exists())?;
        s.persist(data, &mut |_| Ok(()))?;
        Ok(s)
    }
    fn load(path: &Path, anchor: &Path) -> Result<(Self, Vec<u8>)> {
        let mut s = Self::locked(path, anchor)?;
        let data = fs::read(path)?;
        let trusted: Id = serde_json::from_slice(&fs::read(anchor)?)?;
        check(hash(&data) == trusted)?;
        s.digest = trusted;
        Ok((s, data))
    }
    fn persist(&mut self, data: &[u8], hook: &mut impl FnMut(&str) -> Result<()>) -> Result<()> {
        check(!self.poisoned)?;
        if self.anchor.exists() {
            let current: Id = serde_json::from_slice(&fs::read(&self.anchor)?)?;
            check(current == self.digest && hash(&fs::read(&self.path)?) == self.digest)?;
        } else {
            check(self.digest == [0; 32])?;
        }
        self.poisoned = true;
        atomic(&self.path, data)?;
        hook("snapshot_durable")?;
        let digest = hash(data);
        atomic(&self.anchor, &bytes(&digest))?;
        hook("anchor_durable")?;
        self.digest = digest;
        self.poisoned = false;
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct LedgerWire(
    u64,
    u64,
    Vec<(Id, Account)>,
    #[serde(default, skip_serializing_if = "Option::is_none")] Option<crate::economics::Economics>,
);
impl LedgerWire {
    fn from_state(s: &State) -> Self {
        Self(
            s.chain,
            s.epoch,
            s.accounts.iter().map(|(k, v)| (*k, v.clone())).collect(),
            s.economics.clone(),
        )
    }
    fn state(&self) -> Result<State> {
        decode(&bytes(&self))
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Pending {
    statement: Statement,
    next: LedgerWire,
}
#[derive(Clone, Serialize, Deserialize)]
struct ReplicaDisk {
    committee: Id,
    member: Id,
    state: LedgerWire,
    pending: Option<Pending>,
    certificate: Option<Certificate>,
}
#[derive(Clone)]
pub struct ReplicaKeys {
    pub signing: SigningKey,
    pub transport: StaticSecret,
    pub storage: Id,
    pub root_secret: Id,
}
pub struct Replica {
    committee: Committee,
    index: usize,
    keys: ReplicaKeys,
    disk: ReplicaDisk,
    file: AnchoredFile,
}
impl Replica {
    fn check_keys(c: &Committee, index: usize, k: &ReplicaKeys) -> Result<()> {
        c.validate()?;
        let m = c.members.get(index).ok_or("member index")?;
        check(
            m.signing == k.signing.verifying_key().to_bytes()
                && m.transport == PublicKey::from(&k.transport).to_bytes(),
        )
    }
    fn unpack(raw: &[u8], storage: Id) -> Result<ReplicaDisk> {
        let plain = if Snapshot::is_full(raw) {
            Snapshot::parse(raw)?.plaintext(storage)?
        } else {
            let envelope: Envelope = serde_json::from_slice(raw)?;
            open(storage, &envelope, b"YAMI-M21-replica-disk-v1")?
        };
        Ok(serde_json::from_slice(&plain)?)
    }
    fn stored_snapshot(&self) -> Result<Vec<u8>> {
        let raw = fs::read(&self.file.path)?;
        if Snapshot::is_manifest(&raw) {
            let connection = incremental::connect(&self.file.path)?;
            Snapshot::load(&connection, &raw)?.pack()
        } else {
            Ok(raw)
        }
    }
    fn persist_disk(
        &mut self,
        disk: &ReplicaDisk,
        hook: &mut impl FnMut(&str) -> Result<()>,
    ) -> Result<()> {
        check(!self.file.poisoned)?;
        let previous = if self.file.path.exists() {
            let raw = self.checkpoint()?;
            if Snapshot::is_full(&raw) {
                Some(Snapshot::parse(&raw)?)
            } else {
                None
            }
        } else {
            None
        };
        let snapshot = Snapshot::build(&bytes(disk), self.keys.storage, previous.as_ref())?;
        let mut connection = incremental::connect(&self.file.path)?;
        self.file.poisoned = true;
        {
            let tx = connection.transaction()?;
            snapshot.store(&tx, None)?;
            tx.commit()?;
        }
        // New chunks are durable before the manifest/anchor is advanced. Old
        // chunks remain available until BOTH existing durability barriers pass.
        self.file.poisoned = false;
        self.file.persist(&snapshot.manifest(), hook)?;
        self.file.poisoned = true;
        {
            let tx = connection.transaction()?;
            snapshot.retain(&tx)?;
            tx.commit()?;
        }
        self.file.poisoned = false;
        Ok(())
    }
    pub fn create(
        path: &Path,
        anchor: &Path,
        c: Committee,
        index: usize,
        keys: ReplicaKeys,
        genesis: &State,
        expected: &Head,
    ) -> Result<Self> {
        Self::check_keys(&c, index, &keys)?;
        check(
            genesis.epoch == 0
                && genesis.chain == c.chain
                && state_head(genesis, keys.root_secret) == *expected,
        )?;
        let disk = ReplicaDisk {
            committee: c.id(),
            member: c.members[index].signing,
            state: LedgerWire::from_state(genesis),
            pending: None,
            certificate: None,
        };
        let file = AnchoredFile::locked(path, anchor)?;
        check(!path.exists() && !anchor.exists() && !incremental::sidecar(path).exists())?;
        let mut result = Self {
            committee: c,
            index,
            keys,
            disk: disk.clone(),
            file,
        };
        result.persist_disk(&disk, &mut |_| Ok(()))?;
        Ok(result)
    }
    pub fn load(
        path: &Path,
        anchor: &Path,
        c: Committee,
        index: usize,
        keys: ReplicaKeys,
    ) -> Result<Self> {
        Self::check_keys(&c, index, &keys)?;
        let (file, raw) = AnchoredFile::load(path, anchor)?;
        let raw = if Snapshot::is_manifest(&raw) {
            Snapshot::load(&incremental::connect(path)?, &raw)?.pack()?
        } else {
            raw
        };
        let disk = Self::unpack(&raw, keys.storage)?;
        check(disk.committee == c.id() && disk.member == c.members[index].signing)?;
        let state = disk.state.state()?;
        check(state.chain == c.chain)?;
        let head = state_head(&state, keys.root_secret);
        if let Some(qc) = &disk.certificate {
            qc.verify(&c, &qc.statement.previous)?;
            check(qc.statement.next == head)?;
        } else {
            check(head.epoch == 0)?;
        }
        if let Some(p) = &disk.pending {
            p.statement.validate(&c, &head)?;
            check(state_head(&p.next.state()?, keys.root_secret) == p.statement.next)?;
        }
        Ok(Self {
            committee: c,
            index,
            keys,
            disk,
            file,
        })
    }
    /// M23 disposable cache interface: the external authenticated witness supplies authority.
    pub(crate) fn checkpoint(&self) -> Result<Vec<u8>> {
        let raw = self.stored_snapshot()?;
        let stored = Self::unpack(&raw, self.keys.storage)?;
        // A hostile host may swap ciphertext after local preparation. Never confirm a
        // recovery snapshot that differs from the actual private state being signed.
        check(bytes(&stored) == bytes(&self.disk))?;
        Ok(raw)
    }
    pub(crate) fn metadata(&self) -> Result<(Head, Option<Statement>, Option<Certificate>)> {
        Ok((
            self.head()?,
            self.disk.pending.as_ref().map(|p| p.statement.clone()),
            self.disk.certificate.clone(),
        ))
    }
    pub(crate) fn cache_anchor(path: &Path) -> PathBuf {
        let mut name = path.file_name().expect("snapshot file name").to_os_string();
        name.push(".cache-anchor");
        path.with_file_name(name)
    }
    pub(crate) fn restore_cache(
        path: &Path,
        c: Committee,
        index: usize,
        keys: ReplicaKeys,
        blob: &[u8],
    ) -> Result<Self> {
        let anchor = Self::cache_anchor(path);
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock)?;
        // Restore only an authority-authenticated complete snapshot. Legacy
        // keystores stay readable; the next successful mutation migrates them.
        Self::check_keys(&c, index, &keys)?;
        Self::unpack(blob, keys.storage)?;
        let manifest = if Snapshot::is_full(blob) {
            let snapshot = Snapshot::parse(blob)?;
            let mut connection = incremental::connect(path)?;
            let tx = connection.transaction()?;
            snapshot.store(&tx, None)?;
            tx.commit()?;
            snapshot.manifest()
        } else {
            blob.to_vec()
        };
        atomic(path, &manifest)?;
        atomic(&anchor, &bytes(&hash(&manifest)))?;
        if Snapshot::is_full(blob) {
            let snapshot = Snapshot::parse(blob)?;
            let mut connection = incremental::connect(path)?;
            let tx = connection.transaction()?;
            snapshot.retain(&tx)?;
            tx.commit()?;
        }
        drop(lock);
        Self::load(path, &anchor, c, index, keys)
    }
    pub(crate) fn resume_pending(&self) -> Result<Approval> {
        check(!self.file.poisoned)?;
        Ok(self.approval(
            self.disk
                .pending
                .as_ref()
                .ok_or("no prepared vote")?
                .statement
                .clone(),
        ))
    }
    pub fn head(&self) -> Result<Head> {
        Ok(state_head(&self.disk.state.state()?, self.keys.root_secret))
    }
    pub(crate) fn native_scope(&self) -> Result<Option<Id>> {
        Ok(self
            .disk
            .state
            .state()?
            .economics
            .and_then(|e| e.live.map(|l| l.scope)))
    }
    fn approval(&self, s: Statement) -> Approval {
        let signature = self.keys.signing.sign(&s.message()).to_bytes().to_vec();
        Approval {
            member: self.committee.members[self.index].signing,
            statement: s,
            signature,
        }
    }
    pub fn prepare(&mut self, p: &Payment) -> Result<Approval> {
        self.prepare_with_hook(p, |_| Ok(()))
    }
    /// Deterministic error injection for durability tests, never bypasses validation.
    pub fn prepare_with_hook(
        &mut self,
        p: &Payment,
        mut hook: impl FnMut(&str) -> Result<()>,
    ) -> Result<Approval> {
        check(
            !self.file.poisoned
                && p.committee == self.committee.id()
                && p.previous == self.head()?,
        )?;
        // Every replica independently decrypts and validates before voting, even on retry.
        let d: Delivery = serde_json::from_slice(&decrypt(
            &self.keys.transport,
            p.envelopes[self.index]
                .as_ref()
                .ok_or("member was not selected for delivery")?,
            &p.context(),
        )?)?;
        check(tx_commit(&d) == p.transaction)?;
        let next = transition(&self.disk.state.state()?, &d.transfer)?;
        let statement = Statement {
            next_validators: next
                .economics
                .as_ref()
                .and_then(|e| e.live.as_ref().map(|l| l.active.clone())),
            committee: p.committee,
            previous: p.previous.clone(),
            next: state_head(&next, self.keys.root_secret),
            transaction: p.transaction,
        };
        if let Some(pending) = &self.disk.pending {
            check(pending.statement == statement)?;
            return Ok(self.approval(statement));
        }
        let mut disk = self.disk.clone();
        disk.pending = Some(Pending {
            statement: statement.clone(),
            next: LedgerWire::from_state(&next),
        });
        self.persist_disk(&disk, &mut hook)?;
        self.disk = disk;
        Ok(self.approval(statement))
    }
    pub fn commit(&mut self, qc: &Certificate) -> Result<()> {
        self.commit_with_hook(qc, |_| Ok(()))
    }
    pub fn commit_with_hook(
        &mut self,
        qc: &Certificate,
        mut hook: impl FnMut(&str) -> Result<()>,
    ) -> Result<()> {
        check(!self.file.poisoned)?;
        // Idempotent retransmission after a coordinator/executor crash.
        if self
            .disk
            .certificate
            .as_ref()
            .is_some_and(|old| old.statement == qc.statement)
        {
            qc.verify(&self.committee, &qc.statement.previous)?;
            return Ok(());
        }
        qc.verify(&self.committee, &self.head()?)?;
        let p = self
            .disk
            .pending
            .as_ref()
            .ok_or("prepare required for catch-up")?;
        check(p.statement == qc.statement)?;
        let mut disk = self.disk.clone();
        disk.state = p.next.clone();
        disk.pending = None;
        disk.certificate = Some(qc.clone());
        self.persist_disk(&disk, &mut hook)?;
        self.disk = disk;
        Ok(())
    }
    /// Encrypt a certified full-state recovery snapshot only to a pinned committee member.
    pub fn export_recovery(&self, recipient: usize) -> Result<Envelope> {
        check(!self.file.poisoned)?;
        let member = self
            .committee
            .members
            .get(recipient)
            .ok_or("recipient index")?;
        let certificate = self
            .disk
            .certificate
            .clone()
            .ok_or("no finalized certificate")?;
        let data = Recovery {
            committee: self.committee.id(),
            state: self.disk.state.clone(),
            certificate,
        };
        encrypt(member.transport, &bytes(&data), b"YAMI-M21-recovery-v1")
    }
    /// A QC may resolve a competing pending lock without signing another vote.
    /// Only a one-sequence successor or exact already-committed state is accepted.
    pub fn recover(&mut self, envelope: &Envelope) -> Result<()> {
        check(!self.file.poisoned)?;
        let data: Recovery = serde_json::from_slice(&decrypt(
            &self.keys.transport,
            envelope,
            b"YAMI-M21-recovery-v1",
        )?)?;
        check(data.committee == self.committee.id())?;
        data.certificate
            .verify(&self.committee, &data.certificate.statement.previous)?;
        let state = data.state.state()?;
        let next = state_head(&state, self.keys.root_secret);
        check(next == data.certificate.statement.next)?;
        if next == self.head()? {
            return Ok(());
        }
        data.certificate.verify(&self.committee, &self.head()?)?;
        let mut disk = self.disk.clone();
        disk.state = data.state;
        disk.pending = None;
        disk.certificate = Some(data.certificate);
        self.persist_disk(&disk, &mut |_| Ok(()))?;
        self.disk = disk;
        Ok(())
    }
    /// M23 bounded certified state-chain catch-up. Caller additionally verifies witness receipts.
    pub(crate) fn recover_certified_chain(
        &mut self,
        envelope: &Envelope,
        chain: &[Certificate],
    ) -> Result<()> {
        check(!self.file.poisoned && !chain.is_empty() && chain.len() <= 256)?;
        let mut head = self.head()?;
        for certificate in chain {
            certificate.verify(&self.committee, &head)?;
            head = certificate.statement.next.clone();
        }
        self.recover_witnessed_head(envelope, chain.last().ok_or("empty chain")?)
    }
    /// Only the witnessed wrapper may call this after its own authenticated
    /// authority has durably verified every missing successor in bounded pages.
    /// A final certificate alone is never a public snapshot-import capability.
    pub(crate) fn recover_witnessed_head(
        &mut self,
        envelope: &Envelope,
        expected: &Certificate,
    ) -> Result<()> {
        check(!self.file.poisoned)?;
        expected.verify(&self.committee, &expected.statement.previous)?;
        let data: Recovery = serde_json::from_slice(&decrypt(
            &self.keys.transport,
            envelope,
            b"YAMI-M21-recovery-v1",
        )?)?;
        check(data.committee == self.committee.id())?;
        data.certificate
            .verify(&self.committee, &expected.statement.previous)?;
        check(
            expected.statement == data.certificate.statement
                && state_head(&data.state.state()?, self.keys.root_secret)
                    == expected.statement.next,
        )?;
        let mut disk = self.disk.clone();
        disk.state = data.state;
        disk.pending = None;
        disk.certificate = Some(data.certificate);
        self.persist_disk(&disk, &mut |_| Ok(()))?;
        self.disk = disk;
        Ok(())
    }
    /// Queries are encrypted in transit in BOTH directions. Wallet must collect 3 matching replies.
    pub fn query(&self, envelope: &Envelope) -> Result<Envelope> {
        check(!self.file.poisoned)?;
        let q: Query =
            serde_json::from_slice(&decrypt(&self.keys.transport, envelope, QUERY_DOMAIN)?)?;
        check(q.head == self.head()?)?;
        verify(q.account, &q.message(), &q.signature)?;
        let state = self.disk.state.state()?;
        let mut response = BalanceReply {
            committee: self.committee.id(),
            member: self.committee.members[self.index].signing,
            account: state.accounts.get(&q.account).cloned().unwrap_or_default(),
            portfolio: state
                .economics
                .as_ref()
                .map(|e| e.portfolio(q.account, state.epoch)),
            query_hash: hash(&q.message()),
            signature: vec![],
        };
        response.signature = self
            .keys
            .signing
            .sign(&response.message())
            .to_bytes()
            .to_vec();
        encrypt(q.reply_key, &bytes(&response), &q.message())
    }
}

#[derive(Serialize, Deserialize)]
struct Recovery {
    committee: Id,
    state: LedgerWire,
    certificate: Certificate,
}

#[derive(Clone, Serialize, Deserialize)]
struct Accepted {
    committee: Id,
    head: Head,
    certificate: Option<Certificate>,
}
/// Software consensus boundary harness, entirely separate from production consensus.
pub struct Acceptor {
    committee: Committee,
    disk: Accepted,
    file: AnchoredFile,
}
impl Acceptor {
    pub fn create(path: &Path, anchor: &Path, c: Committee, genesis: Head) -> Result<Self> {
        c.validate()?;
        check(genesis.chain == c.chain && genesis.epoch == 0)?;
        let disk = Accepted {
            committee: c.id(),
            head: genesis,
            certificate: None,
        };
        let file = AnchoredFile::create(path, anchor, &bytes(&disk))?;
        Ok(Self {
            committee: c,
            disk,
            file,
        })
    }
    pub fn load(path: &Path, anchor: &Path, c: Committee) -> Result<Self> {
        c.validate()?;
        let (file, raw) = AnchoredFile::load(path, anchor)?;
        let disk: Accepted = serde_json::from_slice(&raw)?;
        check(disk.committee == c.id() && disk.head.chain == c.chain)?;
        if let Some(qc) = &disk.certificate {
            qc.verify(&c, &qc.statement.previous)?;
            check(qc.statement.next == disk.head)?;
        } else {
            check(disk.head.epoch == 0)?;
        }
        Ok(Self {
            committee: c,
            disk,
            file,
        })
    }
    pub fn head(&self) -> Head {
        self.disk.head.clone()
    }
    pub fn certificate(&self) -> Option<Certificate> {
        self.disk.certificate.clone()
    }
    pub fn accept(&mut self, qc: &Certificate) -> Result<()> {
        check(!self.file.poisoned)?;
        qc.verify(&self.committee, &self.disk.head)?;
        let disk = Accepted {
            committee: self.committee.id(),
            head: qc.statement.next.clone(),
            certificate: Some(qc.clone()),
        };
        self.file.persist(&bytes(&disk), &mut |_| Ok(()))?;
        self.disk = disk;
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct BalanceReply {
    pub committee: Id,
    pub member: Id,
    pub account: Account,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portfolio: Option<crate::economics::Portfolio>,
    pub query_hash: Id,
    pub signature: Vec<u8>,
}
impl BalanceReply {
    pub fn message(&self) -> Vec<u8> {
        if let Some(portfolio) = &self.portfolio {
            return bytes(&(
                "YAMI-private-portfolio-reply-v1",
                self.committee,
                self.member,
                &self.account,
                portfolio,
                self.query_hash,
            ));
        }
        bytes(&(
            "YAMI-M21-balance-reply-v1",
            self.committee,
            self.member,
            &self.account,
            self.query_hash,
        ))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerifiedAccount {
    #[serde(flatten)]
    pub account: Account,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub portfolio: Option<crate::economics::Portfolio>,
}
pub fn verify_balance(
    c: &Committee,
    q: &Query,
    secret: &StaticSecret,
    replies: &[Envelope],
) -> Result<Account> {
    Ok(verify_portfolio(c, q, secret, replies)?.account)
}
pub fn verify_portfolio(
    c: &Committee,
    q: &Query,
    secret: &StaticSecret,
    replies: &[Envelope],
) -> Result<VerifiedAccount> {
    c.validate()?;
    check(
        q.head.chain == c.chain
            && q.reply_key == PublicKey::from(secret).to_bytes()
            && replies.len() >= 3
            && replies.len() <= 4,
    )?;
    verify(q.account, &q.message(), &q.signature)?;
    let mut seen = BTreeSet::new();
    let mut groups: Vec<(VerifiedAccount, usize)> = Vec::new();
    for envelope in replies {
        // Invalid/offline/malicious replies count for nothing; never poison honest matching replies.
        let parsed = (|| -> Result<BalanceReply> {
            let r: BalanceReply =
                serde_json::from_slice(&decrypt(secret, envelope, &q.message())?)?;
            check(
                r.committee == c.id()
                    && r.query_hash == hash(&q.message())
                    && c.members.iter().any(|m| m.signing == r.member),
            )?;
            verify(r.member, &r.message(), &r.signature)?;
            Ok(r)
        })();
        if let Ok(r) = parsed {
            if seen.insert(r.member) {
                let verified = VerifiedAccount {
                    account: r.account,
                    portfolio: r.portfolio,
                };
                if let Some(g) = groups.iter_mut().find(|g| g.0 == verified) {
                    g.1 += 1;
                } else {
                    groups.push((verified, 1));
                }
            }
        }
    }
    groups
        .into_iter()
        .find(|g| g.1 >= 3)
        .map(|g| g.0)
        .ok_or("no balance quorum".into())
}

/// Production entry stays closed. No software attestation path can open it.
pub fn production_attestation_gate(quote: &[u8]) -> Result<()> {
    super::verify_tdx_quote(quote)
}

/// Expected Intel TD REPORTDATA: hash binds the complete fixed roster and this member's
/// signing/transport keys to the deployment, software identity and wallet challenge.
/// This is a binding calculation, NOT quote verification or attestation acceptance.
pub fn tdx_report_data(
    c: &Committee,
    index: usize,
    genesis: &Head,
    software_identity: Id,
    challenge: Id,
) -> Result<[u8; 64]> {
    c.validate()?;
    check(genesis.chain == c.chain && genesis.epoch == 0)?;
    let member = c.members.get(index).ok_or("member index")?;
    let digest = hash(&bytes(&(
        "YAMI-M21-TDX-REPORTDATA-v1",
        c.id(),
        index,
        member,
        genesis,
        software_identity,
        challenge,
    )));
    let mut report = [0; 64];
    report[..32].copy_from_slice(&digest);
    Ok(report)
}

#[cfg(test)]
mod incremental_migration_tests {
    use super::*;
    #[test]
    fn legacy_encrypted_state_migrates_atomically_without_changing_financial_roots() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replica.json");
        let anchor = dir.path().join("anchor.json");
        let signing: Vec<_> = (10..14).map(|n| SigningKey::from_bytes(&[n; 32])).collect();
        let transport: Vec<_> = (20..24).map(|n| StaticSecret::from([n; 32])).collect();
        let committee = Committee {
            chain: 24,
            members: std::array::from_fn(|i| Member {
                signing: signing[i].verifying_key().to_bytes(),
                transport: PublicKey::from(&transport[i]).to_bytes(),
            }),
        };
        let alice = SigningKey::from_bytes(&[1; 32]);
        let bob = SigningKey::from_bytes(&[2; 32]);
        let state = State {
            chain: 24,
            epoch: 0,
            economics: None,
            accounts: BTreeMap::from([(
                alice.verifying_key().to_bytes(),
                Account {
                    balance: 1000,
                    nonce: 0,
                },
            )]),
        };
        let keys = ReplicaKeys {
            signing: signing[0].clone(),
            transport: transport[0].clone(),
            storage: [3; 32],
            root_secret: [4; 32],
        };
        let disk = ReplicaDisk {
            committee: committee.id(),
            member: committee.members[0].signing,
            state: LedgerWire::from_state(&state),
            pending: None,
            certificate: None,
        };
        let legacy =
            bytes(&seal(keys.storage, &bytes(&disk), b"YAMI-M21-replica-disk-v1").unwrap());
        fs::write(&path, &legacy).unwrap();
        fs::write(&anchor, bytes(&hash(&legacy))).unwrap();
        use crate::witness::{
            Operation, Outcome, Record, Request, Service, ServiceConfig, WitnessPolicy,
        };
        let authorities: Vec<_> = (30..34).map(|n| SigningKey::from_bytes(&[n; 32])).collect();
        let policy = WitnessPolicy {
            committee: committee.clone(),
            genesis: state_head(&state, keys.root_secret),
            authorities: std::array::from_fn(|i| authorities[i].verifying_key().to_bytes()),
            observer: SigningKey::from_bytes(&[40; 32]).verifying_key().to_bytes(),
            native_scope: None,
        };
        let database = dir.path().join("legacy-authority.sqlite");
        let mut authority = Service::open(ServiceConfig {
            listen: "127.0.0.1:0".parse().unwrap(),
            database: database.clone(),
            policy: policy.clone(),
            index: 0,
            signing_secret: authorities[0].to_bytes(),
            tls: None,
            test_crash_after_prepare_commit: false,
        })
        .unwrap();
        let initialize = Request::signed(
            &policy,
            0,
            &signing[0],
            0,
            Operation::Initialize {
                snapshot: legacy.clone(),
            },
        )
        .unwrap();
        assert!(matches!(
            authority.handle(&initialize).outcome,
            Outcome::Record(_)
        ));
        let mut replica =
            Replica::load(&path, &anchor, committee.clone(), 0, keys.clone()).unwrap();
        let transfer = Signed::sign(
            Transfer {
                chain: 24,
                sender: alice.verifying_key().to_bytes(),
                recipient: bob.verifying_key().to_bytes(),
                amount: 10,
                nonce: 0,
                finance: None,
            },
            &alice,
        );
        let next = transition(&state, &transfer).unwrap();
        let payment = Payment::software(&committee, &replica.head().unwrap(), transfer).unwrap();
        let vote = replica.prepare(&payment).unwrap();
        let snapshot = replica.checkpoint().unwrap();
        let request = Request::signed(
            &policy,
            0,
            &signing[0],
            1,
            Operation::Prepare {
                statement: vote.statement.clone(),
                snapshot: snapshot.clone(),
            },
        )
        .unwrap();
        let record = match authority.handle(&request).outcome {
            Outcome::Record(record) => record,
            _ => panic!("legacy witness migration"),
        };
        assert_eq!(record.snapshot, snapshot);
        assert!(matches!(
            authority.handle(&request).outcome,
            Outcome::Rejected(_)
        ));
        let connection = rusqlite::Connection::open(&database).unwrap();
        let stored: Vec<u8> = connection
            .query_row("SELECT payload FROM state WHERE id=1", [], |r| r.get(0))
            .unwrap();
        assert!(serde_json::from_slice::<Record>(&stored)
            .unwrap()
            .snapshot
            .is_empty());
        assert!(Snapshot::is_manifest(&fs::read(&path).unwrap()));
        let votes = signing[..3]
            .iter()
            .map(|key| Approval {
                member: key.verifying_key().to_bytes(),
                statement: vote.statement.clone(),
                signature: key.sign(&vote.statement.message()).to_bytes().to_vec(),
            })
            .collect();
        let certificate = Certificate::assemble(&committee, &payment.previous, votes).unwrap();
        replica.commit(&certificate).unwrap();
        drop(replica);
        let replica = Replica::load(&path, &anchor, committee, 0, keys.clone()).unwrap();
        assert_eq!(replica.head().unwrap(), state_head(&next, keys.root_secret));
    }
}
