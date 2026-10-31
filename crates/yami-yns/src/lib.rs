//! YNS 0.1: permissionless key-scoped names, explicit local aliases, and a
//! separately configured official registry. Resolution returns a full NodeId;
//! the transport must still authenticate that exact identity. See docs/yns.md.
mod record;
mod store;
mod trust;

pub use record::{Record, RecordKind, MAX_RECORD_BYTES};
use std::{fmt, io};
pub use store::Store;
pub use trust::{resolve, History, RecordSource};
use yami_identity::NodeId;

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug)]
pub enum Error {
    Invalid(&'static str),
    Signature,
    Expired,
    Rollback,
    NotFound,
    UntrustedRegistry,
    IdentityChanged {
        name: String,
        previous: NodeId,
        proposed: NodeId,
    },
    Io(io::Error),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(why) => write!(f, "Invalid YNS data: {why}"),
            Self::Signature => f.write_str("YNS signature or namespace ownership verification failed"),
            Self::Expired => f.write_str("YNS record is expired or not yet valid"),
            Self::Rollback => f.write_str("YNS rollback or conflicting sequence rejected"),
            Self::NotFound => f.write_str("YNS name or delegation not found"),
            Self::UntrustedRegistry => f.write_str("No matching configured trusted .yami registry key"),
            Self::IdentityChanged { name, previous, proposed } => write!(f, "YNS identity change blocked for {name}: trusted {previous}; proposed {proposed}. Review and explicitly approve both identities before continuing."),
            Self::Io(e) => write!(f, "YNS state unavailable: {e}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Input is already canonical or rejected: no case folding, Unicode, IDNA,
/// percent decoding, whitespace removal, empty labels or trailing dots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Name {
    Alias(String),
    Scoped { owner: NodeId, path: String },
    Official(String),
}
impl Name {
    pub fn parse(text: &str) -> Result<Self> {
        if let Some(alias) = text.strip_prefix('@') {
            label(alias)?;
            return Ok(Self::Alias(alias.into()));
        }
        if let Some(body) = text.strip_suffix(".yns") {
            let (path, key) = body.rsplit_once('.').unwrap_or(("", body));
            path_valid(path)?;
            let owner = key
                .parse()
                .map_err(|_| Error::Invalid("use a full namespace owner ID"))?;
            return Ok(Self::Scoped {
                owner,
                path: path.into(),
            });
        }
        if let Some(path) = text.strip_suffix(".yami") {
            if path.is_empty() {
                return Err(Error::Invalid("empty official name"));
            }
            path_valid(path)?;
            return Ok(Self::Official(path.into()));
        }
        Err(Error::Invalid(
            "use @alias, label.full-owner-ID.yns or service.yami",
        ))
    }
    pub fn namespace(&self) -> Option<String> {
        match self {
            Self::Scoped { owner, .. } => Some(format!("{owner}.yns")),
            Self::Official(_) => Some("yami".into()),
            Self::Alias(_) => None,
        }
    }
}
impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Alias(a) => write!(f, "@{a}"),
            Self::Scoped { owner, path } if path.is_empty() => write!(f, "{owner}.yns"),
            Self::Scoped { owner, path } => write!(f, "{path}.{owner}.yns"),
            Self::Official(p) => write!(f, "{p}.yami"),
        }
    }
}
pub(crate) fn label(s: &str) -> Result<()> {
    if s.is_empty()
        || s.len() > 32
        || !s.as_bytes()[0].is_ascii_lowercase()
        || !s.as_bytes()[s.len() - 1].is_ascii_alphanumeric()
        || s.starts_with("xn--")
        || !s
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(Error::Invalid("labels require 1..32 lowercase ASCII letters/digits/hyphens, starting with a letter; IDNA is forbidden"));
    }
    Ok(())
}
pub(crate) fn path_valid(s: &str) -> Result<()> {
    if s.is_empty() {
        return Ok(());
    }
    if s.split('.').count() > 8 {
        return Err(Error::Invalid("at most eight name labels"));
    }
    for part in s.split('.') {
        label(part)?;
    }
    Ok(())
}
