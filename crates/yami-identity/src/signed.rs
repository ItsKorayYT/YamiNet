//! Domain-separated Ed25519 signatures over canonical metadata.
//!
//! The signed message is `purpose tag || canonical bytes`. Tags are distinct
//! NUL-terminated ASCII strings, so no tag is a prefix of another and a
//! signature made for one purpose never verifies for another. Each tag names
//! the version of its canonical encoding; a new encoding gets a new tag.
use crate::{hex, unhex, Error, NodeId, NodeIdentity, Result};
use ed25519_dalek::Signer as _;
use sha2::{Digest as _, Sha256};
use std::{fmt, net::SocketAddrV4, str::FromStr};
use ynp::DocumentData;

pub type Digest = [u8; 32];

pub fn sha256(bytes: &[u8]) -> Digest {
    Sha256::digest(bytes).into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    DocumentMetadata,
    SiteManifest,
    PeerRecord,
    RelayRegistration,
}
impl Purpose {
    pub const ALL: [Self; 4] = [
        Self::DocumentMetadata,
        Self::SiteManifest,
        Self::PeerRecord,
        Self::RelayRegistration,
    ];
    /// The domain separation prefix of every message signed for this purpose.
    pub fn tag(self) -> &'static [u8] {
        match self {
            Self::DocumentMetadata => b"yaminet.document-metadata.v1\0",
            Self::SiteManifest => b"yaminet.site-manifest.v1\0",
            Self::PeerRecord => b"yaminet.peer-record.v1\0",
            Self::RelayRegistration => b"yaminet.relay-registration.v1\0",
        }
    }
    pub fn name(self) -> &'static str {
        let tag = self.tag();
        std::str::from_utf8(&tag[..tag.len() - 1]).expect("tags are ASCII")
    }
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|purpose| purpose.name() == name)
    }
}

mod sealed {
    pub trait Sealed {}
}
/// Metadata with one canonical byte encoding. Sealed: only this crate assigns
/// purposes, so every purpose has exactly one encoding.
pub trait Signable: sealed::Sealed {
    const PURPOSE: Purpose;
    fn canonical_bytes(&self) -> Vec<u8>;
}

fn message(purpose: Purpose, body: &[u8]) -> Vec<u8> {
    [purpose.tag(), body].concat()
}
fn put_key(output: &mut Vec<u8>, key: &str) {
    output.extend_from_slice(&(key.len() as u16).to_be_bytes());
    output.extend_from_slice(key.as_bytes());
}

/// Binds a YNP key to the exact document and style bytes served under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentMetadata {
    key: String,
    source_sha256: Digest,
    style_sha256: Digest,
}
impl DocumentMetadata {
    pub fn new(key: &str, source_sha256: Digest, style_sha256: Digest) -> Result<Self> {
        ynp::validate_key(key).map_err(|_| Error::Malformed("invalid document key"))?;
        Ok(Self {
            key: key.into(),
            source_sha256,
            style_sha256,
        })
    }
    pub fn of(document: &DocumentData) -> Result<Self> {
        Self::new(
            &document.key,
            sha256(document.source.as_bytes()),
            sha256(document.style.as_bytes()),
        )
    }
    pub fn key(&self) -> &str {
        &self.key
    }
}
impl sealed::Sealed for DocumentMetadata {}
impl Signable for DocumentMetadata {
    const PURPOSE: Purpose = Purpose::DocumentMetadata;
    /// `u16 key length || key || SHA-256(source) || SHA-256(style)`.
    fn canonical_bytes(&self) -> Vec<u8> {
        let mut output = Vec::with_capacity(2 + self.key.len() + 64);
        put_key(&mut output, &self.key);
        output.extend_from_slice(&self.source_sha256);
        output.extend_from_slice(&self.style_sha256);
        output
    }
}

/// Every page of a directory site and its shared style. Signing a manifest
/// covers additions and removals as well as page content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteManifest {
    style_sha256: Digest,
    pages: Vec<(String, Digest)>,
}
impl SiteManifest {
    pub const MAX_PAGES: usize = 4096;
    /// Pages are sorted by key bytes; duplicate or invalid keys are rejected.
    pub fn new(
        style_sha256: Digest,
        pages: impl IntoIterator<Item = (String, Digest)>,
    ) -> Result<Self> {
        let mut pages: Vec<_> = pages.into_iter().collect();
        if pages.len() > Self::MAX_PAGES {
            return Err(Error::Malformed("too many pages in site manifest"));
        }
        pages.sort_by(|a, b| a.0.cmp(&b.0));
        if pages.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(Error::Malformed("duplicate page key in site manifest"));
        }
        if pages.iter().any(|(key, _)| ynp::validate_key(key).is_err()) {
            return Err(Error::Malformed("invalid page key in site manifest"));
        }
        Ok(Self {
            style_sha256,
            pages,
        })
    }
    pub fn pages(&self) -> &[(String, Digest)] {
        &self.pages
    }
}
impl sealed::Sealed for SiteManifest {}

