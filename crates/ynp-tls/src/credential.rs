//! Node credentials: an X.509 certificate for the node's own Ed25519 key,
//! self-signed by that key and valid for a limited time. The node key signs
//! the certificate and each handshake through `NodeIdentity::sign_tls`, so the
//! secret key never leaves `yami-identity`.
use crate::{other, CREDENTIAL_LIFETIME};
use rcgen::{
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, KeyUsagePurpose,
    PublicKeyData, SerialNumber, SignatureAlgorithm, PKCS_ED25519,
};
use rustls::{
    pki_types::{CertificateDer, SubjectPublicKeyInfoDer},
    server::{ClientHello, ResolvesServerCert},
    sign::{CertifiedKey, Signer, SigningKey},
    SignatureScheme,
};
use std::{
    io,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use time::OffsetDateTime;
use yami_identity::{sha256, NodeId, NodeIdentity, TlsMessage};

/// How far back a new certificate's validity starts, so a client whose clock
/// is slightly behind the node's still accepts it.
pub const BACKDATE: Duration = Duration::from_secs(5 * 60);

/// DER `SubjectPublicKeyInfo` of an Ed25519 key (RFC 8410): a fixed 12-byte
/// header followed by the 32 key bytes.
const ED25519_SPKI_HEADER: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];
pub(crate) fn ed25519_spki(key: &[u8; 32]) -> Vec<u8> {
    [&ED25519_SPKI_HEADER[..], key].concat()
}
/// The same, without the outer SEQUENCE tag and length, as webpki stores it.
pub(crate) fn ed25519_spki_contents(key: &[u8; 32]) -> Vec<u8> {
    ed25519_spki(key)[2..].to_vec()
}
/// The Ed25519 key inside SPKI contents, if that is what they hold.
pub(crate) fn ed25519_key(spki_contents: &[u8]) -> Option<[u8; 32]> {
    spki_contents
        .strip_prefix(&ED25519_SPKI_HEADER[2..])?
        .try_into()
        .ok()
}

/// The node key, as certificate signer for rcgen and handshake signer for rustls.
#[derive(Debug, Clone)]
struct NodeKey {
    identity: Arc<NodeIdentity>,
    public: [u8; 32],
}
impl NodeKey {
    fn new(identity: Arc<NodeIdentity>) -> Self {
        let public = identity.id().public_key();
        Self { identity, public }
    }
}
impl PublicKeyData for NodeKey {
    fn der_bytes(&self) -> &[u8] {
        &self.public
    }
    fn algorithm(&self) -> &'static SignatureAlgorithm {
        &PKCS_ED25519
    }
}
impl rcgen::SigningKey for NodeKey {
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, rcgen::Error> {
        self.identity
            .sign_tls(TlsMessage::Certificate, message)
            .map(|signature| signature.to_bytes().to_vec())
            .map_err(|_| rcgen::Error::RemoteKeyError)
    }
}
impl SigningKey for NodeKey {
    fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn Signer>> {
        offered
            .contains(&SignatureScheme::ED25519)
            .then(|| Box::new(self.clone()) as Box<dyn Signer>)
    }
    fn public_key(&self) -> Option<SubjectPublicKeyInfoDer<'_>> {
        Some(ed25519_spki(&self.public).into())
    }
    fn algorithm(&self) -> rustls::SignatureAlgorithm {
        rustls::SignatureAlgorithm::ED25519
    }
}
impl Signer for NodeKey {
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, rustls::Error> {
        self.identity
            .sign_tls(TlsMessage::ServerCertificateVerify, message)
            .map(|signature| signature.to_bytes().to_vec())
            .map_err(|error| rustls::Error::General(error.to_string()))
    }
    fn scheme(&self) -> SignatureScheme {
        SignatureScheme::ED25519
    }
}

