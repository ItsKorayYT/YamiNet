//! Experimental persistent node identities: an Ed25519 key pair per node, a
//! versioned node ID derived from the public key, and domain-separated
//! signatures over canonical document and site metadata.
//!
//! A valid signature shows that the holder of a node's secret key signed that
//! exact metadata. It does not encrypt, hide or anonymize anything, does not
//! authenticate a YNP connection, and says nothing about freshness: an old
//! signed version stays valid. See `docs/specs/v0.1/identity.md`.
//!
//! The same key authenticates the node in the TLS transport (`ynp-tls`),
//! which signs only TLS-formatted messages; see [`TlsMessage`].
mod node_id;
mod signed;
mod store;

use std::{fmt, io, path::PathBuf};

pub use node_id::{NodeId, NODE_ID_PREFIX};
pub use signed::{
    sha256, Digest, DocumentMetadata, PeerRecordMetadata, Purpose, RelayRegistration, RelayRoute,
    RendezvousConsent, RoutedPeerRecordMetadata, Signable, Signature, SignatureFile, SiteManifest,
    TlsMessage,
};
pub use store::{default_dir, NodeIdentity, ID_FILE, KEY_FILE};

pub type Result<T> = std::result::Result<T, Error>;

/// Messages never contain secret key material; node IDs are public.
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// The secret key file is truncated, malformed or internally inconsistent.
    CorruptKey(&'static str),
    InvalidNodeId(&'static str),
    /// A signature file or metadata value is not in its canonical form.
    Malformed(&'static str),
    /// The signature does not verify for this signer, purpose and content.
    BadSignature,
    WrongPurpose {
        expected: Purpose,
        found: Purpose,
    },
    /// Two sources disagree about which node this is.
    Mismatch {
        expected: NodeId,
        found: NodeId,
    },
    /// Group or other users can access the secret key file (Unix only).
    InsecurePermissions(PathBuf),
    AlreadyExists(PathBuf),
    /// No secret key file. When a node ID file exists the identity is lost
    /// rather than new, so a replacement is never generated silently.
    MissingKey(PathBuf),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::CorruptKey(reason) => write!(f, "corrupt node key file: {reason}"),
            Self::InvalidNodeId(reason) => write!(f, "invalid node ID: {reason}"),
            Self::Malformed(reason) => write!(f, "malformed signed data: {reason}"),
            Self::BadSignature => write!(f, "signature is not valid for this signer and content"),
            Self::WrongPurpose { expected, found } => write!(
                f,
                "signature is for {} but {} was expected",
                found.name(),
                expected.name()
            ),
            Self::Mismatch { expected, found } => {
                write!(f, "identity mismatch: expected {expected}, found {found}")
            }
            Self::InsecurePermissions(path) => write!(
                f,
                "{} is accessible by other users; restrict it to its owner (chmod 600)",
                path.display()
            ),
            Self::AlreadyExists(path) => write!(
                f,
                "{} already exists; refusing to replace a node identity",
                path.display()
            ),
            Self::MissingKey(path) => write!(f, "node secret key {} is missing", path.display()),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}
impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
/// Strict lowercase hexadecimal of exactly `output.len()` bytes.
fn unhex(text: &str, output: &mut [u8]) -> Option<()> {
    if text.len() != output.len() * 2 {
        return None;
    }
    let digit = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    for (byte, pair) in output.iter_mut().zip(text.as_bytes().chunks(2)) {
        *byte = digit(pair[0])? << 4 | digit(pair[1])?;
    }
    Some(())
}
