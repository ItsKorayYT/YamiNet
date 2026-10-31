use crate::{path_valid, Error, Name, Result};
use serde::{Deserialize, Serialize};
use yami_identity::{sha256, NamingRecordDigest, NodeId, NodeIdentity, Signature};
pub const MAX_RECORD_BYTES: usize = 4096;
/// One exact mapping, child delegation, or signed tombstone. Delegations grant
/// authority only below their path; they never grant authority over siblings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "lowercase",
    deny_unknown_fields
)]
pub enum RecordKind {
    Map(String),
    Delegate(String),
    Revoke,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    version: u8,
    namespace: String,
    path: String,
    signer: String,
    sequence: u64,
    issued: u64,
    expires: u64,
    action: RecordKind,
    signature: String,
}
impl Record {
    #[allow(clippy::too_many_arguments)]
    pub fn sign(
        owner: &NodeIdentity,
        namespace: &str,
        path: &str,
        sequence: u64,
        issued: u64,
        expires: u64,
        action: RecordKind,
    ) -> Result<Self> {
        let mut record = Self {
            version: 1,
            namespace: namespace.into(),
            path: path.into(),
            signer: owner.id().to_string(),
            sequence,
            issued,
            expires,
            action,
            signature: String::new(),
        };
        record.validate()?;
        record.signature = owner.sign(&NamingRecordDigest(record.digest())).to_string();
        Ok(record)
    }
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(Error::Invalid("record too large"));
        }
        let r: Self = serde_json::from_slice(bytes).map_err(|_| Error::Invalid("record JSON"))?;
        r.validate()?;
        r.verify_signature()?;
        Ok(r)
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("record serialization")
    }
    pub fn namespace(&self) -> &str {
        &self.namespace
    }
    pub fn path(&self) -> &str {
        &self.path
    }
    pub fn signer(&self) -> Result<NodeId> {
        self.signer.parse().map_err(|_| Error::Signature)
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn action(&self) -> &RecordKind {
        &self.action
    }
    pub fn target(&self) -> Result<NodeId> {
        match &self.action {
            RecordKind::Map(id) | RecordKind::Delegate(id) => id
                .parse()
                .map_err(|_| Error::Invalid("full target identity required")),
            RecordKind::Revoke => Err(Error::NotFound),
        }
    }
    pub fn verify(&self, now: u64) -> Result<()> {
        self.validate()?;
        self.verify_signature()?;
        if now < self.issued || now >= self.expires {
            return Err(Error::Expired);
        }
        Ok(())
    }
    fn validate(&self) -> Result<()> {
        if self.version != 1
            || self.sequence == 0
            || self.issued == 0
            || self.issued >= self.expires
        {
            return Err(Error::Invalid(
                "unsupported version or invalid sequence/time interval",
            ));
        }
        path_valid(&self.path)?;
        if self.namespace == "yami" && self.path.is_empty() {
            return Err(Error::Invalid("empty official name"));
        }
        if self.namespace != "yami" {
            match Name::parse(&self.namespace)? {
                Name::Scoped { path, .. } if path.is_empty() => {}
                _ => return Err(Error::Invalid("invalid namespace")),
            }
        }
        self.signer()?;
        if !matches!(self.action, RecordKind::Revoke) {
            self.target()?;
        }
        if self.path.is_empty() && matches!(self.action, RecordKind::Delegate(_)) {
            return Err(Error::Invalid("root delegation is not supported in v1"));
        }
        Ok(())
    }
    fn verify_signature(&self) -> Result<()> {
        let sig: Signature = self.signature.parse().map_err(|_| Error::Signature)?;
        self.signer()?
            .verify(&NamingRecordDigest(self.digest()), &sig)
            .map_err(|_| Error::Signature)
    }
    /// Fixed order, big-endian integers and length-prefixed UTF-8 fields. JSON
    /// is only an envelope; its key order/whitespace never changes a signature.
    pub fn digest(&self) -> [u8; 32] {
        let mut b = vec![self.version];
        for field in [&self.namespace, &self.path, &self.signer] {
            put(&mut b, field);
        }
        for n in [self.sequence, self.issued, self.expires] {
            b.extend_from_slice(&n.to_be_bytes());
        }
        match &self.action {
            RecordKind::Map(id) => {
                b.push(1);
                put(&mut b, id);
            }
            RecordKind::Delegate(id) => {
                b.push(2);
                put(&mut b, id);
            }
            RecordKind::Revoke => b.push(3),
        }
        sha256(&b)
    }
}
fn put(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_payload_matches_independent_v1_vector() {
        // Ed25519 basepoint encoding; public test fixture, no registry authority.
        let id = "yn1-lbtgmztgmztgmztgmztgmztgmztgmztgmztgmztgmztgmztgmztfk4s3km";
        let r = Record {
            version: 1,
            namespace: format!("{id}.yns"),
            path: "blog".into(),
            signer: id.into(),
            sequence: 1,
            issued: 100,
            expires: 1000,
            action: RecordKind::Map(id.into()),
            signature: String::new(),
        };
        r.validate().unwrap();
        let hex: String = r.digest().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "181724b79f0aeebb5b204781f32adc4e3d6f84a31faaf5406fa71cd0f7c83c42"
        );
    }
    #[test]
    fn record_signature_cannot_authenticate_other_identity_domains() {
        use yami_identity::{DocumentMetadata, Signable};
        let owner = NodeIdentity::generate().unwrap();
        let r = Record::sign(
            &owner,
            &format!("{}.yns", owner.id()),
            "blog",
            1,
            100,
            1000,
            RecordKind::Map(owner.id().to_string()),
        )
        .unwrap();
        let signature: Signature = r.signature.parse().unwrap();
        let document = DocumentMetadata::new("blog", r.digest(), r.digest()).unwrap();
        assert!(owner.id().verify(&document, &signature).is_err());
        assert_eq!(
            NamingRecordDigest::PURPOSE.tag(),
            b"yaminet.yns-record.v1\0"
        );
    }
    #[test]
    fn json_envelope_cannot_smuggle_extra_action_fields() {
        let owner = NodeIdentity::generate().unwrap();
        let r = Record::sign(
            &owner,
            &format!("{}.yns", owner.id()),
            "blog",
            1,
            100,
            1000,
            RecordKind::Revoke,
        )
        .unwrap();
        let mut wire: serde_json::Value = serde_json::from_slice(&r.to_bytes()).unwrap();
        wire["action"]["value"] = serde_json::json!(owner.id().to_string());
        assert!(Record::parse(&serde_json::to_vec(&wire).unwrap()).is_err());
    }
}
