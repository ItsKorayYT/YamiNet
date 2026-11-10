//! Disposable-funds TDX guest adapter. No quote simulation and no production activation.
use crate::{attestation::*, quorum::*, witness::*, *};
use libloading::Library;
use std::{
    io::Read,
    net::SocketAddr,
    os::fd::AsRawFd,
    time::{SystemTime, UNIX_EPOCH},
};

const ENROLL: &str = "YAMI-TDX-enrollment-v1";
const PROVISION: &[u8] = b"YAMI-TDX-provision-v1";
const BACKUP: &[u8] = b"YAMI-TDX-recovery-v1";
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestConfig {
    pub index: usize,
    pub operator: Id,
    pub recovery: Id,
    pub software_identity: Id,
    pub quote_library: PathBuf,
    pub quote_sha256: Id,
    pub listen: SocketAddr,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub member: Member,
    pub index: usize,
    pub boot: Id,
    pub software_identity: Id,
    pub operator: Id,
    pub recovery: Id,
}
pub fn enrollment_data(id: &Identity, challenge: Id) -> [u8; 64] {
    let mut out = [0; 64];
    out[..32].copy_from_slice(&hash(&bytes(&(ENROLL, id, challenge))));
    out
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provision {
    #[serde(default)]
    pub economics: Option<crate::economics::Policy>,
    pub root: Id,
    pub allocations: Vec<(Id, u64)>,
    pub restored: Option<Seeds>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Seeds {
    pub signing: Id,
    pub transport: Id,
    pub storage: Id,
    pub root: Id,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub boot: Id,
    pub policy: WitnessPolicy,
    pub authority: SocketAddr,
    pub capsule: Envelope,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configure {
    pub configuration: Configuration,
    pub signature: Vec<u8>,
}
impl Configuration {
    pub fn message(&self) -> Vec<u8> {
        bytes(&("YAMI-TDX-configure-v1", self))
    }
}
// Exact ABI: Intel DCAP tdx_attest.h at the source commit recorded in deployment/tdx/inputs.json.
type GetQuote =
    unsafe extern "C" fn(*const u8, *const u8, u32, *mut u8, *mut *mut u8, *mut u32, u32) -> u32;
type FreeQuote = unsafe extern "C" fn(*mut u8) -> u32;
pub struct QuoteGenerator {
    _library: Library,
    _file: File,
    get: GetQuote,
    free: FreeQuote,
}
impl QuoteGenerator {
    pub fn load(config: &GuestConfig) -> Result<Self> {
        check(cfg!(all(target_os = "linux", target_arch = "x86_64")))?;
        check(
            Path::new("/dev/tdx_guest").exists()
                || Path::new("/dev/tdx-guest").exists()
                || Path::new("/sys/kernel/config/tsm/report").exists(),
        )?;
        check(config.quote_library.is_absolute())?;
        let mut file = File::open(&config.quote_library)?;
        check(file.metadata()?.len() <= 16 * 1024 * 1024)?;
        let mut raw = Vec::new();
        file.read_to_end(&mut raw)?;
        check(hash(&raw) == config.quote_sha256)?;
        // SAFETY: immutable measured guest supplies pinned official Intel native library and dependencies.
        // Keep the checked inode open; never load a path or function pointer supplied by a caller.
        let lib = unsafe { Library::new(format!("/proc/self/fd/{}", file.as_raw_fd()))? };
        unsafe {
            Ok(Self {
                get: *lib.get(b"tdx_att_get_quote\0")?,
                free: *lib.get(b"tdx_att_free_quote\0")?,
                _library: lib,
                _file: file,
            })
        }
    }
    pub fn quote(&self, data: &[u8; 64]) -> Result<Vec<u8>> {
        let mut ptr = std::ptr::null_mut();
        let mut size = 0;
        // SAFETY: reportdata points to 64 bytes; null key list/count selects Intel's default key;
        // optional selected key is null; output buffer is owned by Intel until free_quote below.
        let code = unsafe {
            (self.get)(
                data.as_ptr(),
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                &mut ptr,
                &mut size,
                0,
            )
        };
        let result = (|| {
            check(code == 0 && !ptr.is_null() && size > 0 && size <= 1024 * 1024)?;
            let quote = unsafe { std::slice::from_raw_parts(ptr, size as usize) }.to_vec();
            let parsed = TdxReport::parse(&quote)?;
            check(parsed.report_data == *data && parsed.td_attributes & 1 == 0)?;
            // Parsing checks generation/binding only. Wallet-local QVL must verify authentic signatures.
            Ok(quote)
        })();
        if !ptr.is_null() {
            unsafe {
                (self.free)(ptr);
            }
        }
        result
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupAck {
    pub boot: Id,
    pub digest: Id,
    pub signature: Vec<u8>,
}
impl BackupAck {
    pub fn message(&self) -> Vec<u8> {
        bytes(&("YAMI-TDX-backup-durable-v1", self.boot, self.digest))
    }
}
pub fn backup_ack(id: &Identity, backup: &Envelope, operator: &SigningKey) -> BackupAck {
    let mut ack = BackupAck {
        boot: id.boot,
        digest: hash(&bytes(backup)),
        signature: vec![],
    };
    ack.signature = operator.sign(&ack.message()).to_bytes().to_vec();
    ack
}
pub struct Guest {
    pub config: GuestConfig,
    identity: Identity,
    keys: Option<ReplicaKeys>,
    replica: Option<WitnessedReplica>,
    policy: Option<WitnessPolicy>,
    backup: Option<Envelope>,
    active: bool,
}
impl Guest {
    pub fn new(config: GuestConfig) -> Result<Self> {
        check(config.index < 4 && config.operator != [0; 32] && config.recovery != [0; 32])?;
        VerifyingKey::from_bytes(&config.operator)?;
        let signing = SigningKey::generate(&mut OsRng);
        let transport = StaticSecret::random_from_rng(OsRng);
        let mut storage = [0; 32];
        OsRng.fill_bytes(&mut storage);
        let mut boot = [0; 32];
        OsRng.fill_bytes(&mut boot);
        let identity = Identity {
            member: Member {
                signing: signing.verifying_key().to_bytes(),
                transport: PublicKey::from(&transport).to_bytes(),
            },
            index: config.index,
            boot,
            software_identity: config.software_identity,
            operator: config.operator,
            recovery: config.recovery,
        };
        Ok(Self {
            config,
            identity,
            keys: Some(ReplicaKeys {
                signing,
                transport,
                storage,
                root_secret: [0; 32],
            }),
            replica: None,
            policy: None,
            backup: None,
            active: false,
        })
    }
    pub fn identity(&self) -> &Identity {
        &self.identity
    }
    pub fn report_data(&self, challenge: Id, enroll: bool) -> Result<[u8; 64]> {
        check(challenge != [0; 32])?;
        if enroll {
            return Ok(enrollment_data(&self.identity, challenge));
        }
        let p = self.policy.as_ref().ok_or("guest not provisioned")?;
        let identity = hash(&bytes(&(
            "YAMI-M23-software-and-witness-policy-v1",
            self.config.software_identity,
            p.id(),
        )));
        tdx_report_data(
            &p.committee,
            self.config.index,
            &p.genesis,
            identity,
            challenge,
        )
    }
    pub fn configure(&mut self, request: Configure, path: &Path) -> Result<Envelope> {
        check(self.replica.is_none() && self.policy.is_none())?;
        let c = request.configuration;
        check(c.boot == self.identity.boot)?;
        verify(self.config.operator, &c.message(), &request.signature)?;
        c.policy.validate()?;
        let boot_keys = self.keys.as_ref().ok_or("already provisioned")?;
        let provision: Provision =
            serde_json::from_slice(&decrypt(&boot_keys.transport, &c.capsule, PROVISION)?)?;
        let keys = match provision.restored {
            Some(s) => {
                check(s.root == provision.root)?;
                ReplicaKeys {
                    signing: SigningKey::from_bytes(&s.signing),
                    transport: StaticSecret::from(s.transport),
                    storage: s.storage,
                    root_secret: s.root,
                }
            }
            None => ReplicaKeys {
                signing: boot_keys.signing.clone(),
                transport: boot_keys.transport.clone(),
                storage: boot_keys.storage,
                root_secret: provision.root,
            },
        };
        let expected = Member {
            signing: keys.signing.verifying_key().to_bytes(),
            transport: PublicKey::from(&keys.transport).to_bytes(),
        };
        check(c.policy.committee.members[self.config.index] == expected)?;
        let client = Client::new(
            c.authority,
            c.policy.clone(),
            self.config.index,
            keys.signing.clone(),
        )?;
        let backup = encrypt(
            self.config.recovery,
            &bytes(&Seeds {
                signing: keys.signing.to_bytes(),
                transport: keys.transport.to_bytes(),
                storage: keys.storage,
                root: keys.root_secret,
            }),
            BACKUP,
        )?;
        // Witness, never local disk presence, determines whether a recovery is required.
        // Restore is explicit; an old/missing local snapshot never reinitializes a witness.
        let replica = if client.get().is_ok() {
            WitnessedReplica::load(path, client, keys)?
        } else {
            check(!provision.allocations.is_empty())?;
            let mut accounts = BTreeMap::new();
            for (id, balance) in provision.allocations {
                check(accounts.insert(id, Account { balance, nonce: 0 }).is_none())?;
            }
            let mut state = State {
                economics: None,

                chain: c.policy.committee.chain,
                epoch: 0,
                accounts,
            };
            if let Some(policy) = provision.economics {
                state.enable_economics(policy)?;
            }
            check(state_head(&state, keys.root_secret) == c.policy.genesis)?;
            WitnessedReplica::create(path, client, keys, &state)?
        };
        self.keys = None;
        // Keep only the public identity once keys have moved into WitnessedReplica.
        self.identity.member = expected;
        self.replica = Some(replica);
        self.policy = Some(c.policy);
        self.backup = Some(backup.clone());
        Ok(backup)
    }
    pub fn backup(&self) -> Result<&Envelope> {
        self.backup.as_ref().ok_or("not provisioned".into())
    }
    pub fn activate(&mut self, ack: BackupAck) -> Result<()> {
        check(ack.boot == self.identity.boot && ack.digest == hash(&bytes(self.backup()?)))?;
        verify(self.config.operator, &ack.message(), &ack.signature)?;
        self.active = true;
        Ok(())
    }
    pub fn command(&mut self, cmd: WorkerCommand) -> Result<WorkerReply> {
        check(self.active)?;
        let r = self.replica.as_mut().ok_or("guest not provisioned")?;
        Ok(match cmd {
            WorkerCommand::Prepare(p) => WorkerReply::Approval(Box::new(r.prepare(&p)?)),
            WorkerCommand::Commit(q) => {
                r.commit(&q)?;
                WorkerReply::Committed
            }
            WorkerCommand::Head => WorkerReply::Head(r.head()?),
            WorkerCommand::Query(q) => WorkerReply::Balance(r.query(&q)?),
            WorkerCommand::Resume => WorkerReply::Approval(Box::new(r.resume_vote()?)),
            WorkerCommand::Export(i) => WorkerReply::Recovery(Box::new(r.export_recovery(i)?)),
            WorkerCommand::ExportSince { index, after } => {
                WorkerReply::Recovery(Box::new(r.export_recovery_since(index, after)?))
            }
            WorkerCommand::Recover(bundle) => {
                r.recover(&bundle)?;
                WorkerReply::Committed
            }
            _ => return Err("fault injection unavailable in TDX guest".into()),
        })
    }
}
// Custodian-only helper: never used to decrypt anything in a host assertion or quote path.
pub fn decrypt_seeds(secret: Id, capsule: &Envelope) -> Result<Seeds> {
    Ok(serde_json::from_slice(&decrypt(
        &StaticSecret::from(secret),
        capsule,
        BACKUP,
    )?)?)
}
pub fn provision(
    id: &Identity,
    policy: WitnessPolicy,
    authority: SocketAddr,
    body: &Provision,
    operator: &SigningKey,
) -> Result<Configure> {
    check(id.operator == operator.verifying_key().to_bytes())?;
    let configuration = Configuration {
        boot: id.boot,
        policy,
        authority,
        capsule: encrypt(id.member.transport, &bytes(body), PROVISION)?,
    };
    let signature = operator.sign(&configuration.message()).to_bytes().to_vec();
    Ok(Configure {
        configuration,
        signature,
    })
}
pub fn verify_enrollment(
    qvl: &IntelQvl,
    policy: &SoftwarePolicy,
    id: &Identity,
    challenge: Id,
    e: &Evidence,
) -> Result<()> {
    check(id.software_identity == policy.identity && challenge != [0; 32])?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    qvl.verify_at(&e.quote, &e.collateral, now)?
        .enforce(policy, &enrollment_data(id, challenge))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn guest() -> (Guest, SigningKey) {
        let operator = SigningKey::generate(&mut OsRng);
        let recovery = StaticSecret::random_from_rng(OsRng);
        let config = GuestConfig {
            index: 0,
            operator: operator.verifying_key().to_bytes(),
            recovery: PublicKey::from(&recovery).to_bytes(),
            software_identity: [7; 32],
            quote_library: "/absent".into(),
            quote_sha256: [1; 32],
            listen: "127.0.0.1:1".parse().unwrap(),
        };
        (Guest::new(config).unwrap(), operator)
    }
    #[test]
    fn tdx_guest_enrollment_binds_keys_boot_software_and_custodian() {
        // Binding calculation only; no fabricated quote or hardware acceptance.
        let (g, _) = guest();
        let id = g.identity();
        let c = [9; 32];
        let reference = enrollment_data(id, c);
        for n in 0..6 {
            let mut changed = id.clone();
            match n {
                0 => changed.member.signing[0] ^= 1,
                1 => changed.member.transport[0] ^= 1,
                2 => changed.boot[0] ^= 1,
                3 => changed.software_identity[0] ^= 1,
                4 => changed.operator[0] ^= 1,
                _ => changed.recovery[0] ^= 1,
            }
            assert_ne!(reference, enrollment_data(&changed, c));
        }
        assert_ne!(reference, enrollment_data(id, [8; 32]));
        assert!(g.report_data(c, false).is_err());
        let raw = serde_json::to_value(id).unwrap();
        for key in [
            "signing_secret",
            "transport_secret",
            "storage",
            "root_secret",
        ] {
            assert!(raw.get(key).is_none());
        }
    }
    #[test]
    fn tdx_guest_backup_authentication_and_wrong_recipient_fail_closed() {
        let recovery = StaticSecret::random_from_rng(OsRng);
        let seeds = Seeds {
            signing: [1; 32],
            transport: [2; 32],
            storage: [3; 32],
            root: [4; 32],
        };
        let mut capsule = encrypt(
            PublicKey::from(&recovery).to_bytes(),
            &bytes(&seeds),
            BACKUP,
        )
        .unwrap();
        assert_eq!(
            decrypt_seeds(recovery.to_bytes(), &capsule)
                .unwrap()
                .storage,
            seeds.storage
        );
        assert!(decrypt_seeds([9; 32], &capsule).is_err());
        capsule.ciphertext[0] ^= 1;
        assert!(decrypt_seeds(recovery.to_bytes(), &capsule).is_err());
    }
    #[test]
    fn tdx_guest_cannot_vote_without_configured_witness_and_durable_backup_ack() {
        let (mut g, key) = guest();
        assert!(g.command(WorkerCommand::Head).is_err());
        let envelope = encrypt(g.config.recovery, b"backup", BACKUP).unwrap();
        g.backup = Some(envelope.clone());
        let mut ack = backup_ack(g.identity(), &envelope, &key);
        ack.boot[0] ^= 1;
        assert!(g.activate(ack).is_err());
        assert!(!g.active);
        let other = SigningKey::generate(&mut OsRng);
        assert!(g
            .activate(backup_ack(g.identity(), &envelope, &other))
            .is_err());
        g.activate(backup_ack(g.identity(), &envelope, &key))
            .unwrap();
        assert!(g.command(WorkerCommand::Head).is_err()); // No witness-backed replica.
    }
    #[test]
    fn tdx_guest_rejects_host_configuration_tampering_before_witness_io() {
        let (mut g, key) = guest();
        let mut members = std::array::from_fn(|_| Member {
            signing: SigningKey::generate(&mut OsRng).verifying_key().to_bytes(),
            transport: PublicKey::from(&StaticSecret::random_from_rng(OsRng)).to_bytes(),
        });
        members[0] = g.identity().member.clone();
        let policy = WitnessPolicy {
            native_scope: None,
            committee: Committee { chain: 24, members },
            genesis: Head {
                chain: 24,
                epoch: 0,
                commitment: [5; 32],
            },
            authorities: std::array::from_fn(|_| {
                SigningKey::generate(&mut OsRng).verifying_key().to_bytes()
            }),
            observer: SigningKey::generate(&mut OsRng).verifying_key().to_bytes(),
        };
        let body = Provision {
            economics: None,
            root: [6; 32],
            allocations: vec![([7; 32], 1000)],
            restored: None,
        };
        for attack in 0..4 {
            let mut request = provision(
                g.identity(),
                policy.clone(),
                "127.0.0.1:1".parse().unwrap(),
                &body,
                &key,
            )
            .unwrap();
            match attack {
                0 => request.signature[0] ^= 1,
                1 => request.configuration.boot[0] ^= 1,
                2 => {
                    request.configuration.capsule.ciphertext[0] ^= 1;
                    request.signature = key
                        .sign(&request.configuration.message())
                        .to_bytes()
                        .to_vec();
                }
                _ => {
                    request.configuration.policy.committee.members[0] =
                        request.configuration.policy.committee.members[1].clone();
                    request.signature = key
                        .sign(&request.configuration.message())
                        .to_bytes()
                        .to_vec();
                }
            }
            assert!(g.configure(request, Path::new("/unused")).is_err());
            assert!(g.replica.is_none() && !g.active);
        }
    }
    #[test]
    fn tdx_guest_missing_native_library_never_creates_quote_generator() {
        let (g, _) = guest();
        assert!(QuoteGenerator::load(&g.config).is_err());
    }
}
