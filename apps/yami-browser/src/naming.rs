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
        self.resolve_until(
            resolver,
            std::time::Instant::now() + yami_yns::network::DISCOVERY_BUDGET,
        )
    }
    pub fn resolve_until(
        &self,
        resolver: &Resolver,
        deadline: std::time::Instant,
    ) -> Result<Address, String> {
        Ok(Address {
            id: resolver.resolve_name_until(&self.name, deadline, false)?,
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
/// Resolve on a loader worker. Cached records are usable only while their entire
/// chain and persistent history remain valid. A failed refresh never erases pins
/// or permits rollback, and observed signed changes are installed before resolving.
pub fn resolve_remote(
    resolver: &Resolver,
    name: &Name,
    deadline: std::time::Instant,
    force: bool,
) -> yami_yns::Result<yami_identity::NodeId> {
    use yami_yns::{network, Error, Store};
    let mut store = Store::open(resolver.naming_path())?;
    let at = now().map_err(|_| Error::Invalid("clock unavailable"))?;
    if matches!(name, Name::Alias(_))
        || (matches!(name, Name::Official(_)) && store.registry_keys()?.is_empty())
    {
        return store.resolve(name, at);
    }
    let cached = store.cached_resolution(name, at);
    let recent = store
        .refreshed_at(name)
        .is_some_and(|last| at >= last && at - last < network::REFRESH_SECONDS);
    if !force && cached.is_ok() && recent {
        return store.resolve(name, at);
    }
    if store.providers().is_empty() {
        return match cached {
            Err(Error::NotFound | Error::Expired) if !matches!(name, Name::Alias(_)) => {
                Err(Error::NoProviders)
            }
            _ if force => Err(Error::NoProviders),
            _ => store.resolve(name, at),
        };
    }
    let discovered = network::discover(
        store.providers(),
        resolver.naming_network(),
        name,
        &store.registry_keys()?,
        at,
        deadline,
    )?;
    let after = now().map_err(|_| Error::Invalid("clock unavailable"))?;
    if discovered.bundles.is_empty() {
        if force {
            return Err(Error::NotFound);
        }
        return store.resolve(name, after);
    }
    store.refresh(name, &discovered.bundles, after)?;
    let id = store.resolve(name, after)?;
    store.mark_refreshed(name, after)?;
    Ok(id)
}

/// Native management interface. Publication sends existing signed records; no
/// owner secret key is sent to serving nodes or mirrors.
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
        ["resolve", name] => Ok(resolver
            .resolve_name(&Name::parse(name).map_err(error)?)?
            .to_string()),
        ["refresh", name] => Ok(resolver
            .resolve_name_until(
                &Name::parse(name).map_err(error)?,
                std::time::Instant::now() + yami_yns::network::DISCOVERY_BUDGET,
                true,
            )?
            .to_string()),
        ["publish", name, provider, files @ ..] if !files.is_empty() && files.len() <= 8 => {
            let name = Name::parse(name).map_err(error)?;
            let provider = yami_yns::network::Provider::parse(provider, resolver.naming_network())
                .map_err(error)?;
            let mut records = Vec::new();
            for path in files {
                let mut bytes = Vec::new();
                File::open(path)
                    .map_err(|e| e.to_string())?
                    .take(yami_yns::MAX_RECORD_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                records.push(Record::parse(&bytes).map_err(error)?);
            }
            let bundle = yami_yns::Bundle {
                name: name.to_string(),
                records,
            };
            let store = Store::open(resolver.naming_path()).map_err(error)?;
            bundle
                .verify(&store.registry_keys().map_err(error)?, now()?)
                .map_err(error)?;
            yami_yns::network::publish(
                &provider,
                resolver.naming_network(),
                bundle,
                std::time::Instant::now() + yami_yns::network::RPC_BUDGET,
            )
            .map_err(error)?;
            Ok(
                "Signed chain published over authenticated YNP; mirror ownership remains unchanged"
                    .into(),
            )
        }
        ["mirror", name, source, destination] => {
            let name = Name::parse(name).map_err(error)?;
            let source = yami_yns::network::Provider::parse(source, resolver.naming_network())
                .map_err(error)?;
            let destination =
                yami_yns::network::Provider::parse(destination, resolver.naming_network())
                    .map_err(error)?;
            let mut store = Store::open(resolver.naming_path()).map_err(error)?;
            let at = now()?;
            let found = yami_yns::network::discover(
                &[source],
                resolver.naming_network(),
                &name,
                &store.registry_keys().map_err(error)?,
                at,
                std::time::Instant::now() + yami_yns::network::DISCOVERY_BUDGET,
            )
            .map_err(error)?;
            if found.bundles.is_empty() {
                return Err("No authenticated chain available to mirror".into());
            }
            store
                .refresh(&name, &found.bundles, now()?)
                .map_err(error)?;
            let bundle = store
                .export_bundle(&name, now()?)
                .map_err(error)?
                .ok_or("No live chain to mirror")?;
            yami_yns::network::publish(
                &destination,
                resolver.naming_network(),
                bundle,
                std::time::Instant::now() + yami_yns::network::RPC_BUDGET,
            )
            .map_err(error)?;
            Ok("Signed chain mirrored without access to owner keys".into())
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
                ["providers"] => Ok(store.providers().iter().map(|p|format!("{}@{}\n",p.id,p.endpoint)).collect()),
                ["providers-set", peers @ ..] => {
                    let peers=peers.iter().map(|p|yami_yns::network::Provider::parse(p,resolver.naming_network())).collect::<yami_yns::Result<Vec<_>>>().map_err(error)?;
                    store.configure_providers(peers,resolver.naming_network()).map_err(error)?; Ok("Initial YNS provider pins configured locally".into())
                },
                ["approve-change", name, previous, proposed] => { store.approve_change(&Name::parse(name).map_err(error)?, id(previous)?, id(proposed)?, now()?).map_err(error)?; Ok("Reviewed identity change saved".into()) },
                ["registry-keys"] => Ok(store.registry_keys().map_err(error)?.iter().map(|k| format!("{k}\n")).collect()),
                ["registry-set", keys @ ..] => { let keys = keys.iter().map(|s| id(s)).collect::<Result<Vec<_>,_>>()?; store.configure_registry(&keys).map_err(error)?; Ok("Official .yami registry trust configured locally".into()) },
                _ => Err("Use yns providers | providers-set [ID@IPv4:port ...] | publish NAME PROVIDER RECORD-FILE... | mirror NAME SOURCE DESTINATION | refresh NAME | namespace-create DIR | record-sign DIR NAMESPACE PATH|- SEQUENCE ISSUED EXPIRES map|delegate|revoke TARGET|- | aliases | alias-set LABEL ID | alias-replace LABEL OLD-ID NEW-ID | alias-remove LABEL OLD-ID | import FILE | resolve NAME | approve-change NAME OLD-ID NEW-ID | registry-keys | registry-set [KEY ...]. See docs/yns.md".into()),
            }
        }
    }
}
