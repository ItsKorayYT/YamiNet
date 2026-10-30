//! Canonical identity addresses used by directory entries.
use crate::{Error, Result, MAX_ADDRESS_BYTES};
use std::{fmt, str::FromStr};
use yami_identity::NodeId;

/// A canonical `yami://<full-node-id>/<document-key>` address.
///
/// Parsing is strict about the identity: the complete lowercase `yn1-` node ID
/// with a valid checksum is required. The document key follows the existing
/// YNP key rules. An omitted or empty key is normalized to `home`, matching the
/// native browser's address handling. Display always emits the canonical form,
/// which is also the deduplication key for [`crate::Catalog`].
///
/// This type names a destination. It does **not** prove that the destination
/// exists, controls the page or should be trusted.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WebsiteAddress {
    node: NodeId,
    key: String,
}

impl WebsiteAddress {
    /// Parses and canonicalizes a `yami://` address.
    pub fn parse(text: &str) -> Result<Self> {
        if text.len() > MAX_ADDRESS_BYTES {
            return Err(Error::invalid(format!(
                "address exceeds {MAX_ADDRESS_BYTES} bytes"
            )));
        }
        let rest = text
            .strip_prefix("yami://")
            .ok_or_else(|| Error::invalid("address must start with yami://"))?;
        if rest.is_empty() {
            return Err(Error::invalid("address has no node ID after yami://"));
        }
        let (node, key) = rest.split_once('/').unwrap_or((rest, "home"));
        let key = if key.is_empty() { "home" } else { key };
        let node = node.parse::<NodeId>().map_err(|_| {
            Error::invalid(
                "node ID must be the complete lowercase yn1- identity, including its checksum",
            )
        })?;
        if !ylang::valid_key(key) {
            return Err(Error::invalid(
                "document key must be 1..128 ASCII letters, digits, underscores or hyphens",
            ));
        }
        Ok(Self {
            node,
            key: key.into(),
        })
    }

    /// The exact node identity named by this address.
    pub fn node_id(&self) -> NodeId {
        self.node
    }

    /// The YNP document key (for example `home`).
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The canonical URL.
    pub fn url(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for WebsiteAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "yami://{}/{}", self.node, self.key)
    }
}

impl FromStr for WebsiteAddress {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        Self::parse(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_id() -> &'static str {
        "yn1-sp2hcdqey6gfx62bqb3vzymxh4hjsjv2ocfqk4hecwplc77pkh2yatmgrq"
    }

    #[test]
    fn canonicalizes_missing_and_empty_keys() {
        for text in [
            format!("yami://{}/home", valid_id()),
            format!("yami://{}/", valid_id()),
            format!("yami://{}", valid_id()),
        ] {
            let address = WebsiteAddress::parse(&text).unwrap();
            assert_eq!(address.to_string(), format!("yami://{}/home", valid_id()));
            assert_eq!(address.key(), "home");
        }
    }

    #[test]
    fn preserves_document_keys_and_identity() {
        let address = WebsiteAddress::parse(&format!("yami://{}/About-2", valid_id())).unwrap();
        assert_eq!(address.key(), "About-2");
        assert_eq!(address.url(), format!("yami://{}/About-2", valid_id()));
        assert_eq!(address.node_id().to_string(), valid_id());
    }

    #[test]
    fn rejects_wrong_scheme_bad_checksum_and_extra_path() {
        for text in [
            "https://example.com/".to_string(),
            "yami:///home".to_string(),
            format!("yami://{}", valid_id().replace('a', "b")),
            format!("yami://{}/home/extra", valid_id()),
            format!("yami://{}/keys?query", valid_id()),
            format!("yami://{}/", valid_id()).replace("//", "// "),
            format!("YAMI://{}/home", valid_id()),
            format!("yami://{}/home\n", valid_id()),
        ] {
            assert!(WebsiteAddress::parse(&text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn rejects_oversized_addresses() {
        let text = format!("yami://{}/{}", valid_id(), "a".repeat(MAX_ADDRESS_BYTES));
        assert!(WebsiteAddress::parse(&text).is_err());
    }
}
