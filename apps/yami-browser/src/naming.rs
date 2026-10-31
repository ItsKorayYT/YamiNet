//! Browser integration for YNS. Readable names are resolved on loader workers;
//! the resulting full identity enters the unchanged DHT/pinned TLS path.
use crate::native::{Address, Resolver, SCHEME};
use yami_yns::Name;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadableAddress {
    pub name: Name,
    pub key: String,
}
impl ReadableAddress {
    pub fn parse(text: &str) -> Result<Self, String> {
        let rest = text
            .strip_prefix(SCHEME)
            .ok_or("Expected yami:// address")?;
        let (name, key) = rest.split_once('/').unwrap_or((rest, "home"));
        let key = if key.is_empty() { "home" } else { key };
        ynp::validate_key(key).map_err(|e| e.to_string())?;
        Ok(Self {
            name: Name::parse(name).map_err(|e| e.to_string())?,
            key: key.into(),
        })
    }
    pub fn url(&self) -> String {
        format!("{SCHEME}{}/{}", self.name, self.key)
    }
    pub fn resolve(&self, resolver: &Resolver) -> Result<Address, String> {
        Ok(Address {
            id: resolver.resolve_name(&self.name)?,
            key: self.key.clone(),
        })
    }
}
pub fn now() -> Result<u64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|e| e.to_string())
}
/// Native, local management interface. No network publication or registry
/// registration is implied by creating a namespace or importing a record.
pub fn command(args: &[String], resolver: &Resolver) -> Result<String, String> {
    use std::{fs::File, io::Read};
    use yami_identity::NodeIdentity;
    use yami_yns::{Record, RecordKind, Store};
    let error = |e: yami_yns::Error| e.to_string();
    let id = |s: &str| {
        s.parse::<yami_identity::NodeId>()
            .map_err(|e| e.to_string())
    };
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["namespace-create", dir] => {
            let owner = NodeIdentity::create(dir).map_err(|e| e.to_string())?;
            Ok(format!("{}.yns\nBack up the owner key directory privately. No project approval is required.", owner.id()))
        }
        ["record-sign", dir, namespace, path, sequence, issued, expires, kind, target] => {
            let owner = NodeIdentity::load(dir).map_err(|e| e.to_string())?;
            let action = match *kind {
                "map" => RecordKind::Map(id(target)?.to_string()),
                "delegate" => RecordKind::Delegate(id(target)?.to_string()),
                "revoke" if *target == "-" => RecordKind::Revoke,
                _ => return Err("Use map|delegate FULL-ID or revoke -".into()),
            };
            let number = |s: &str| {
                s.parse::<u64>()
                    .map_err(|_| "sequence and times must be positive integers".to_string())
            };
            let record = Record::sign(
                &owner,
                namespace,
                if *path == "-" { "" } else { path },
                number(sequence)?,
                number(issued)?,
                number(expires)?,
                action,
            )
            .map_err(error)?;
            String::from_utf8(record.to_bytes()).map_err(|e| e.to_string())
        }
        rest => {
            let mut store = Store::open(resolver.naming_path()).map_err(error)?;
            match rest {
                ["aliases"] => Ok(store.aliases().iter().map(|(a,id)| format!("@{a} {id}\n")).collect()),
                ["alias-set", alias, target] => { store.set_alias(alias, id(target)?, None).map_err(error)?; Ok("Alias saved".into()) },
                ["alias-replace", alias, previous, target] => { store.set_alias(alias, id(target)?, Some(id(previous)?)).map_err(error)?; Ok("Alias replacement saved".into()) },
                ["alias-remove", alias, previous] => { store.remove_alias(alias, id(previous)?).map_err(error)?; Ok("Alias removed".into()) },
                ["import", path] => {
                    let mut bytes = Vec::new();
                    File::open(path).map_err(|e| e.to_string())?.take(yami_yns::MAX_RECORD_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
                    store.import(Record::parse(&bytes).map_err(error)?, now()?).map_err(error)?;
                    Ok("Authenticated record imported; existing destination bindings remain pinned".into())
                },
                ["resolve", name] => Ok(store.resolve(&Name::parse(name).map_err(error)?, now()?).map_err(error)?.to_string()),
                ["approve-change", name, previous, proposed] => { store.approve_change(&Name::parse(name).map_err(error)?, id(previous)?, id(proposed)?, now()?).map_err(error)?; Ok("Reviewed identity change saved".into()) },
                ["registry-keys"] => Ok(store.registry_keys().map_err(error)?.iter().map(|k| format!("{k}\n")).collect()),
                ["registry-set", keys @ ..] => { let keys = keys.iter().map(|s| id(s)).collect::<Result<Vec<_>,_>>()?; store.configure_registry(&keys).map_err(error)?; Ok("Official .yami registry trust configured locally".into()) },
                _ => Err("Use yns namespace-create DIR | record-sign DIR NAMESPACE PATH|- SEQUENCE ISSUED EXPIRES map|delegate|revoke TARGET|- | aliases | alias-set LABEL ID | alias-replace LABEL OLD-ID NEW-ID | alias-remove LABEL OLD-ID | import FILE | resolve NAME | approve-change NAME OLD-ID NEW-ID | registry-keys | registry-set [KEY ...]. See docs/yns.md".into()),
            }
        }
    }
}
