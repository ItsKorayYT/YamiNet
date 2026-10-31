use crate::{trust::authenticate, Error, History, Name, Record, RecordSource, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use yami_identity::NodeId;
const MAX_STATE: usize = 4 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Data {
    version: u8,
    aliases: BTreeMap<String, String>,
    registry: Vec<String>,
    records: BTreeMap<String, Record>,
    history: History,
}
impl Default for Data {
    fn default() -> Self {
        Self {
            version: 1,
            aliases: BTreeMap::new(),
            registry: Vec::new(),
            records: BTreeMap::new(),
            history: History::default(),
        }
    }
}
impl RecordSource for Data {
    fn lookup(&self, namespace: &str, path: &str) -> Result<Option<Record>> {
        Ok(self.records.get(&format!("{namespace}/{path}")).cloned())
    }
}
/// Exclusive local state lock and durable replacement. Missing state starts
/// empty; damaged, locked, or unwritable state fails closed. Direct identity
/// browsing does not open this store. Local filesystem administrators remain
/// trusted; restoring an old profile also restores its rollback history.
pub struct Store {
    dir: PathBuf,
    data: Data,
    _lock: File,
}
fn regular(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if !m.file_type().is_file() => Err(Error::Invalid(
            "state files must be regular non-symlink files",
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
        _ => Ok(()),
    }
}
impl Store {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(dir)?;
        if !fs::symlink_metadata(dir)?.file_type().is_dir() {
            return Err(Error::Invalid("state directory must not be a symlink"));
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        regular(&dir.join("yns.lock"))?;
        let lock = options.open(dir.join("yns.lock"))?;
        lock.try_lock()
            .map_err(|_| Error::Invalid("YNS state is locked by another operation"))?;
        regular(&dir.join("yns.json"))?;
        let data = match File::open(dir.join("yns.json")) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(MAX_STATE as u64 + 1).read_to_end(&mut bytes)?;
                if bytes.len() > MAX_STATE {
                    return Err(Error::Invalid("state too large"));
                }
                let data: Data = serde_json::from_slice(&bytes)
                    .map_err(|_| Error::Invalid("corrupt YNS state; restore a trusted backup"))?;
                if data.version != 1
                    || data.aliases.len() > MAX_ENTRIES
                    || data.records.len() > MAX_ENTRIES
                    || data.registry.len() > 32
                {
                    return Err(Error::Invalid("unsupported or oversized state"));
                }
                for (alias, id) in &data.aliases {
                    crate::label(alias)?;
                    parse_id(id)?;
                }
                for key in &data.registry {
                    parse_id(key)?;
                }
                for (key, record) in &data.records {
                    Record::parse(&record.to_bytes())?;
                    if key != &format!("{}/{}", record.namespace(), record.path()) {
                        return Err(Error::Invalid("record index mismatch"));
                    }
                }
                data
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Data::default(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            dir: dir.into(),
            data,
            _lock: lock,
        })
    }
    fn commit(&mut self, data: Data) -> Result<()> {
        let bytes =
            serde_json::to_vec_pretty(&data).map_err(|_| Error::Invalid("state serialization"))?;
        if bytes.len() > MAX_STATE {
            return Err(Error::Invalid(
                "state full; retain history in a trusted backup",
            ));
        }
        let tmp = self.dir.join("yns.tmp");
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&tmp)?;
        let result = (|| -> std::io::Result<()> {
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            regular(&self.dir.join("yns.json")).map_err(std::io::Error::other)?;
            fs::rename(&tmp, self.dir.join("yns.json"))?;
            #[cfg(unix)]
            File::open(&self.dir)?.sync_all()?;
            Ok(())
        })();
        if let Err(e) = result {
            let _ = fs::remove_file(tmp);
            return Err(e.into());
        }
        self.data = data;
        Ok(())
    }
    pub fn aliases(&self) -> &BTreeMap<String, String> {
        &self.data.aliases
    }
    /// Replacement requires the exact previously trusted identity. Creation
    /// requires None; stale approvals cannot overwrite newer local edits.
    pub fn set_alias(
        &mut self,
        alias: &str,
        target: NodeId,
        expected: Option<NodeId>,
    ) -> Result<()> {
        crate::label(alias)?;
        let previous = self
            .data
            .aliases
            .get(alias)
            .map(|id| parse_id(id))
            .transpose()?;
        if previous != expected {
            if let Some(previous) = previous {
                return Err(Error::IdentityChanged {
                    name: format!("@{alias}"),
                    previous,
                    proposed: target,
                });
            }
            return Err(Error::Invalid(
                "alias approval does not match existing state",
            ));
        }
        if !self.data.aliases.contains_key(alias) && self.data.aliases.len() >= MAX_ENTRIES {
            return Err(Error::Invalid("alias store full"));
        }
        let mut next = self.data.clone();
        next.aliases.insert(alias.into(), target.to_string());
        self.commit(next)
    }
    pub fn remove_alias(&mut self, alias: &str, expected: NodeId) -> Result<()> {
        crate::label(alias)?;
        if self.data.aliases.get(alias) != Some(&expected.to_string()) {
            return Err(Error::Invalid(
                "alias approval does not match existing state",
            ));
        }
        let mut next = self.data.clone();
        next.aliases.remove(alias);
        self.commit(next)
    }
    pub fn registry_keys(&self) -> Result<Vec<NodeId>> {
        self.data.registry.iter().map(|s| parse_id(s)).collect()
    }
    /// Explicit operator configuration only. No keys are generated or shipped.
    pub fn configure_registry(&mut self, keys: &[NodeId]) -> Result<()> {
        if keys.len() > 32 {
            return Err(Error::Invalid("too many registry keys"));
        }
        let mut next = self.data.clone();
        next.registry = keys.iter().map(ToString::to_string).collect();
        next.registry.sort();
        next.registry.dedup();
        self.commit(next)
    }
    pub fn import(&mut self, record: Record, now: u64) -> Result<()> {
        record.verify(now)?;
        let name = record_name(&record)?;
        let key = format!("{}/{}", record.namespace(), record.path());
        if !self.data.records.contains_key(&key) && self.data.records.len() >= MAX_ENTRIES {
            return Err(Error::Invalid("record store full"));
        }
        let mut next = self.data.clone();
        next.records.insert(key, record);
        let mut history = next.history.clone();
        // Validate the complete chain, including updates to delegations and
        // tombstones. Unrelated signers cannot poison the local record cache.
        authenticate(&next, &name, &self.registry_keys()?, &mut history, now)?;
        next.history = history;
        self.commit(next)
    }
    pub fn resolve(&mut self, name: &Name, now: u64) -> Result<NodeId> {
        if Name::parse(&name.to_string())? != *name {
            return Err(Error::Invalid("noncanonical name"));
        }
        if let Name::Alias(alias) = name {
            return self
                .data
                .aliases
                .get(alias)
                .ok_or(Error::NotFound)
                .and_then(|s| parse_id(s));
        }
        let mut next = self.data.clone();
        let result = crate::resolve(
            &self.data,
            name,
            &self.registry_keys()?,
            &mut next.history,
            now,
        );
        if next != self.data {
            self.commit(next)?;
        }
        result
    }
    pub fn approve_change(
        &mut self,
        name: &Name,
        previous: NodeId,
        proposed: NodeId,
        now: u64,
    ) -> Result<()> {
        if matches!(name, Name::Alias(_)) {
            return Err(Error::Invalid("use explicit alias replacement"));
        }
        if self.data.history.binding(name) != Some(previous.to_string().as_str()) {
            return Err(Error::Invalid("approval does not match trusted binding"));
        }
        let mut next = self.data.clone();
        next.history.forget_binding(name);
        let found = crate::resolve(
            &self.data,
            name,
            &self.registry_keys()?,
            &mut next.history,
            now,
        )?;
        if found != proposed {
            return Err(Error::Invalid(
                "approval does not match current signed target",
            ));
        }
        self.commit(next)
    }
}
fn parse_id(s: &str) -> Result<NodeId> {
    s.parse()
        .map_err(|_| Error::Invalid("invalid full identity in state"))
}
fn record_name(record: &Record) -> Result<Name> {
    let suffix = if record.namespace() == "yami" {
        "yami"
    } else {
        record.namespace()
    };
    let name = if record.path().is_empty() {
        suffix.into()
    } else {
        format!("{}.{suffix}", record.path())
    };
    Name::parse(&name)
}
