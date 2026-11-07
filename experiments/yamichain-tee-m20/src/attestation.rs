//! M22: official Intel QVL executed INSIDE the wallet's trusted process.
//! Offline cryptographic verification is separate from live challenge authorization.
use crate::{
    quorum::*,
    witness::{WitnessPolicy, WitnessedCertificate},
    *,
};
use libloading::Library;
use openssl::{
    asn1::Asn1Time,
    x509::{X509Crl, X509},
};
use std::{
    ffi::{c_char, c_int, CString},
    fmt,
    io::Read,
    os::fd::AsRawFd,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
const MAX_QUOTE: usize = 1024 * 1024;
const MAX_TEXT: usize = 1024 * 1024;
const ROOT: &str = include_str!("../native/intel-sgx-root.pem");
const TTL: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub struct QvlFailure {
    pub stage: &'static str,
    pub status: i32,
}
impl fmt::Display for QvlFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Intel QVL {} rejected, status {}",
            self.stage, self.status
        )
    }
}
impl std::error::Error for QvlFailure {}
fn status(stage: &'static str, code: i32) -> Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(Box::new(QvlFailure {
            stage,
            status: code,
        }))
    }
}
type SizeFn = unsafe extern "C" fn(*const u8, u32, *mut u32) -> c_int;
type DataFn = unsafe extern "C" fn(*const u8, u32, u32, *mut u8, *mut u16) -> c_int;
type PckFn = unsafe extern "C" fn(
    *const c_char,
    *const *const c_char,
    *const c_char,
    *const libc::time_t,
) -> c_int;
type CollateralFn = unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    *const c_char,
    *const c_char,
    *const libc::time_t,
) -> c_int;
type QuoteFn = unsafe extern "C" fn(
    *const u8,
    u32,
    *const c_char,
    *const c_char,
    *const c_char,
    *const c_char,
) -> c_int;