/// Canonical public endpoint claim. Network address policy and clock/rollback
/// checks belong to `yami-dht`; the signing API remains sealed and separated
/// from document, site and TLS signing domains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerRecordMetadata {
    id: NodeId,
    endpoints: Vec<SocketAddrV4>,
    expires: u64,
    sequence: u64,
}
impl PeerRecordMetadata {
    pub const MAX_ENDPOINTS: usize = 4;
    pub fn new(
        id: NodeId,
        endpoints: Vec<SocketAddrV4>,
        expires: u64,
        sequence: u64,
    ) -> Result<Self> {
        if endpoints.is_empty()
            || endpoints.len() > Self::MAX_ENDPOINTS
            || expires == 0
            || sequence == 0
            || endpoints.iter().any(|e| {
                e.port() == 0
                    || e.ip().is_unspecified()
                    || e.ip().is_multicast()
                    || e.ip().is_broadcast()
            })
            || endpoints.windows(2).any(|e| e[0] >= e[1])
        {
            return Err(Error::Malformed("peer record needs 1..4 strictly sorted IPv4 endpoints, positive expiry and sequence"));
        }
        Ok(Self {
            id,
            endpoints,
            expires,
            sequence,
        })
    }
    pub fn id(&self) -> NodeId {
        self.id
    }
    pub fn endpoints(&self) -> &[SocketAddrV4] {
        &self.endpoints
    }
    pub fn expires(&self) -> u64 {
        self.expires
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
}
impl sealed::Sealed for PeerRecordMetadata {}

/// One fresh reverse-relay challenge, bound to both identities and the exact
/// outer TLS session's exporter. This cannot sign document/DHT/TLS messages.
#[derive(Debug, Clone)]
pub struct RelayRegistration {
    pub relay: NodeId,
    pub host: NodeId,
    pub nonce: [u8; 32],
    pub channel: [u8; 32],
    pub expires: u64,
}
impl sealed::Sealed for RelayRegistration {}
impl Signable for RelayRegistration {
    const PURPOSE: Purpose = Purpose::RelayRegistration;
    fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(136);
        bytes.extend_from_slice(&self.relay.public_key());
        bytes.extend_from_slice(&self.host.public_key());
        bytes.extend_from_slice(&self.nonce);
        bytes.extend_from_slice(&self.channel);
        bytes.extend_from_slice(&self.expires.to_be_bytes());
        bytes
    }
}
impl Signable for PeerRecordMetadata {
    const PURPOSE: Purpose = Purpose::PeerRecord;
    fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(49 + 6 * self.endpoints.len());
        bytes.extend_from_slice(&self.id.public_key());
        bytes.extend_from_slice(&self.expires.to_be_bytes());
        bytes.extend_from_slice(&self.sequence.to_be_bytes());
        bytes.push(self.endpoints.len() as u8);
        for endpoint in &self.endpoints {
            bytes.extend_from_slice(&endpoint.ip().octets());
            bytes.extend_from_slice(&endpoint.port().to_be_bytes());
        }
        bytes
    }
}
impl Signable for SiteManifest {
    const PURPOSE: Purpose = Purpose::SiteManifest;
    /// `SHA-256(style) || u32 page count || (u16 key length || key || SHA-256(page))*`
    /// with pages in ascending key byte order.
    fn canonical_bytes(&self) -> Vec<u8> {
        let mut output = Vec::with_capacity(36 + self.pages.len() * (2 + 32 + 32));
        output.extend_from_slice(&self.style_sha256);
        output.extend_from_slice(&(self.pages.len() as u32).to_be_bytes());
        for (key, digest) in &self.pages {
            put_key(&mut output, key);
            output.extend_from_slice(digest);
        }
        output
    }
}

/// A raw 64-byte Ed25519 signature, written as 128 lowercase hex digits.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Signature([u8; 64]);
impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex(&self.0))
    }
}
impl Signature {
    pub fn from_bytes(bytes: [u8; 64]) -> Self {
        Self(bytes)
    }
    pub fn to_bytes(&self) -> [u8; 64] {
        self.0
    }
}
impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Signature({self})")
    }
}
impl FromStr for Signature {
    type Err = Error;
    fn from_str(text: &str) -> Result<Self> {
        let mut bytes = [0; 64];
        unhex(text, &mut bytes).ok_or(Error::Malformed(
            "signature must be 128 lowercase hex digits",
        ))?;
        Ok(Self(bytes))
    }
}

