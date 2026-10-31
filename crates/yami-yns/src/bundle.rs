use crate::{
    store::record_name, trust::authenticate, Error, History, Name, Record, RecordSource, Result,
};
use serde::{Deserialize, Serialize};
use yami_identity::NodeId;
/// An exact root-to-leaf chain or an authenticated prefix (e.g. a revoked
/// delegation). The signed v1 records are carried byte-for-byte semantically;
/// the serving node contributes no naming authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub name: String,
    pub records: Vec<Record>,
}
impl Bundle {
    pub fn verify(&self, registry: &[NodeId], now: u64) -> Result<()> {
        let name = Name::parse(&self.name)?;
        let namespace = name
            .namespace()
            .ok_or(Error::Invalid("aliases cannot be published"))?;
        let paths = paths(&name)?;
        if self.records.is_empty() || self.records.len() > paths.len() {
            return Err(Error::Invalid("invalid chain length"));
        }
        for (record, path) in self.records.iter().zip(paths) {
            if record.namespace() != namespace
                || record.path() != path
                || record.to_bytes().len() > crate::MAX_RECORD_BYTES
            {
                return Err(Error::Invalid(
                    "chain selector mismatch or oversized record",
                ));
            }
        }
        let last = self.records.last().ok_or(Error::NotFound)?;
        authenticate(
            self,
            &record_name(last)?,
            registry,
            &mut History::default(),
            now,
        )?;
        Ok(())
    }
}
impl RecordSource for Bundle {
    fn lookup(&self, namespace: &str, path: &str) -> Result<Option<Record>> {
        Ok(self
            .records
            .iter()
            .find(|r| r.namespace() == namespace && r.path() == path)
            .cloned())
    }
}
pub(crate) fn paths(name: &Name) -> Result<Vec<String>> {
    let path = match name {
        Name::Scoped { path, .. } | Name::Official(path) => path,
        Name::Alias(_) => return Err(Error::Invalid("aliases are local")),
    };
    if path.is_empty() {
        return Ok(vec![String::new()]);
    }
    let parts: Vec<_> = path.split('.').collect();
    Ok((0..parts.len())
        .rev()
        .map(|i| parts[i..].join("."))
        .collect())
}