/// These function pointers are private and always keep their native library alive.
pub struct IntelQvl {
    _library: Library,
    _file: File,
    get_size: SizeFn,
    get_data: DataFn,
    pck: PckFn,
    tcb: CollateralFn,
    qe: CollateralFn,
    quote: QuoteFn,
}
impl IntelQvl {
    /// Caller pins the trusted native build digest in WALLET configuration, never in host evidence.
    /// Linux 64-bit loader: an open file descriptor pins the checked inode across dlopen.
    pub fn load(path: &Path, expected_digest: Id) -> Result<Self> {
        check(cfg!(all(target_os = "linux", target_pointer_width = "64")) && path.is_absolute())?;
        let mut file = File::open(path)?;
        check(file.metadata()?.len() <= 128 * 1024 * 1024)?;
        let mut binary = Vec::new();
        file.read_to_end(&mut binary)?;
        check(hash(&binary) == expected_digest)?;
        // SAFETY: native code is an explicitly trusted pinned Intel QVL build. The open FD
        // pins its inode. Dependencies and the library must remain immutable in the trusted
        // wallet OS. Loading arbitrary host-selected code here would violate the trust model.
        let library = unsafe { Library::new(format!("/proc/self/fd/{}", file.as_raw_fd()))? };
        // SAFETY: these exact prototypes match Intel QuoteVerification.h at the pinned commit.
        // Copies of symbols never outlive the owning Library stored in this object.
        unsafe {
            Ok(Self {
                get_size: *library.get(b"sgxAttestationGetQECertificationDataSize\0")?,
                get_data: *library.get(b"sgxAttestationGetQECertificationData\0")?,
                pck: *library.get(b"sgxAttestationVerifyPCKCertificate\0")?,
                tcb: *library.get(b"sgxAttestationVerifyTCBInfo\0")?,
                qe: *library.get(b"sgxAttestationVerifyEnclaveIdentity\0")?,
                quote: *library.get(b"sgxAttestationVerifyQuote\0")?,
                _library: library,
                _file: file,
            })
        }
    }
    /// Unverified quote certification data for retrieving collateral. This grants no authorization.
    #[cfg(feature = "tdx-guest-devnet")]
    pub fn unverified_pck_chain(&self, quote: &[u8]) -> Result<String> {
        TdxReport::parse(quote)?;
        let mut size = 0;
        status("certification size", unsafe {
            (self.get_size)(quote.as_ptr(), quote.len() as u32, &mut size)
        })?;
        check(size > 0 && size as usize <= MAX_TEXT)?;
        let mut chain = vec![0; size as usize];
        let mut kind = 0;
        status("certification data", unsafe {
            (self.get_data)(
                quote.as_ptr(),
                quote.len() as u32,
                size,
                chain.as_mut_ptr(),
                &mut kind,
            )
        })?;
        check(kind == 5)?;
        if chain.last() == Some(&0) {
            chain.pop();
        }
        Ok(String::from_utf8(chain)?)
    }
    /// Offline diagnostic API. Historical time is allowed here ONLY for archived fixture tests.
    /// This result cannot create a wallet authorization; the live gate uses its own trusted clock.
    pub fn verify_at(
        &self,
        quote: &[u8],
        collateral: &Collateral,
        unix: u64,
    ) -> Result<VerifiedQuote> {
        let report = TdxReport::parse(quote)?;
        let now = libc::time_t::try_from(unix)?;
        let c = collateral.native()?;
        let root = CString::new(ROOT)?;
        let mut size = 0u32;
        // SAFETY: bounded immutable quote and valid exclusive u32 output; synchronous API.
        status("certification size", unsafe {
            (self.get_size)(quote.as_ptr(), quote.len() as u32, &mut size)
        })?;
        check(size > 0 && size as usize <= MAX_TEXT)?;
        let mut chain = vec![0; size as usize];
        let mut kind = 0u16;
        // SAFETY: Intel reported the exact output capacity, backed by a live mutable Vec.
        status("certification data", unsafe {
            (self.get_data)(
                quote.as_ptr(),
                quote.len() as u32,
                size,
                chain.as_mut_ptr(),
                &mut kind,
            )
        })?;
        check(kind == 5)?; // Full PCK certificate chain; no external unverified leaf substitution.
        if chain.last() == Some(&0) {
            chain.pop();
        }
        let chain = CString::new(chain)?;
        let crls = [c.root_crl.as_ptr(), c.pck_crl.as_ptr()];
        // SAFETY: all CStrings/array/time_t live across these calls. CRL array has EXACTLY
        // the two slots required by Intel. No native function retains caller memory.
        status("PCK chain and revocation", unsafe {
            (self.pck)(chain.as_ptr(), crls.as_ptr(), root.as_ptr(), &now)
        })?;
        status("TCB signature and expiry", unsafe {
            (self.tcb)(
                c.tcb.as_ptr(),
                c.tcb_chain.as_ptr(),
                c.root_crl.as_ptr(),
                root.as_ptr(),
                &now,
            )
        })?;
        status("QE identity signature and expiry", unsafe {
            (self.qe)(
                c.qe.as_ptr(),
                c.qe_chain.as_ptr(),
                c.root_crl.as_ptr(),
                root.as_ptr(),
                &now,
            )
        })?;
        // Intel's lower-level QVL checks expiry; additionally reject not-yet-valid
        // certificates, CRLs and signed collateral at the same trusted timestamp.
        check_chain_time(chain.to_str()?, unix)
            .map_err(|e| format!("PCK certificate validity: {e}"))?;
        check_chain_time(&collateral.tcb_info_issuer_chain, unix)
            .map_err(|e| format!("TCB certificate validity: {e}"))?;
        check_chain_time(&collateral.qe_identity_issuer_chain, unix)
            .map_err(|e| format!("QE certificate validity: {e}"))?;
        check_crl_time(&collateral.root_ca_crl, unix)
            .map_err(|e| format!("root CRL validity: {e}"))?;
        check_crl_time(&collateral.pck_crl, unix).map_err(|e| format!("PCK CRL validity: {e}"))?;
        signed_time(&collateral.tcb_info, unix).map_err(|e| format!("TCB signed dates: {e}"))?;
        signed_time(&collateral.qe_identity, unix).map_err(|e| format!("QE signed dates: {e}"))?;
        // SAFETY: immutable bounded quote and authenticated collateral CStrings live
        // through the call. Only STATUS_OK is accepted, including TDX TCB/module status.
        status("quote signature and TCB", unsafe {
            (self.quote)(
                quote.as_ptr(),
                quote.len() as u32,
                chain.as_ptr(),
                c.pck_crl.as_ptr(),
                c.tcb.as_ptr(),
                c.qe.as_ptr(),
            )
        })?;
        Ok(VerifiedQuote {
            report,
            unix,
            quote_hash: hash(quote),
        })
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Collateral {
    pub pck_crl_issuer_chain: String,
    pub root_ca_crl: String,
    pub pck_crl: String,
    pub tcb_info_issuer_chain: String,
    pub tcb_info: String,
    pub tcb_info_signature: String,
    pub qe_identity_issuer_chain: String,
    pub qe_identity: String,
    pub qe_identity_signature: String,
}
struct NativeCollateral {
    root_crl: CString,
    pck_crl: CString,
    tcb_chain: CString,
    qe_chain: CString,
    tcb: CString,
    qe: CString,
}
fn text(s: &str) -> Result<CString> {
    check(!s.is_empty() && s.len() <= MAX_TEXT)?;
    Ok(CString::new(s)?)
}
impl Collateral {
    fn native(&self) -> Result<NativeCollateral> {
        for signature in [&self.tcb_info_signature, &self.qe_identity_signature] {
            check(signature.len() == 128 && signature.bytes().all(|b| b.is_ascii_hexdigit()))?;
        }
        text(&self.pck_crl_issuer_chain)?;
        text(&self.tcb_info)?;
        text(&self.qe_identity)?;
        Ok(NativeCollateral {
            root_crl: text(&self.root_ca_crl)?,
            pck_crl: text(&self.pck_crl)?,
            tcb_chain: text(&self.tcb_info_issuer_chain)?,
            qe_chain: text(&self.qe_identity_issuer_chain)?,
            tcb: text(&format!(
                "{{\"tcbInfo\":{},\"signature\":\"{}\"}}",
                self.tcb_info, self.tcb_info_signature
            ))?,
            qe: text(&format!(
                "{{\"enclaveIdentity\":{},\"signature\":\"{}\"}}",
                self.qe_identity, self.qe_identity_signature
            ))?,
        })
    }
}
fn check_chain_time(pem: &str, unix: u64) -> Result<()> {
    let now = Asn1Time::from_unix(i64::try_from(unix)?)?;
    let certs = X509::stack_from_pem(pem.as_bytes())?;
    check(!certs.is_empty())?;
    for cert in certs {
        check(cert.not_before() <= now.as_ref() && now.as_ref() <= cert.not_after())?;
    }
    Ok(())
}
fn check_crl_time(raw: &str, unix: u64) -> Result<()> {
    let crl = if raw.starts_with("-----BEGIN") {
        X509Crl::from_pem(raw.as_bytes())?
    } else {
        X509Crl::from_der(&hex::decode(raw)?)?
    };
    let now = Asn1Time::from_unix(i64::try_from(unix)?)?;
    check(
        crl.last_update() <= now.as_ref()
            && now.as_ref() <= crl.next_update().ok_or("CRL has no nextUpdate")?,
    )
}
fn signed_time(raw: &str, unix: u64) -> Result<()> {
    let json: serde_json::Value = serde_json::from_str(raw)?;
    let timestamp = |name: &str| -> Result<i64> {
        Ok(
            OffsetDateTime::parse(json[name].as_str().ok_or("missing signed date")?, &Rfc3339)?
                .unix_timestamp(),
        )
    };
    let now = i64::try_from(unix)?;
    check(timestamp("issueDate")? <= now && now <= timestamp("nextUpdate")?)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TdxReport {
    pub mrtd: Vec<u8>,
    pub rtmrs: [Vec<u8>; 4],
    pub config_id: Vec<u8>,
    pub owner: Vec<u8>,
    pub owner_config: Vec<u8>,
    pub td_attributes: u64,
    pub xfam: u64,
    pub seam_attributes: u64,
    pub report_data: [u8; 64],
    pub extension: Vec<u8>,
}
impl TdxReport {
    /// Strict supported profiles: v4 TDX 1.0; v5 TD10/TD15 (584/648-byte bodies).
    /// SGX, newer EX extensions, truncation, oversized lengths and trailing bytes reject.
    pub(crate) fn parse(q: &[u8]) -> Result<Self> {
        check(q.len() >= 636 && q.len() <= MAX_QUOTE)?;
        check(
            u16::from_le_bytes(q[2..4].try_into()?) == 2
                && u32::from_le_bytes(q[4..8].try_into()?) == 0x81,
        )?;
        let version = u16::from_le_bytes(q[0..2].try_into()?);
        let (offset, len) = match version {
            4 => (48, 584),
            5 => {
                let kind = u16::from_le_bytes(q[48..50].try_into()?);
                let size = u32::from_le_bytes(q[50..54].try_into()?) as usize;
                check(matches!((kind, size), (2, 584) | (3, 648)))?;
                (54, size)
            }
            _ => return Err("unsupported TDX quote version".into()),
        };
        let signature_offset = offset + len;
        check(q.len() >= signature_offset + 4)?;
        let signature =
            u32::from_le_bytes(q[signature_offset..signature_offset + 4].try_into()?) as usize;
        check(signature > 0 && signature.checked_add(signature_offset + 4) == Some(q.len()))?;
        let b = &q[offset..offset + len];
        let u64_at =
            |i| -> u64 { u64::from_le_bytes(b[i..i + 8].try_into().expect("fixed report field")) };
        Ok(Self {
            mrtd: b[136..184].to_vec(),
            rtmrs: std::array::from_fn(|i| b[328 + i * 48..376 + i * 48].to_vec()),
            config_id: b[184..232].to_vec(),
            owner: b[232..280].to_vec(),
            owner_config: b[280..328].to_vec(),
            td_attributes: u64_at(120),
            xfam: u64_at(128),
            seam_attributes: u64_at(112),
            report_data: b[520..584].try_into()?,
            extension: b[584..].to_vec(),
        })
    }
}
/// Cannot be constructed or deserialized by a host. Only native QVL can mint this result.
pub struct VerifiedQuote {
    report: TdxReport,
    unix: u64,
    quote_hash: Id,
}
impl VerifiedQuote {
    pub fn report(&self) -> &TdxReport {
        &self.report
    }
    pub fn verified_at(&self) -> u64 {
        self.unix
    }
    pub fn quote_hash(&self) -> Id {
        self.quote_hash
    }
    pub fn enforce(&self, policy: &SoftwarePolicy, expected_report_data: &[u8; 64]) -> Result<()> {
        policy.validate()?;
        let r = &self.report;
        check(
            r.td_attributes & 1 == 0
                && r.mrtd == policy.mrtd
                && r.rtmrs == policy.rtmrs
                && r.config_id == policy.config_id
                && r.owner == policy.owner
                && r.owner_config == policy.owner_config
                && r.td_attributes == policy.td_attributes
                && r.xfam == policy.xfam
                && r.seam_attributes == policy.seam_attributes
                && r.extension == policy.extension
                && r.report_data == *expected_report_data,
        )
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoftwarePolicy {
    pub identity: Id,
    pub mrtd: Vec<u8>,
    pub rtmrs: [Vec<u8>; 4],
    pub config_id: Vec<u8>,
    pub owner: Vec<u8>,
    pub owner_config: Vec<u8>,
    pub td_attributes: u64,
    pub xfam: u64,
    pub seam_attributes: u64,
    pub extension: Vec<u8>,
}
impl SoftwarePolicy {
    fn validate(&self) -> Result<()> {
        check(
            matches!(self.extension.len(), 0 | 64)
                && self.td_attributes & 1 == 0
                && self.mrtd.len() == 48
                && self.rtmrs.iter().all(|r| r.len() == 48)
                && self.config_id.len() == 48
                && self.owner.len() == 48
                && self.owner_config.len() == 48,
        )
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub quote: Vec<u8>,
    pub collateral: Collateral,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ChallengeSet {
    pub id: Id,
    pub challenges: [Id; 4],
    pub expected_report_data: [Vec<u8>; 4],
}
enum Intent {
    Payment { transfer: Signed, previous: Head },
    Query(Query),
}
struct Round {
    public: ChallengeSet,
    started: Instant,
    unix: u64,
    intent: Intent,
}
/// All decisions run locally in this wallet process. No host boolean/result is consumed.
/// A round is single-use, is never restored after restart, and expires after 60 seconds.
pub struct WalletGate {
    qvl: IntelQvl,
    committee: Committee,
    genesis: Head,
    policies: [SoftwarePolicy; 4],
    pending: Option<Round>,
    witness_policy: Option<WitnessPolicy>,
}
impl WalletGate {
    fn new(
        qvl: IntelQvl,
        committee: Committee,
        genesis: Head,
        policies: [SoftwarePolicy; 4],
    ) -> Result<Self> {
        committee.validate()?;
        check(genesis.chain == committee.chain && genesis.epoch == 0)?;
        for policy in &policies {
            policy.validate()?;
        }
        Ok(Self {
            qvl,
            committee,
            genesis,
            policies,
            pending: None,
            witness_policy: None,
        })
    }
    /// M23 wallet profile: attestation also binds the independently pinned witness policy.
    pub fn new_witnessed(
        qvl: IntelQvl,
        policy: WitnessPolicy,
        policies: [SoftwarePolicy; 4],
    ) -> Result<Self> {
        policy.validate()?;
        let mut gate = Self::new(
            qvl,
            policy.committee.clone(),
            policy.genesis.clone(),
            policies,
        )?;
        gate.witness_policy = Some(policy);
        Ok(gate)
    }
    fn begin(&mut self, intent: Intent) -> Result<ChallengeSet> {
        check(self.pending.is_none())?;
        let mut id = [0; 32];
        OsRng.fill_bytes(&mut id);
        let challenges = std::array::from_fn(|_| {
            let mut c = [0; 32];
            OsRng.fill_bytes(&mut c);
            c
        });
        let mut expected = Vec::new();
        for (i, challenge) in challenges.iter().enumerate() {
            expected.push(
                tdx_report_data(
                    &self.committee,
                    i,
                    &self.genesis,
                    self.witness_policy
                        .as_ref()
                        .map_or(self.policies[i].identity, |p| {
                            hash(&bytes(&(
                                "YAMI-M23-software-and-witness-policy-v1",
                                self.policies[i].identity,
                                p.id(),
                            )))
                        }),
                    *challenge,
                )?
                .to_vec(),
            );
        }
        let public = ChallengeSet {
            id,
            challenges,
            expected_report_data: expected.try_into().map_err(|_| "four members")?,
        };
        self.pending = Some(Round {
            public: public.clone(),
            started: Instant::now(),
            unix: wallet_time()?,
            intent,
        });
        Ok(public)
    }
    /// Caller holds the wallet signing key and supplies the signed transfer; only this
    /// attested delivery path encrypts after authenticating at least three pinned executors.
    pub fn begin_payment(&mut self, transfer: Signed, previous: Head) -> Result<ChallengeSet> {
        transfer.verify()?;
        check(
            transfer.transfer.chain == self.committee.chain
                && previous.chain == self.committee.chain,
        )?;
        self.begin(Intent::Payment { transfer, previous })
    }
    pub fn begin_query(&mut self, query: Query) -> Result<ChallengeSet> {
        check(query.head.chain == self.committee.chain && query.challenge != [0; 32])?;
        verify(query.account, &query.message(), &query.signature)?;
        self.begin(Intent::Query(query))
    }
    pub fn cancel(&mut self) {
        self.pending = None;
    }
    /// Consume even a failed attempt. No quote/result/session can be replayed into a new round.
    /// Historical diagnostic VerifiedQuote values are deliberately NOT accepted here.
    pub fn finish(&mut self, id: Id, evidence: &[Option<Evidence>; 4]) -> Result<WalletRequest> {
        let round = self
            .pending
            .take()
            .ok_or("no outstanding attestation challenge")?;
        check(id == round.public.id && round.started.elapsed() <= TTL)?;
        let now = wallet_time()?;
        check(now >= round.unix && now - round.unix <= TTL.as_secs())?;
        let mut seen = std::collections::BTreeSet::new();
        let mut selected = [false; 4];
        for (i, evidence) in evidence.iter().enumerate() {
            if let Some(e) = evidence {
                let result = (|| -> Result<()> {
                    let verified = self.qvl.verify_at(&e.quote, &e.collateral, now)?;
                    let expected: [u8; 64] =
                        round.public.expected_report_data[i].as_slice().try_into()?;
                    verified.enforce(&self.policies[i], &expected)?;
                    check(seen.insert(verified.quote_hash()))
                })();
                selected[i] = result.is_ok();
            }
        }
        require_attested_quorum(selected)?;
        let end = wallet_time()?;
        check(round.started.elapsed() <= TTL && end >= now && end - round.unix <= TTL.as_secs())?;
        // First encryption occurs here, only for the at least three genuinely verified members.
        match round.intent {
            Intent::Payment { transfer, previous } => Ok(WalletRequest::Payment(GatedPayment {
                payment: Payment::software_selected(
                    &self.committee,
                    &previous,
                    transfer,
                    selected,
                )?,
                committee: self.committee.clone(),
                witness_policy: self.witness_policy.clone(),
            })),
            Intent::Query(query) => {
                let mut envelopes = Vec::new();
                for (i, member) in self.committee.members.iter().enumerate() {
                    envelopes.push(if selected[i] {
                        Some(encrypt(member.transport, &bytes(&query), QUERY_DOMAIN)?)
                    } else {
                        None
                    });
                }
                Ok(WalletRequest::Queries(GatedQueries {
                    committee: self.committee.clone(),
                    query,
                    witness_policy: self.witness_policy.clone(),
                    envelopes: envelopes.try_into().map_err(|_| "four queries")?,
                }))
            }
        }
    }
    pub fn finish_and_submit<T>(
        &mut self,
        id: Id,
        evidence: &[Option<Evidence>; 4],
        submit: impl FnOnce(WalletRequest) -> Result<T>,
    ) -> Result<T> {
        let request = self.finish(id, evidence)?;
        submit(request)
    }
}
fn wallet_time() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}
/// These request wrappers have no public constructors and cannot be deserialized from a host.
pub enum WalletRequest {
    Payment(GatedPayment),
    Queries(GatedQueries),
}
pub struct GatedPayment {
    payment: Payment,
    committee: Committee,
    witness_policy: Option<WitnessPolicy>,
}
impl GatedPayment {
    pub fn packet(&self) -> &Payment {
        &self.payment
    }
    /// Attestation authenticates executors; it never replaces financial quorum verification.
    pub fn verify_receipt(&self, qc: &Certificate) -> Result<()> {
        check(self.witness_policy.is_none())?;
        qc.verify(&self.committee, &self.payment.previous)?;
        check(qc.statement.transaction == self.payment.transaction)
    }
    pub fn verify_witnessed_receipt(&self, qc: &WitnessedCertificate) -> Result<()> {
        let p = self
            .witness_policy
            .as_ref()
            .ok_or("not a witness-backed request")?;
        qc.verify(p, &self.payment.previous)?;
        check(qc.certificate.statement.transaction == self.payment.transaction)
    }
}
pub struct GatedQueries {
    committee: Committee,
    witness_policy: Option<WitnessPolicy>,
    query: Query,
    envelopes: [Option<Envelope>; 4],
}
impl GatedQueries {
    pub fn envelopes(&self) -> &[Option<Envelope>; 4] {
        &self.envelopes
    }
    pub fn verify_replies(&self, secret: &StaticSecret, replies: &[Envelope]) -> Result<Account> {
        check(self.witness_policy.is_none())?;
        verify_balance(&self.committee, &self.query, secret, replies)
    }
    pub fn verify_witnessed_replies(
        &self,
        secret: &StaticSecret,
        replies: &[Envelope],
        certificate: Option<&WitnessedCertificate>,
    ) -> Result<Account> {
        let p = self
            .witness_policy
            .as_ref()
            .ok_or("not a witness-backed query")?;
        if self.query.head != p.genesis {
            let qc = certificate.ok_or("certified head required")?;
            qc.verify(p, &qc.certificate.statement.previous)?;
            check(qc.certificate.statement.next == self.query.head)?;
        }
        verify_balance(&self.committee, &self.query, secret, replies)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Signer;
    fn wallet() -> SigningKey {
        SigningKey::generate(&mut OsRng)
    }
    /// White-box request/receipt context test only; deliberately NOT an attestation-positive test.
    /// No fake quote, native decision or wallet authorization is manufactured.
    #[test]
    fn attestation_request_context_never_substitutes_for_three_financial_signatures() {
        let signers: [SigningKey; 4] = std::array::from_fn(|_| wallet());
        let c = Committee {
            chain: 22,
            members: std::array::from_fn(|i| Member {
                signing: signers[i].verifying_key().to_bytes(),
                transport: PublicKey::from(&StaticSecret::random_from_rng(OsRng)).to_bytes(),
            }),
        };
        let owner = wallet();
        let recipient = wallet();
        let previous = Head {
            chain: 22,
            epoch: 0,
            commitment: [7; 32],
        };
        let transfer = Signed::sign(
            Transfer {
                sender: owner.verifying_key().to_bytes(),
                recipient: recipient.verifying_key().to_bytes(),
                amount: 1,
                nonce: 0,
                chain: 22,
            },
            &owner,
        );
        let payment = Payment::software(&c, &previous, transfer).unwrap();
        let statement = Statement {
            committee: c.id(),
            previous: previous.clone(),
            next: Head {
                chain: 22,
                epoch: 1,
                commitment: [8; 32],
            },
            transaction: payment.transaction,
        };
        let approvals: Vec<_> = signers
            .iter()
            .map(|s| Approval {
                member: s.verifying_key().to_bytes(),
                statement: statement.clone(),
                signature: s.sign(&statement.message()).to_bytes().to_vec(),
            })
            .collect();
        let context = GatedPayment {
            payment,
            committee: c.clone(),
            witness_policy: None,
        };
        let mut qc = Certificate {
            statement: statement.clone(),
            approvals: approvals[..2].to_vec(),
        };
        assert!(context.verify_receipt(&qc).is_err());
        qc.approvals = approvals[..3].to_vec();
        context.verify_receipt(&qc).unwrap();
        let witness_policy = WitnessPolicy {
            committee: c.clone(),
            genesis: previous.clone(),
            authorities: std::array::from_fn(|_| wallet().verifying_key().to_bytes()),
            observer: wallet().verifying_key().to_bytes(),
        };
        let secured = GatedPayment {
            payment: context.payment.clone(),
            committee: c.clone(),
            witness_policy: Some(witness_policy),
        };
        assert!(
            secured.verify_receipt(&qc).is_err(),
            "M23 cannot downgrade to bare financial signatures"
        );
        assert!(secured
            .verify_witnessed_receipt(&WitnessedCertificate {
                certificate: qc.clone(),
                confirmations: vec![]
            })
            .is_err());
        qc.approvals = vec![approvals[0].clone(); 3];
        assert!(context.verify_receipt(&qc).is_err());
        let mut another = statement;
        another.transaction = [99; 32];
        let approvals: Vec<_> = signers
            .iter()
            .take(3)
            .map(|s| Approval {
                member: s.verifying_key().to_bytes(),
                statement: another.clone(),
                signature: s.sign(&another.message()).to_bytes().to_vec(),
            })
            .collect();
        assert!(context
            .verify_receipt(&Certificate {
                statement: another,
                approvals
            })
            .is_err());
    }
    #[test]
    fn unverified_quote_parser_is_bounded_and_not_an_authorization() {
        let raw = include_bytes!("../tests/fixtures/m22/tdx_quote");
        assert!(TdxReport::parse(raw).is_ok());
        assert!(TdxReport::parse(include_bytes!(
            "../tests/fixtures/m22/tdx_quote_upstream_padded"
        ))
        .is_err());
        let mut q = raw.to_vec();
        q[632..636].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(TdxReport::parse(&q).is_err());
        for n in [0, 1, 48, 54, 635] {
            assert!(TdxReport::parse(&raw[..n]).is_err());
        }
    }
    #[test]
    #[ignore = "requires explicitly pinned official Intel QVL native build"]
    fn native_wallet_rounds_reject_deadlines_wrong_round_ids_and_backwards_time() {
        let path = PathBuf::from(std::env::var("YAMI_QVL_LIBRARY").unwrap());
        let pin: Id = hex::decode(std::env::var("YAMI_QVL_SHA256").unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let qvl = IntelQvl::load(&path, pin).unwrap();
        let owner = wallet();
        let c = Committee {
            chain: 22,
            members: std::array::from_fn(|_| Member {
                signing: wallet().verifying_key().to_bytes(),
                transport: PublicKey::from(&StaticSecret::random_from_rng(OsRng)).to_bytes(),
            }),
        };
        let head = Head {
            chain: 22,
            epoch: 0,
            commitment: [1; 32],
        };
        let policy = SoftwarePolicy {
            identity: [2; 32],
            mrtd: vec![1; 48],
            rtmrs: std::array::from_fn(|_| vec![1; 48]),
            config_id: vec![0; 48],
            owner: vec![0; 48],
            owner_config: vec![0; 48],
            td_attributes: 0,
            xfam: 0,
            seam_attributes: 0,
            extension: vec![],
        };
        let mut gate = WalletGate::new(
            qvl,
            c,
            head.clone(),
            std::array::from_fn(|_| policy.clone()),
        )
        .unwrap();
        let evidence: [Option<Evidence>; 4] = std::array::from_fn(|_| {
            Some(Evidence {
                quote: vec![],
                collateral: serde_json::from_str(include_str!(
                    "../tests/fixtures/m22/tdx_quote_collateral.json"
                ))
                .unwrap(),
            })
        });
        for mode in 0..3 {
            let t = Signed::sign(
                Transfer {
                    sender: owner.verifying_key().to_bytes(),
                    recipient: wallet().verifying_key().to_bytes(),
                    amount: 1,
                    nonce: 0,
                    chain: 22,
                },
                &owner,
            );
            let challenge = gate.begin_payment(t, head.clone()).unwrap();
            if mode == 0 {
                gate.pending.as_mut().unwrap().started = Instant::now()
                    .checked_sub(TTL + Duration::from_secs(1))
                    .unwrap();
            }
            if mode == 1 {
                gate.pending.as_mut().unwrap().unix += 3600;
            }
            let id = if mode == 2 { [0; 32] } else { challenge.id };
            assert!(gate.finish(id, &evidence).is_err());
            assert!(gate.pending.is_none());
            assert!(gate.finish(challenge.id, &evidence).is_err());
        }
    }
}

fn require_attested_quorum(selected: [bool; 4]) -> Result<()> {
    check(selected.into_iter().filter(|yes| *yes).count() >= 3)
}
#[cfg(test)]
mod m23_selection_tests {
    use super::*;
    #[test]
    fn verification_selection_requires_three_independent_native_decisions() {
        // Selection algorithm only; no fake quote or public verifier-success injection.
        for mask in 0u8..16 {
            let selected = std::array::from_fn(|i| mask & (1 << i) != 0);
            assert_eq!(
                require_attested_quorum(selected).is_ok(),
                mask.count_ones() >= 3
            );
        }
    }
}