impl NodeIdentity {
    pub fn sign<T: Signable>(&self, value: &T) -> Signature {
        self.sign_message(T::PURPOSE, &value.canonical_bytes())
    }
    fn sign_message(&self, purpose: Purpose, body: &[u8]) -> Signature {
        Signature(self.signing_key().sign(&message(purpose, body)).to_bytes())
    }
}
/// Messages of the TLS transport (`ynp-tls`) that a node key signs directly.
/// The first byte keeps every signing domain apart: YamiNet metadata messages
/// start with a `yaminet.` tag (`y`), an X.509 `TBSCertificate` with a DER
/// SEQUENCE (0x30) and TLS 1.3 CertificateVerify content with 64 spaces (0x20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMessage {
    /// The DER `TBSCertificate` of the node's own self-signed certificate.
    Certificate,
    /// The content a TLS 1.3 server signs in CertificateVerify (RFC 8446
    /// section 4.4.3): 64 spaces, the server context string, a zero byte and
    /// the SHA-256 or SHA-384 handshake transcript hash.
    ServerCertificateVerify,
}
impl TlsMessage {
    const MAX_CERTIFICATE: usize = 4096;
    const SERVER_CONTEXT: &'static [u8] = b"TLS 1.3, server CertificateVerify\0";
    fn check(self, message: &[u8]) -> Result<()> {
        let valid = match self {
            Self::Certificate => {
                message.first() == Some(&0x30) && message.len() <= Self::MAX_CERTIFICATE
            }
            Self::ServerCertificateVerify => {
                message.len() > 64 && {
                    let (padding, rest) = message.split_at(64);
                    padding.iter().all(|&byte| byte == b' ')
                        && rest
                            .strip_prefix(Self::SERVER_CONTEXT)
                            .is_some_and(|hash| matches!(hash.len(), 32 | 48))
                }
            }
        };
        valid.then_some(()).ok_or(Error::Malformed(
            "not a message of the requested TLS format",
        ))
    }
}

impl NodeIdentity {
    /// Sign a TLS transport message. Anything not in the stated format is
    /// refused, so this cannot produce a signature over YamiNet metadata.
    pub fn sign_tls(&self, format: TlsMessage, message: &[u8]) -> Result<Signature> {
        format.check(message)?;
        Ok(Signature(self.signing_key().sign(message).to_bytes()))
    }
}

impl NodeId {
    /// Strict verification: rejects non-canonical and small-order encodings.
    pub fn verify<T: Signable>(&self, value: &T, signature: &Signature) -> Result<()> {
        self.verify_message(T::PURPOSE, &value.canonical_bytes(), signature)
    }
    fn verify_message(&self, purpose: Purpose, body: &[u8], signature: &Signature) -> Result<()> {
        let signature = ed25519_dalek::Signature::from_bytes(&signature.0);
        self.key()?
            .verify_strict(&message(purpose, body), &signature)
            .map_err(|_| Error::BadSignature)
    }
}

/// A detached signature in a small line-based text format:
///
/// ```text
/// yaminet-signature 1
/// purpose yaminet.site-manifest.v1
/// signer yn1-...
/// signature <128 hex digits>
/// ```
///
/// The signed metadata is not included; the verifier recomputes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureFile {
    pub purpose: Purpose,
    pub signer: NodeId,
    pub signature: Signature,
}
impl SignatureFile {
    pub const HEADER: &'static str = "yaminet-signature 1";
    pub const MAX_LEN: usize = 512;

