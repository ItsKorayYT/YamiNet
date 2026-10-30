//! Strict versioned local configuration. No network, environment mutation or secrets.
use std::{
    collections::HashSet,
    fs::File,
    io::{self, Read},
    net::{SocketAddr, SocketAddrV4},
    path::{Path, PathBuf},
};
use yami_dht::{Contact, Network, RelayRoute};
pub const MAX_CONFIG_BYTES: usize = 16 * 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Node,
    Relay,
    Browser,
}
#[derive(Debug, Clone)]
pub struct Config {
    pub role: Role,
    pub network: Network,
    pub public_listen: bool,
    pub identity: Option<PathBuf>,
    pub state: PathBuf,
    pub site: Option<PathBuf>,
    pub listen: Option<SocketAddr>,
    pub advertised: Vec<SocketAddrV4>,
    pub bootstrap: Vec<Contact>,
    pub relays: Vec<RelayRoute>,
    pub relay_fallback: bool,
    pub dht: bool,
    pub workers: usize,
    pub renew_secs: u64,
    pub health_secs: u64,
}
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}
impl Config {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        if !path.symlink_metadata()?.file_type().is_file() {
            return Err(invalid(
                "configuration must be a regular non-symlink UTF-8 file",
            ));
        }
        let mut text = String::new();
        File::open(path)?
            .take((MAX_CONFIG_BYTES + 1) as u64)
            .read_to_string(&mut text)?;
        Self::parse(&text)
    }
    pub fn parse(text: &str) -> io::Result<Self> {
        if text.len() > MAX_CONFIG_BYTES
            || text.lines().count() > 96
            || text
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r'))
        {
            return Err(invalid(
                "configuration exceeds size/line bounds or contains control characters",
            ));
        }
        let mut lines = text
            .lines()
            .enumerate()
            .filter(|(_, l)| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
        if lines.next().map(|(_, l)| l.trim()) != Some("yaminet-config 1") {
            return Err(invalid("expected yaminet-config 1 header"));
        }
        let mut fields = Vec::new();
        let mut seen = HashSet::new();
        const KEYS: &[&str] = &[
            "role",
            "network",
            "public-listen",
            "identity",
            "state",
            "site",
            "listen",
            "advertise",
            "bootstrap",
            "relay",
            "relay-fallback",
            "dht",
            "workers",
            "renew-secs",
            "health-secs",
        ];
        for (index, line) in lines {
            let (key, value) = line
                .trim()
                .split_once(' ')
                .ok_or_else(|| invalid(format!("line {}: expected field and value", index + 1)))?;
            let value = value.trim();
            if !KEYS.contains(&key) || value.is_empty() {
                return Err(invalid(format!(
                    "line {}: unknown field or missing value",
                    index + 1
                )));
            }
            if !matches!(key, "advertise" | "bootstrap" | "relay") && !seen.insert(key) {
                return Err(invalid(format!(
                    "line {}: duplicate field {key}",
                    index + 1
                )));
            }
            fields.push((key, value));
        }
        let value = |key: &str| fields.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
        let required =
            |key: &str| value(key).ok_or_else(|| invalid(format!("missing required field {key}")));
        let boolean = |key: &str, default: bool| match value(key) {
            None => Ok(default),
            Some("yes") => Ok(true),
            Some("no") => Ok(false),
            _ => Err(invalid(format!("{key}: expected yes or no"))),
        };
        let path = |key: &str| -> io::Result<Option<PathBuf>> {
            value(key)
                .map(|v| {
                    let p = PathBuf::from(v);
                    if !p.is_absolute() {
                        Err(invalid(format!(
                            "{key}: use an absolute persistent path (spaces require no quotes)"
                        )))
                    } else {
                        Ok(p)
                    }
                })
                .transpose()
        };
        let number = |key: &str, default: u64, min: u64, max: u64| -> io::Result<u64> {
            let n = value(key).map_or(Ok(default), |v| {
                v.parse()
                    .map_err(|_| invalid(format!("{key}: expected an integer")))
            })?;
            if !(min..=max).contains(&n) {
                Err(invalid(format!("{key}: must be {min}..{max}")))
            } else {
                Ok(n)
            }
        };
        let role = match required("role")? {
            "node" => Role::Node,
            "relay" => Role::Relay,
            "browser" => Role::Browser,
            _ => return Err(invalid("role: expected node, relay or browser")),
        };
        let network = match value("network").unwrap_or("loopback") {
            "loopback" => Network::Loopback,
            "public" => Network::Public,
            _ => return Err(invalid("network: expected loopback or public")),
        };
        let public_listen = boolean("public-listen", false)?;
        let identity = path("identity")?;
        let state =
            path("state")?.ok_or_else(|| invalid("missing required absolute state directory"))?;
        let site = path("site")?;
        let listen = value("listen")
            .map(|v| {
                v.parse::<SocketAddr>()
                    .map_err(|_| invalid("listen: expected numeric IP:port"))
            })
            .transpose()?;
        if let Some(listen) = listen {
            if !listen.is_ipv4() || listen.port() == 0 {
                return Err(invalid("listen: select one IPv4 address and nonzero port"));
            }
            if public_listen {
                ynp_tls::Scope::Internet.check(listen)?;
            } else {
                ynp_tls::Scope::Loopback.check(listen)?;
            }
        }
        if public_listen && network != Network::Public {
            return Err(invalid("public-listen yes requires network public"));
        }
        let mut advertised = Vec::new();
        let mut bootstrap = Vec::new();
        let mut relays = Vec::new();
        for (key, value) in &fields {
            match *key {
                "advertise" => {
                    let endpoint: SocketAddrV4 = value
                        .parse()
                        .map_err(|_| invalid("advertise: expected IPv4:port"))?;
                    network.check(endpoint)?;
                    if advertised.contains(&endpoint) || advertised.len() == 4 {
                        return Err(invalid("advertise: at most four unique endpoints"));
                    }
                    advertised.push(endpoint);
                }
                "bootstrap" | "relay" => {
                    let (id, endpoint) = value.split_once('@').ok_or_else(|| {
                        invalid(format!("{key}: expected full-node-ID@IPv4:port"))
                    })?;
                    let id = id.parse().map_err(|_| {
                        invalid(format!("{key}: invalid full cryptographic node ID"))
                    })?;
                    let endpoint = endpoint
                        .parse()
                        .map_err(|_| invalid(format!("{key}: invalid IPv4 endpoint")))?;
                    network.check(endpoint)?;
                    if *key == "bootstrap" {
                        if bootstrap.len() == 15 || bootstrap.iter().any(|c: &Contact| c.id == id) {
                            return Err(invalid(
                                "bootstrap: at most fifteen unique pinned identities",
                            ));
                        }
                        bootstrap.push(Contact::new(id, endpoint, network)?);
                    } else {
                        let route = RelayRoute {
                            relay: id,
                            endpoint,
                        };
                        if relays.len() == 2 || relays.contains(&route) {
                            return Err(invalid("relay: at most two unique explicit routes"));
                        }
                        relays.push(route);
                    }
                }
                _ => {}
            }
        }
        relays.sort();
        advertised.sort();
        let relay_fallback = boolean("relay-fallback", false)?;
        let dht = boolean("dht", false)?;
        let workers = number("workers", 4, 1, 32)? as usize;
        let renew_secs = number("renew-secs", 21600, 30, 43200)?;
        let health_secs = number("health-secs", 300, 5, 3600)?;
        let allowed: &[&str] = match role {
            Role::Node => &[
                "role",
                "network",
                "public-listen",
                "identity",
                "state",
                "site",
                "listen",
                "advertise",
                "bootstrap",
                "relay",
                "dht",
                "workers",
                "renew-secs",
                "health-secs",
            ],
            Role::Relay => &[
                "role",
                "network",
                "public-listen",
                "identity",
                "state",
                "listen",
                "advertise",
            ],
            Role::Browser => &["role", "network", "state", "bootstrap", "relay-fallback"],
        };
        if let Some((key, _)) = fields.iter().find(|(key, _)| !allowed.contains(key)) {
            return Err(invalid(format!(
                "field {key} is not supported for role {role:?}"
            )));
        }
        match role {
            Role::Node => {
                if identity.is_none() || site.is_none() {
                    return Err(invalid("node needs existing identity and site directories"));
                }
                if relays.is_empty() {
                    if listen.is_none() {
                        return Err(invalid("direct node needs an explicit listen address"));
                    }
                    if dht
                        && (advertised.is_empty() || (network == Network::Public && !public_listen))
                    {
                        return Err(invalid("DHT direct node needs advertised endpoints and explicit public-listen for public mode"));
                    }
                    if !dht && (!bootstrap.is_empty() || !advertised.is_empty()) {
                        return Err(invalid(
                            "bootstrap/advertise require dht yes for a direct node",
                        ));
                    }
                } else if listen.is_some() || public_listen || !advertised.is_empty() || dht {
                    return Err(invalid("outbound relay-host uses relay pins and no listener, advertise or dht flag (maintenance is automatic)"));
                }
            }
            Role::Relay => {
                if identity.is_none() || listen.is_none() {
                    return Err(invalid(
                        "relay needs existing identity and explicit listen address",
                    ));
                }
                if public_listen && advertised.len() != 1 {
                    return Err(invalid(
                        "public relay needs exactly one separately selected advertise endpoint",
                    ));
                }
                if !public_listen && !advertised.is_empty() {
                    return Err(invalid("loopback relay has no public advertise field"));
                }
            }
            Role::Browser => {}
        }
        Ok(Self {
            role,
            network,
            public_listen,
            identity,
            state,
            site,
            listen,
            advertised,
            bootstrap,
            relays,
            relay_fallback,
            dht,
            workers,
            renew_secs,
            health_secs,
        })
    }
    pub fn expect_role(&self, expected: Role) -> io::Result<()> {
        if self.role != expected {
            Err(invalid("configuration role does not match executable"))
        } else {
            Ok(())
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> String {
        std::env::temp_dir()
            .join("yami-config-test state")
            .display()
            .to_string()
    }
    #[test]
    fn configuration_defaults_are_local_and_reject_ambiguous_or_unsafe_values() {
        let valid = format!("yaminet-config 1\nrole browser\nstate {}\n", state());
        let parsed = Config::parse(&valid).unwrap();
        assert_eq!(parsed.network, Network::Loopback);
        assert!(!parsed.relay_fallback);
        assert!(parsed.listen.is_none());
        for addition in [
            "role relay\n",
            "password NEVER_PRINT_THIS_VALUE\n",
            "relay-fallback maybe\n",
            "listen 0.0.0.0:1\n",
            "public-listen yes\n",
        ] {
            let error = Config::parse(&(valid.clone() + addition))
                .unwrap_err()
                .to_string();
            assert!(!error.contains("NEVER_PRINT_THIS_VALUE"));
        }
        assert!(Config::parse(&valid.replace(&state(), "relative/path")).is_err());
        assert!(Config::parse(&"x".repeat(MAX_CONFIG_BYTES + 1)).is_err());
    }
    #[test]
    fn public_relays_require_independent_explicit_bind_and_advertisement() {
        let base=format!("yaminet-config 1\nrole relay\nidentity {}\nstate {}\nnetwork public\npublic-listen yes\nlisten 127.0.0.1:17550\n",state(),state());
        assert!(Config::parse(&base).is_err());
        assert!(Config::parse(&(base.clone() + "advertise 8.8.8.8:17550\n")).is_ok());
        for endpoint in [
            "0.0.0.0:17550",
            "127.0.0.1:17550",
            "10.0.0.1:17550",
            "100.64.0.1:17550",
            "198.51.100.1:17550",
            "8.8.8.8:0",
        ] {
            assert!(Config::parse(&(base.clone() + &format!("advertise {endpoint}\n"))).is_err());
        }
        assert!(Config::parse(
            &(base.replace("127.0.0.1:17550", "0.0.0.0:17550") + "advertise 8.8.8.8:17550\n")
        )
        .is_err());
    }
    #[test]
    fn explicit_pins_are_bounded_and_role_specific() {
        let id = yami_identity::NodeIdentity::generate()
            .unwrap()
            .id()
            .to_string();
        let pin = format!("bootstrap {id}@127.0.0.1:7443\n");
        let base = format!("yaminet-config 1\nrole browser\nstate {}\n", state());
        assert_eq!(
            Config::parse(&(base.clone() + &pin))
                .unwrap()
                .bootstrap
                .len(),
            1
        );
        assert!(Config::parse(&(base.clone() + &pin + &pin)).is_err());
        assert!(Config::parse(&(base + "network public\n" + &pin)).is_err());
    }
}

pub mod status;
