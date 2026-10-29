//! Node IDs: `yn1-` followed by the unpadded lowercase RFC 4648 base32 encoding
//! of the 32-byte Ed25519 public key and a 4-byte checksum. The ID contains the
//! whole key, so a signature can be checked against an ID without any lookup.
use crate::{hex, Error, Result};
use ed25519_dalek::VerifyingKey;
use sha2::{Digest as _, Sha256};
use std::{fmt, str::FromStr};

/// The `1` is the node ID format version; another version gets another prefix.
pub const NODE_ID_PREFIX: &str = "yn1-";
const CHECKSUM_TAG: &[u8] = b"yaminet.node-id.v1\0";
const CHECKSUM: usize = 4;
const BODY: usize = 32 + CHECKSUM;
const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// Holds the public key bytes, which were checked to be a valid, non-weak
/// Ed25519 point when the ID was created or parsed.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId([u8; 32]);
impl NodeId {
    pub(crate) fn new(key: VerifyingKey) -> Self {
        Self(key.to_bytes())
    }
    pub(crate) fn key(&self) -> Result<VerifyingKey> {
        VerifyingKey::from_bytes(&self.0).map_err(|_| Error::BadSignature)
    }
    /// The node ID of a raw Ed25519 public key, with the same checks as parsing.
    pub fn from_public_key(key: [u8; 32]) -> Result<Self> {
        let point = VerifyingKey::from_bytes(&key)
            .map_err(|_| Error::InvalidNodeId("not a valid Ed25519 public key"))?;
        if point.is_weak() {
            return Err(Error::InvalidNodeId("small-order Ed25519 public key"));
        }
        Ok(Self(key))
    }
    pub fn public_key(&self) -> [u8; 32] {
        self.0
    }
    pub fn public_key_hex(&self) -> String {
        hex(&self.0)
    }
}
/// Detects typing and copying mistakes; it is not a security mechanism.
fn checksum(key: &[u8; 32]) -> [u8; CHECKSUM] {
    let digest = Sha256::new()
        .chain_update(CHECKSUM_TAG)
        .chain_update(key)
        .finalize();
    let mut sum = [0; CHECKSUM];
    sum.copy_from_slice(&digest[..CHECKSUM]);
    sum
}
impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let key = &self.0;
        let mut body = [0; BODY];
        body[..32].copy_from_slice(key);
        body[32..].copy_from_slice(&checksum(key));
        write!(f, "{NODE_ID_PREFIX}{}", encode(&body))
    }
}
impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NodeId({self})")
    }
}
impl FromStr for NodeId {
    type Err = Error;
    fn from_str(text: &str) -> Result<Self> {
        let encoded = text
            .strip_prefix(NODE_ID_PREFIX)
            .ok_or(Error::InvalidNodeId("must start with yn1-"))?;
        let mut body = [0; BODY];
        decode(encoded, &mut body).ok_or(Error::InvalidNodeId(
            "expected 58 lowercase base32 characters (a-z, 2-7)",
        ))?;
        let mut key = [0; 32];
        key.copy_from_slice(&body[..32]);
        if body[32..] != checksum(&key) {
            return Err(Error::InvalidNodeId("checksum mismatch; check for typos"));
        }
        Self::from_public_key(key)
    }
}

fn encode(bytes: &[u8]) -> String {
    let mut text = String::with_capacity((bytes.len() * 8).div_ceil(5));
    let (mut buffer, mut bits) = (0u32, 0);
    for &byte in bytes {
        buffer = buffer << 8 | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            text.push(ALPHABET[(buffer >> bits) as usize & 31] as char);
        }
    }
    if bits > 0 {
        text.push(ALPHABET[(buffer << (5 - bits)) as usize & 31] as char);
    }
    text
}
/// Accepts only the exact text `encode` produces for `output.len()` bytes, so
/// every node ID has a single spelling and IDs compare as strings.
fn decode(text: &str, output: &mut [u8]) -> Option<()> {
    if text.len() != (output.len() * 8).div_ceil(5) {
        return None;
    }
    let (mut buffer, mut bits, mut written) = (0u32, 0, 0);
    for c in text.bytes() {
        let value = ALPHABET.iter().position(|&a| a == c)? as u32;
        buffer = buffer << 5 | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            output[written] = (buffer >> bits) as u8;
            written += 1;
        }
    }
    // Leftover padding bits must be zero.
    (buffer & ((1 << bits) - 1) == 0).then_some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base32_matches_rfc4648_vectors() {
        // RFC 4648 section 10, lowercased and without padding.
        for (input, expected) in [
            ("", ""),
            ("f", "my"),
            ("fo", "mzxq"),
            ("foo", "mzxw6"),
            ("foob", "mzxw6yq"),
            ("fooba", "mzxw6ytb"),
            ("foobar", "mzxw6ytboi"),
        ] {
            assert_eq!(encode(input.as_bytes()), expected);
            let mut output = vec![0; input.len()];
            assert_eq!(decode(expected, &mut output), Some(()));
            assert_eq!(output, input.as_bytes());
        }
    }

    #[test]
    fn base32_rejects_noncanonical_text() {
        let mut output = [0; 1];
        // "mz" decodes to "f" but sets a padding bit.
        assert_eq!(decode("mz", &mut output), None);
        assert_eq!(decode("MY", &mut output), None);
        assert_eq!(decode("m1", &mut output), None);
        assert_eq!(decode("my2", &mut output), None);
    }
}