    pub fn sign<T: Signable>(identity: &NodeIdentity, value: &T) -> Self {
        Self {
            purpose: T::PURPOSE,
            signer: *identity.id(),
            signature: identity.sign(value),
        }
    }
    /// Checks purpose and signer before the signature: a valid signature by
    /// another node is reported as a mismatch, never as success.
    pub fn verify<T: Signable>(&self, value: &T, expected: &NodeId) -> Result<()> {
        if self.purpose != T::PURPOSE {
            return Err(Error::WrongPurpose {
                expected: T::PURPOSE,
                found: self.purpose,
            });
        }
        if self.signer != *expected {
            return Err(Error::Mismatch {
                expected: *expected,
                found: self.signer,
            });
        }
        expected.verify(value, &self.signature)
    }
    /// Exactly four lines, each ending in LF or CRLF, with no other content.
    pub fn parse(text: &str) -> Result<Self> {
        if text.len() > Self::MAX_LEN {
            return Err(Error::Malformed("signature file is too large"));
        }
        let body = text
            .strip_suffix('\n')
            .ok_or(Error::Malformed("signature file must end with a newline"))?;
        let lines: Vec<_> = body
            .split('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line))
            .collect();
        let [header, purpose, signer, signature] = lines[..] else {
            return Err(Error::Malformed(
                "signature file must have exactly four lines",
            ));
        };
        if header != Self::HEADER {
            return Err(Error::Malformed("not a version 1 YamiNet signature file"));
        }
        fn value<'a>(line: &'a str, name: &str) -> Result<&'a str> {
            line.strip_prefix(name)
                .and_then(|rest| rest.strip_prefix(' '))
                .ok_or(Error::Malformed("unexpected field in signature file"))
        }
        let purpose = Purpose::from_name(value(purpose, "purpose")?)
            .ok_or(Error::Malformed("unknown signature purpose"))?;
        Ok(Self {
            purpose,
            signer: value(signer, "signer")?.parse()?,
            signature: value(signature, "signature")?.parse()?,
        })
    }
}
impl fmt::Display for SignatureFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", Self::HEADER)?;
        writeln!(f, "purpose {}", self.purpose.name())?;
        writeln!(f, "signer {}", self.signer)?;
        writeln!(f, "signature {}", self.signature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_distinct_and_prefix_free() {
        for a in Purpose::ALL {
            assert!(a.tag().ends_with(b"\0") && a.tag().is_ascii());
            assert_eq!(a.tag().iter().filter(|&&b| b == 0).count(), 1);
            assert_eq!(Purpose::from_name(a.name()), Some(a));
            for b in Purpose::ALL {
                if a != b {
                    assert!(!b.tag().starts_with(a.tag()));
                }
            }
        }
    }

    #[test]
    fn tls_formats_cannot_overlap_metadata_messages() {
        for purpose in Purpose::ALL {
            assert!(purpose.tag().starts_with(b"yaminet."));
        }
        let identity = NodeIdentity::generate().unwrap();
        let mut verify = vec![b' '; 64];
        verify.extend_from_slice(TlsMessage::SERVER_CONTEXT);
        for hash in [32, 48] {
            let mut message = verify.clone();
            message.resize(verify.len() + hash, 7);
            let signature = identity
                .sign_tls(TlsMessage::ServerCertificateVerify, &message)
                .unwrap();
            let key = identity.id().key().unwrap();
            let signature = ed25519_dalek::Signature::from_bytes(&signature.to_bytes());
            assert!(key.verify_strict(&message, &signature).is_ok());
        }
        let mut client = vec![b' '; 64];
        client.extend_from_slice(b"TLS 1.3, client CertificateVerify\0");
        client.resize(client.len() + 32, 0);
        let mut short = verify.clone();
        short.resize(verify.len() + 20, 0);
        let tagged = message(Purpose::SiteManifest, &[0x30; 40]);
        for bad in [&client, &short, &verify, &tagged, &vec![b' '; 128]] {
            assert!(identity
                .sign_tls(TlsMessage::ServerCertificateVerify, bad)
                .is_err());
            assert!(identity.sign_tls(TlsMessage::Certificate, bad).is_err());
        }
        assert!(identity
            .sign_tls(TlsMessage::Certificate, &[0x30, 0])
            .is_ok());
        assert!(identity
            .sign_tls(TlsMessage::Certificate, &[0x30; 4097])
            .is_err());
    }

    #[test]
    fn signature_for_one_purpose_fails_for_another_with_identical_body() {
        let identity = NodeIdentity::generate().unwrap();
        let body = b"identical canonical bytes";
        let id = identity.id();
        for purpose in Purpose::ALL {
            let signature = identity.sign_message(purpose, body);
            for checked in Purpose::ALL {
                assert_eq!(
                    id.verify_message(checked, body, &signature).is_ok(),
                    checked == purpose
                );
            }
        }
    }

    #[test]
    fn canonical_encodings_are_fixed() {
        let metadata = DocumentMetadata::new("home", [1; 32], [2; 32]).unwrap();
        let mut expected = vec![0, 4];
        expected.extend_from_slice(b"home");
        expected.extend_from_slice(&[1; 32]);
        expected.extend_from_slice(&[2; 32]);
        assert_eq!(metadata.canonical_bytes(), expected);

        let manifest =
            SiteManifest::new([9; 32], [("b".into(), [4; 32]), ("a".into(), [3; 32])]).unwrap();
        let mut expected = vec![9; 32];
        expected.extend_from_slice(&[0, 0, 0, 2, 0, 1, b'a']);
        expected.extend_from_slice(&[3; 32]);
        expected.extend_from_slice(&[0, 1, b'b']);
        expected.extend_from_slice(&[4; 32]);
        assert_eq!(manifest.canonical_bytes(), expected);
    }

    #[test]
    fn manifests_reject_ambiguous_pages() {
        let dup = [("a".to_owned(), [0; 32]), ("a".to_owned(), [1; 32])];
        assert!(SiteManifest::new([0; 32], dup).is_err());
        assert!(SiteManifest::new([0; 32], [("../x".to_owned(), [0; 32])]).is_err());
        assert!(DocumentMetadata::new("", [0; 32], [0; 32]).is_err());
    }
}
