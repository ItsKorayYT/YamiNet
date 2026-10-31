//! Deterministic systemd unit rendering. Writing or installing a unit remains an
//! explicit operator action; this module never touches `/etc` or systemd.
use std::{
    fs, io,
    path::{Path, PathBuf},
};
use yami_config::Role;

use crate::config::Loaded;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitRole {
    Bootstrap,
    Relay,
    Host,
}

impl UnitRole {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "bootstrap" => Ok(Self::Bootstrap),
            "relay" => Ok(Self::Relay),
            "host" => Ok(Self::Host),
            _ => Err("unit role must be bootstrap, relay or host".into()),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bootstrap => "bootstrap",
            Self::Relay => "relay",
            Self::Host => "host",
        }
    }

    pub fn unit_name(self) -> &'static str {
        match self {
            Self::Bootstrap => "yaminet-testnet-bootstrap.service",
            Self::Relay => "yaminet-testnet-relay.service",
            Self::Host => "yaminet-testnet-host.service",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Bootstrap => "YamiNet public testnet DHT bootstrap node",
            Self::Relay => "YamiNet public testnet relay",
            Self::Host => "YamiNet public testnet website host",
        }
    }

    fn expected_role(self) -> Role {
        match self {
            Self::Bootstrap | Self::Host => Role::Node,
            Self::Relay => Role::Relay,
        }
    }
}

#[derive(Debug, Clone)]
pub struct UnitSpec {
    pub role: UnitRole,
    pub config: PathBuf,
    pub binary: PathBuf,
    pub user: String,
    pub group: String,
}

impl UnitSpec {
    pub fn new(role: UnitRole, config: PathBuf) -> Self {
        Self {
            role,
            config,
            binary: default_binary(role),
            user: "yaminet".into(),
            group: "yaminet".into(),
        }
    }
}

fn default_binary(role: UnitRole) -> PathBuf {
    let name = match role {
        UnitRole::Relay => "yami-relay",
        _ => "yami-node",
    };
    #[cfg(unix)]
    {
        PathBuf::from("/opt/yaminet").join(name)
    }
    #[cfg(not(unix))]
    {
        std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|parent| parent.join(name)))
            .unwrap_or_else(|| PathBuf::from(name))
    }
}

const SAFE_UNIT_CHARS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_.-";

