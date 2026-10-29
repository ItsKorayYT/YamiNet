//! Client-side server verification against one pinned node ID.
use crate::credential::{ed25519_key, ed25519_spki_contents};
use rustls::{
    client::{
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        WebPkiServerVerifier,
    },
    crypto::CryptoProvider,
    pki_types::{CertificateDer, ServerName, UnixTime},
    CertificateError, DigitallySignedStruct, Error, OtherError, PeerMisbehaved, RootCertStore,
    SignatureScheme,
};
use std::{fmt, sync::Arc};
use yami_identity::NodeId;

/// The server proved a key, but not the key of the node the client trusts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityMismatch {
    pub expected: NodeId,
    /// `None` when the certificate key is not an Ed25519 node key at all.
    pub found: Option<NodeId>,
}
impl fmt::Display for IdentityMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "server is not the trusted node {}: it ", self.expected)?;
        match &self.found {
            Some(found) => write!(f, "presented node identity {found}"),
            None => write!(f, "presented a key that is not a node identity"),
        }
    }
}
impl std::error::Error for IdentityMismatch {}

/// Accepts a server only if, in this order,
///
/// 1. it sent exactly one certificate, whose key is the trusted node key;
/// 2. that certificate verifies as its own trust anchor: valid self-signature,
///    current validity period, server usage and the node ID as DNS name;
/// 3. the TLS 1.3 handshake signature verifies with that key (Ed25519 only).
#[derive(Debug)]
pub(crate) struct PinnedServer {
    pub(crate) trusted: NodeId,
    pub(crate) provider: Arc<CryptoProvider>,
}
impl ServerCertVerifier for PinnedServer {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        if !intermediates.is_empty() {
            return Err(Error::InvalidCertificate(CertificateError::UnknownIssuer));
        }
        let mut anchor = RootCertStore::empty();
        anchor.add(end_entity.clone().into_owned())?;
        let spki = anchor.roots[0].subject_public_key_info.as_ref();
        if spki != ed25519_spki_contents(&self.trusted.public_key()) {
            let mismatch = IdentityMismatch {
                expected: self.trusted,
                found: ed25519_key(spki).and_then(|key| NodeId::from_public_key(key).ok()),
            };
            return Err(Error::InvalidCertificate(CertificateError::Other(
                OtherError(Arc::new(mismatch)),
            )));
        }
        WebPkiServerVerifier::builder_with_provider(Arc::new(anchor), Arc::clone(&self.provider))
            .build()
            .map_err(|error| Error::General(error.to_string()))?
            .verify_server_cert(end_entity, &[], server_name, ocsp_response, now)
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Err(Error::General("TLS 1.2 is never accepted".into()))
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        if dss.scheme != SignatureScheme::ED25519 {
            return Err(PeerMisbehaved::SignedHandshakeWithUnadvertisedSigScheme.into());
        }
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}
