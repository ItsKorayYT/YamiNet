use crate::{Error, Name, Record, RecordKind, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use yami_identity::NodeId;

/// Sources are untrusted. Implementations may use files, explicit peer queries,
/// or future DHT distribution; none may assign ownership or change TLS pins.
pub trait RecordSource {
    fn lookup(&self, namespace: &str, path: &str) -> Result<Option<Record>>;
}
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct History {
    high_water: BTreeMap<String, (u64, [u8; 32])>,
    bindings: BTreeMap<String, String>,
    #[serde(default)]
    conflicts: BTreeMap<String, u64>,
}
impl History {
    pub(crate) fn observe(&mut self, record: &Record) -> Result<()> {
        let key = format!("{}/{}", record.namespace(), record.path());
        let next = (record.sequence(), record.digest());
        if self
            .conflicts
            .get(&key)
            .is_some_and(|sequence| next.0 <= *sequence)
        {
            return Err(Error::Rollback);
        }
        if let Some(old) = self.high_water.get(&key) {
            if next.0 < old.0 {
                return Err(Error::Rollback);
            }
            if next.0 == old.0 && next.1 != old.1 {
                self.conflicts.insert(key, next.0);
                return Err(Error::Rollback);
            }
        }
        self.conflicts.remove(&key);
        self.high_water.insert(key, next);
        Ok(())
    }
    pub(crate) fn conflict(&mut self, namespace: &str, path: &str, sequence: u64) {
        self.conflicts
            .entry(format!("{namespace}/{path}"))
            .and_modify(|old| *old = (*old).max(sequence))
            .or_insert(sequence);
    }
    pub(crate) fn pin(&mut self, name: &Name, id: NodeId) -> Result<()> {
        let name = name.to_string();
        if let Some(old) = self.bindings.get(&name) {
            let previous = old
                .parse()
                .map_err(|_| Error::Invalid("corrupt trusted binding"))?;
            if previous != id {
                return Err(Error::IdentityChanged {
                    name,
                    previous,
                    proposed: id,
                });
            }
        }
        self.bindings.insert(name, id.to_string());
        Ok(())
    }
    pub(crate) fn binding(&self, name: &Name) -> Option<&str> {
        self.bindings.get(&name.to_string()).map(String::as_str)
    }
    pub(crate) fn forget_binding(&mut self, name: &Name) {
        self.bindings.remove(&name.to_string());
    }
}
/// Resolve an exact chain of delegations and remember authenticated sequences.
/// Persist history even when resolution reports a changed destination or a
/// signed tombstone. No fallback to other naming systems is permitted.
pub fn resolve(
    source: &dyn RecordSource,
    name: &Name,
    trusted_registry: &[NodeId],
    history: &mut History,
    now: u64,
) -> Result<NodeId> {
    let record = authenticate(source, name, trusted_registry, history, now)?;
    if !matches!(record.action(), RecordKind::Map(_)) {
        return Err(Error::NotFound);
    }
    let id = record.target()?;
    history.pin(name, id)?;
    Ok(id)
}

pub(crate) fn authenticate(
    source: &dyn RecordSource,
    name: &Name,
    trusted_registry: &[NodeId],
    history: &mut History,
    now: u64,
) -> Result<Record> {
    if Name::parse(&name.to_string())? != *name {
        return Err(Error::Invalid("noncanonical name"));
    }
    let (namespace, path, mut authority) = match name {
        Name::Scoped { owner, path } => (format!("{owner}.yns"), path.as_str(), Some(*owner)),
        Name::Official(path) => {
            if trusted_registry.is_empty() {
                return Err(Error::UntrustedRegistry);
            }
            ("yami".into(), path.as_str(), None)
        }
        Name::Alias(_) => return Err(Error::Invalid("local aliases require a local store")),
    };
    let parts: Vec<_> = if path.is_empty() {
        Vec::new()
    } else {
        path.split('.').collect()
    };
    let paths: Vec<String> = if parts.is_empty() {
        vec![String::new()]
    } else {
        (0..parts.len())
            .rev()
            .map(|i| parts[i..].join("."))
            .collect()
    };
    for (index, path) in paths.iter().enumerate() {
        let record = source.lookup(&namespace, path)?.ok_or(Error::NotFound)?;
        record.verify(now)?;
        if record.namespace() != namespace || record.path() != path {
            return Err(Error::Signature);
        }
        let signer = record.signer()?;
        if let Some(expected) = authority {
            if signer != expected {
                return Err(Error::Signature);
            }
        } else if !trusted_registry.contains(&signer) {
            return Err(Error::UntrustedRegistry);
        }
        history.observe(&record)?;
        let last = index + 1 == paths.len();
        if last {
            return Ok(record);
        }
        match record.action() {
            RecordKind::Delegate(_) if !last => authority = Some(record.target()?),
            _ => return Err(Error::NotFound),
        }
    }
    Err(Error::NotFound)
}