fn valid_account(text: &str) -> bool {
    if text.is_empty() || text.len() > 32 || text.chars().any(char::is_whitespace) {
        return false;
    }
    let mut chars = text.chars();
    let first = chars.next().unwrap_or(' ');
    (first.is_ascii_alphabetic() || first == '_')
        && chars
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

/// Quote one systemd command argument. `%` starts a specifier, so it is always
/// doubled even when the argument does not otherwise need quoting.
fn quote(argument: &Path) -> Result<String, String> {
    let text = argument
        .to_str()
        .ok_or_else(|| "systemd unit paths must be valid UTF-8".to_string())?;
    if text.chars().any(char::is_control) || text.is_empty() {
        return Err("systemd unit paths must be nonempty and free of control characters".into());
    }
    let escaped = text.replace('%', "%%");
    let safe = escaped
        .chars()
        .all(|character| SAFE_UNIT_CHARS.contains(character) || matches!(character, '/' | ':'));
    if safe {
        Ok(escaped)
    } else {
        Ok(format!(
            "\"{}\"",
            escaped.replace('\\', "\\\\").replace('"', "\\\"")
        ))
    }
}

fn reject_protected_home(path: &Path, label: &str) -> Result<(), String> {
    #[cfg(unix)]
    {
        if path.starts_with("/home") || path.starts_with("/root") || path.starts_with("/run/user") {
            return Err(format!(
                "{label} is under a home directory hidden by ProtectHome=true; use a service-owned path such as /var/lib/yaminet"
            ));
        }
    }
    let _ = (path, label);
    Ok(())
}

pub fn render(spec: &UnitSpec) -> Result<String, String> {
    if !valid_account(&spec.user) {
        return Err("service user must be a simple account name (letters, digits, _ or -)".into());
    }
    if !valid_account(&spec.group) {
        return Err("service group must be a simple account name (letters, digits, _ or -)".into());
    }
    if !spec.binary.is_absolute() {
        return Err("service binary path must be absolute".into());
    }
    if !spec.config.is_absolute() {
        return Err("service configuration path must be absolute".into());
    }
    let config_path = fs::canonicalize(&spec.config)
        .map_err(|_| "service configuration file is missing or unreadable".to_string())?;
    reject_protected_home(&config_path, "configuration file")?;
    let loaded = Loaded::load(&config_path, Some(spec.role.expected_role()))?;
    if spec.role == UnitRole::Bootstrap && !loaded.config.dht {
        return Err("a bootstrap unit requires dht yes in its node configuration".into());
    }
    let identity =
        loaded.config.identity.as_ref().ok_or_else(|| {
            "service configuration has no persistent identity directory".to_string()
        })?;
    let site = loaded.config.site.as_ref();
    if spec.role != UnitRole::Relay && site.is_none() {
        return Err("service configuration has no website directory".into());
    }
    if !identity.is_dir() {
        return Err("configured identity directory does not exist".into());
    }
    if let Some(site) = site {
        if !site.is_dir() {
            return Err("configured website directory does not exist".into());
        }
        reject_protected_home(site, "website directory")?;
    }
    reject_protected_home(identity, "identity directory")?;
    reject_protected_home(&loaded.config.state, "state directory")?;
    let binary = quote(&spec.binary)?;
    let config = quote(&config_path)?;
    let state = quote(&loaded.config.state)?;
    let mut unit = format!(
        "[Unit]\nDescription={}\nAfter=network-online.target\nWants=network-online.target\nStartLimitIntervalSec=60\nStartLimitBurst=5\n\n[Service]\nType=simple\nUser={}\nGroup={}\nExecStart={binary} --config {config}\nRestart=on-failure\nRestartSec=5s\nTimeoutStopSec=20s\nUMask=0077\nNoNewPrivileges=true\nPrivateTmp=true\nProtectSystem=strict\nProtectHome=true\nReadWritePaths={state}\nRestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX\nLimitNOFILE=512\nTasksMax=80\nMemoryMax=256M\nStandardOutput=journal\nStandardError=journal\nSyslogIdentifier=yaminet-testnet-{}\n\n[Install]\nWantedBy=multi-user.target\n",
        spec.role.description(),
        spec.user,
        spec.group,
        spec.role.as_str()
    );
    if let Some(listen) = loaded.config.listen {
        unit.push_str(&format!(
            "# Configured listener: {listen}; public-listen: {}. This comment does not open a port.\n",
            loaded.config.public_listen
        ));
    }
    Ok(unit)
}

/// Write a rendered unit only when the destination is an operator-selected
/// path. Existing different content is not overwritten unless `force` is set.
pub fn write_unit(path: &Path, unit: &str, force: bool) -> io::Result<bool> {
    if let Ok(existing) = fs::read_to_string(path) {
        if existing == unit {
            return Ok(false);
        }
        if !force {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "unit file already exists with different content; inspect it or pass --force",
            ));
        }
    }
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "unit needs a parent directory")
    })?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, unit)?;
    fs::rename(&temporary, path)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn site(root: &Path) {
        yami_node::authoring::new_site(root, "personal").unwrap();
    }

    fn spec(role: UnitRole, base: &Path) -> (UnitSpec, std::path::PathBuf) {
        let identity = base.join("identity");
        let site_path = base.join("site");
        let state = base.join("state");
        if !identity.exists() {
            yami_identity::NodeIdentity::create(&identity).unwrap();
            site(&site_path);
        }
        let config = base.join("node.conf");
        let text = if role == UnitRole::Relay {
            format!(
                "yaminet-config 1\nrole relay\nnetwork loopback\nidentity {}\nstate {}\nlisten 127.0.0.1:17550\n",
                identity.display(),
                state.display()
            )
        } else if role == UnitRole::Bootstrap {
            format!(
                "yaminet-config 1\nrole node\nnetwork loopback\npublic-listen no\nidentity {}\nstate {}\nsite {}\nlisten 127.0.0.1:17444\nadvertise 127.0.0.1:17444\ndht yes\n",
                identity.display(),
                state.display(),
                site_path.display()
            )
        } else {
            format!(
                "yaminet-config 1\nrole node\nnetwork loopback\nidentity {}\nstate {}\nsite {}\nrelay {}@127.0.0.1:17550\nbootstrap {}@127.0.0.1:17444\n",
                identity.display(),
                state.display(),
                site_path.display(),
                yami_identity::NodeIdentity::generate().unwrap().id(),
                yami_identity::NodeIdentity::generate().unwrap().id()
            )
        };
        fs::write(&config, text).unwrap();
        (UnitSpec::new(role, config), identity)
    }

    #[test]
    fn units_are_role_specific_and_restart_safely() {
        for (index, role) in [UnitRole::Bootstrap, UnitRole::Relay, UnitRole::Host]
            .into_iter()
            .enumerate()
        {
            let base = std::env::temp_dir()
                .join(format!("yami-testnet-unit-{index}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&base);
            let (spec, _) = spec(role, &base);
            let unit = render(&spec).unwrap();
            assert!(unit.contains(&format!("User={}", spec.user)));
            assert!(unit.contains("Restart=on-failure"));
            assert!(unit.contains("RestartSec=5s"));
            assert!(unit.contains("ProtectSystem=strict"));
            assert!(unit.contains("SyslogIdentifier=yaminet-testnet-"));
            assert!(unit.contains("ExecStart="));
            assert!(unit.contains("--config"));
            assert!(unit.contains(role.unit_name().trim_end_matches(".service")));
            let _ = fs::remove_dir_all(&base);
        }
    }

    #[test]
    fn exec_start_quotes_spaces_percent_and_rejects_bad_accounts() {
        let base = std::env::temp_dir().join(format!("yami-testnet-quote-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let (mut spec, _) = spec(UnitRole::Bootstrap, &base);
        spec.binary = base.join("Yami Net 100%/yami-node");
        let unit = render(&spec).unwrap();
        assert!(unit.contains("Yami Net 100%%/yami-node"));
        assert!(unit.contains("ExecStart=\""));
        spec.user = "bad user\nExecStart=/bin/sh".into();
        assert!(render(&spec).is_err());
        spec.user = "yaminet".into();
        spec.group = "yaminet;rm".into();
        assert!(render(&spec).is_err());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn wrong_role_and_missing_identity_are_rejected() {
        let base = std::env::temp_dir().join(format!("yami-testnet-role-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let (spec, _) = spec(UnitRole::Bootstrap, &base);
        let mut relay = spec.clone();
        relay.role = UnitRole::Relay;
        assert!(render(&relay).is_err());
        let mut missing = spec;
        missing.config = base.join("missing.conf");
        assert!(render(&missing).is_err());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn write_unit_is_idempotent_and_refuses_unreviewed_overwrite() {
        let base = std::env::temp_dir().join(format!("yami-testnet-write-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let path = base.join("unit.service");
        assert!(write_unit(&path, "one\n", false).unwrap());
        assert!(!write_unit(&path, "one\n", false).unwrap());
        assert!(write_unit(&path, "two\n", false).is_err());
        assert!(write_unit(&path, "two\n", true).unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), "two\n");
        let _ = fs::remove_dir_all(&base);
    }
}