/// A node's TLS certificate together with the key that proves it.
#[derive(Debug, Clone)]
pub struct Credential {
    key: NodeKey,
    certificate: CertificateDer<'static>,
}
impl Credential {
    /// A certificate valid from `not_before` to `not_after` (whole seconds),
    /// naming the node ID as its subject and only DNS name.
    pub fn issue(
        identity: Arc<NodeIdentity>,
        not_before: SystemTime,
        not_after: SystemTime,
    ) -> io::Result<Self> {
        if not_after <= not_before {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "credential must end after it starts",
            ));
        }
        let key = NodeKey::new(identity);
        let id = key.identity.id().to_string();
        let mut params = CertificateParams::new([id.clone()]).map_err(other)?;
        params.distinguished_name = DistinguishedName::new();
        params.distinguished_name.push(DnType::CommonName, id);
        params.not_before = OffsetDateTime::from(not_before);
        params.not_after = OffsetDateTime::from(not_after);
        params.serial_number = Some(serial(&key.public, not_before));
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let certificate = params.self_signed(&key).map_err(other)?.der().clone();
        Ok(Self { key, certificate })
    }
    /// A certificate valid from `BACKDATE` ago for `CREDENTIAL_LIFETIME`.
    pub fn issue_now(identity: Arc<NodeIdentity>) -> io::Result<Self> {
        let now = SystemTime::now();
        Self::issue(identity, now - BACKDATE, now + CREDENTIAL_LIFETIME)
    }
    /// Present arbitrary certificate bytes with this node's handshake key.
    /// Only for testing that clients reject such certificates.
    #[doc(hidden)]
    pub fn with_certificate(
        identity: Arc<NodeIdentity>,
        certificate: CertificateDer<'static>,
    ) -> Self {
        Self {
            key: NodeKey::new(identity),
            certificate,
        }
    }
    pub fn certificate(&self) -> &CertificateDer<'static> {
        &self.certificate
    }
    pub fn node_id(&self) -> &NodeId {
        self.key.identity.id()
    }
    fn certified_key(&self) -> Arc<CertifiedKey> {
        Arc::new(CertifiedKey::new(
            vec![self.certificate.clone()],
            Arc::new(self.key.clone()),
        ))
    }
}

/// Positive, at most 16 bytes, distinct for every issue time.
fn serial(key: &[u8; 32], not_before: SystemTime) -> SerialNumber {
    let nanos = not_before
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut bytes = sha256(&[&key[..], &nanos.to_be_bytes()].concat())[..16].to_vec();
    bytes[0] = bytes[0] & 0x7f | 0x40;
    SerialNumber::from_slice(&bytes)
}

/// Hands rustls the node's current certificate. A listener bound to an
/// identity issues a fresh certificate once half the lifetime has passed; a
/// listener given a fixed credential presents exactly that one.
#[derive(Debug)]
pub(crate) struct Resolver {
    renew: Option<Arc<NodeIdentity>>,
    current: Mutex<(SystemTime, Arc<CertifiedKey>)>,
}
impl Resolver {
    pub(crate) fn renewing(identity: Arc<NodeIdentity>) -> io::Result<Self> {
        let credential = Credential::issue_now(Arc::clone(&identity))?;
        Ok(Self {
            renew: Some(identity),
            current: Mutex::new((renew_at(), credential.certified_key())),
        })
    }
    pub(crate) fn fixed(credential: &Credential) -> Self {
        Self {
            renew: None,
            current: Mutex::new((SystemTime::now(), credential.certified_key())),
        }
    }
    fn current(&self) -> Arc<CertifiedKey> {
        let mut current = self.current.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(identity) = &self.renew {
            if SystemTime::now() >= current.0 {
                // On failure keep presenting the current certificate; clients
                // reject it themselves once it expires.
                if let Ok(credential) = Credential::issue_now(Arc::clone(identity)) {
                    *current = (renew_at(), credential.certified_key());
                }
            }
        }
        Arc::clone(&current.1)
    }
}
fn renew_at() -> SystemTime {
    SystemTime::now() + CREDENTIAL_LIFETIME / 2
}
impl ResolvesServerCert for Resolver {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        // Node keys sign only Ed25519; without it there is nothing to offer.
        hello
            .signature_schemes()
            .contains(&SignatureScheme::ED25519)
            .then(|| self.current())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::RootCertStore;

    #[test]
    fn certificates_hold_the_node_key_and_renew_halfway() {
        let identity = Arc::new(NodeIdentity::generate().unwrap());
        let key = identity.id().public_key();
        let credential = Credential::issue_now(Arc::clone(&identity)).unwrap();
        let mut anchor = RootCertStore::empty();
        anchor.add(credential.certificate().clone()).unwrap();
        let spki = anchor.roots[0].subject_public_key_info.as_ref();
        assert_eq!(spki, ed25519_spki_contents(&key));
        assert_eq!(ed25519_key(spki), Some(key));
        assert_eq!(ed25519_key(&spki[1..]), None);

        let resolver = Resolver::renewing(Arc::clone(&identity)).unwrap();
        let first = resolver.current();
        assert!(Arc::ptr_eq(&first, &resolver.current()), "not yet due");
        resolver.current.lock().unwrap().0 = UNIX_EPOCH;
        let renewed = resolver.current();
        assert_ne!(first.cert, renewed.cert);
        assert!(renewed.keys_match().is_ok());

        let fixed = Resolver::fixed(&credential);
        fixed.current.lock().unwrap().0 = UNIX_EPOCH;
        assert_eq!(fixed.current().cert[0], *credential.certificate());
    }
}
