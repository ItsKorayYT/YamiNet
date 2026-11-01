//! Configuration and local-file checks reused by `check-config` and `preflight`.
use std::{
    fs, io,
    net::TcpListener,
    path::{Path, PathBuf},
};
use yami_config::{Config, Role};
use yami_identity::{NodeId, NodeIdentity, ID_FILE, KEY_FILE};

use crate::report::Status;

pub struct Loaded {
    pub config: Config,
    pub path: PathBuf,
}

impl Loaded {
    pub fn load(path: &Path, expected: Option<Role>) -> Result<Self, String> {
        let config = Config::load(path).map_err(config_error)?;
        if let Some(expected) = expected {
            if config.role != expected {
                return Err(format!(
                    "configuration role is {:?}, expected {:?}",
                    config.role, expected
                ));
            }
        }
        cross_field_checks(&config)?;
        Ok(Self {
            config,
            path: path.to_path_buf(),
        })
    }
}

fn config_error(error: io::Error) -> String {
    match error.kind() {
        io::ErrorKind::NotFound => "configuration file is missing".into(),
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => {
            format!("configuration failed strict validation: {error}")
        }
        io::ErrorKind::PermissionDenied => "configuration file is not readable".into(),
        _ => "configuration could not be read".into(),
    }
}

/// Checks that are intentionally outside the shared config parser because they
/// need the filesystem layout rather than just the selected role.
fn cross_field_checks(config: &Config) -> Result<(), String> {
    if let (Some(identity), Some(site)) = (&config.identity, &config.site) {
        if identity.starts_with(site) {
            return Err("identity directory must stay outside the website directory".into());
        }
        if site.starts_with(identity) {
            return Err("website directory must stay outside the identity directory".into());
        }
    }
    if let Some(site) = &config.site {
        if config.state.starts_with(site) && config.state != *site {
            return Err("state directory must stay outside the website directory".into());
        }
        if config.state == *site {
            return Err("state and website directories must be different".into());
        }
    }
    Ok(())
}

pub fn describe(config: &Config) -> String {
    let mode = match config.role {
        Role::Node if !config.relays.is_empty() => "outbound relay host (no inbound listener)",
        Role::Node if config.public_listen => "direct node with explicit public listener",
        Role::Node => "local node",
        Role::Relay if config.public_listen => "relay with explicit public listener",
        Role::Relay => "local relay",
        Role::Browser => "browser/client profile",
    };
    let listener = config
        .listen
        .map(|address| address.to_string())
        .unwrap_or_else(|| "none".into());
    format!(
        "{mode}; network {:?}; listener {listener}; public-listen {}; {} bootstrap pin(s); {} relay pin(s); dht {}; yns {}; relay-fallback {}",
        config.network,
        config.public_listen,
        config.bootstrap.len(),
        config.relays.len(),
        config.dht,
        config.yns,
        config.relay_fallback
    )
}

#[derive(Debug)]
pub struct IdentityInfo {
    pub id: NodeId,
    pub directory_mode_ok: Option<bool>,
    pub key_mode_ok: Option<bool>,
    pub node_id_present: bool,
}

/// Inspect files without ever reading key material into a report.
pub fn inspect_identity(directory: &Path) -> Result<IdentityInfo, String> {
    let metadata = fs::symlink_metadata(directory).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => "identity directory is missing".to_string(),
        io::ErrorKind::PermissionDenied => "identity directory is not readable".to_string(),
        _ => "identity directory could not be inspected".to_string(),
    })?;
    if metadata.file_type().is_symlink() {
        return Err("identity directory must not be a symlink".into());
    }
    if !metadata.is_dir() {
        return Err("identity path is not a directory".into());
    }
    let key_path = directory.join(KEY_FILE);
    let key_metadata = fs::symlink_metadata(&key_path).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => {
            "node.key is missing (refusing to create a replacement)".to_string()
        }
        io::ErrorKind::PermissionDenied => "node.key is not readable".to_string(),
        _ => "node.key could not be inspected".to_string(),
    })?;
    if key_metadata.file_type().is_symlink() || !key_metadata.file_type().is_file() {
        return Err("node.key must be a regular non-symlink file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if key_metadata.nlink() != 1 {
            return Err("node.key must have exactly one hard link".into());
        }
    }
    let directory_mode_ok = permission_ok(&metadata);
    let key_mode_ok = permission_ok(&key_metadata);
    if key_mode_ok == Some(false) {
        return Err(
            "node.key permissions are too open; run chmod 600 node.key (owner-only access required)"
                .into(),
        );
    }
    let identity = NodeIdentity::load(directory).map_err(identity_error)?;
    let node_id_present = fs::symlink_metadata(directory.join(ID_FILE)).is_ok();
    Ok(IdentityInfo {
        id: *identity.id(),
        directory_mode_ok,
        key_mode_ok,
        node_id_present,
    })
}

#[cfg(unix)]
fn permission_ok(metadata: &fs::Metadata) -> Option<bool> {
    use std::os::unix::fs::PermissionsExt;
    Some(metadata.permissions().mode() & 0o077 == 0)
}

#[cfg(not(unix))]
fn permission_ok(_: &fs::Metadata) -> Option<bool> {
    None
}

fn identity_error(error: yami_identity::Error) -> String {
    use yami_identity::Error;
    match error {
        Error::MissingKey(_) => "node.key is missing (refusing to create a replacement)".into(),
        Error::CorruptKey(reason) => format!("node.key is malformed or inconsistent: {reason}"),
        Error::InsecurePermissions(_) => {
            "node.key permissions are too open; owner-only access is required".into()
        }
        Error::Mismatch { .. } => "node.id does not match node.key".into(),
        Error::InvalidNodeId(reason) => format!("node.id is invalid: {reason}"),
        Error::Io(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            "identity files are not readable by this user".into()
        }
        Error::Io(_) => "identity files could not be read safely".into(),
        _ => "identity failed validation".into(),
    }
}

#[derive(Debug)]
pub struct SiteInfo {
    pub pages: usize,
    pub assets: usize,
}

pub fn inspect_site(directory: &Path) -> Result<SiteInfo, String> {
    match fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err("site directory must not be a symlink".into())
        }
        Ok(metadata) if !metadata.is_dir() => return Err("site path is not a directory".into()),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err("site directory is missing".into())
        }
        Err(_) => return Err("site directory could not be inspected".into()),
    }
    let report = yami_node::authoring::check(directory)
        .map_err(|error| format!("site validation failed: {error}"))?;
    Ok(SiteInfo {
        pages: report.pages.len(),
        assets: report.assets.len(),
    })
}

/// A brief bind is only performed when the operator explicitly requests it.
/// No accept loop or persistent listener is created.
pub fn inspect_listener(config: &Config, enabled: bool) -> (Status, String) {
    let Some(address) = config.listen else {
        return (
            Status::NotRun,
            "no listener is configured (outbound-only relay host or browser profile)".into(),
        );
    };
    if !enabled {
        return (
            Status::NotRun,
            format!(
                "configured listener {address}; pass --check-listeners to briefly test whether this address is local and free (no accept loop)"
            ),
        );
    }
    match TcpListener::bind(address) {
        Ok(listener) => {
            drop(listener);
            (
                Status::Pass,
                format!(
                    "configured listener {address} is assigned to this machine and was free; no accept loop was started. External reachability is NOT VERIFIED"
                ),
            )
        }
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => (
            Status::NotRun,
            format!(
                "port {address} is already in use; a service may be running, but ownership cannot be proven by this check"
            ),
        ),
        Err(error) if error.kind() == io::ErrorKind::AddrNotAvailable => (
            Status::Fail,
            format!("{address} is not assigned to any local interface"),
        ),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => (
            Status::Fail,
            format!("permission denied while checking {address}; run with the service account"),
        ),
        Err(_) => (
            Status::Fail,
            format!("{address} could not be checked because the local socket failed"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn config(text: &str) -> Config {
        Config::parse(text).unwrap()
    }

    #[test]
    fn cross_field_checks_refuse_identity_and_state_inside_the_site() {
        let base = format!(
            "yaminet-config 1\nrole node\nnetwork loopback\nidentity {}\nstate {}\nsite {}\nlisten 127.0.0.1:7443\ndht yes\nadvertise 127.0.0.1:7443\n",
            if cfg!(windows) { "C:\\keys" } else { "/keys" },
            if cfg!(windows) { "C:\\state" } else { "/state" },
            if cfg!(windows) { "C:\\site" } else { "/site" },
        );
        assert!(cross_field_checks(&config(&base)).is_ok());
        let inside = base.replace(
            if cfg!(windows) { "C:\\keys" } else { "/keys" },
            if cfg!(windows) {
                "C:\\site\\keys"
            } else {
                "/site/keys"
            },
        );
        assert!(cross_field_checks(&config(&inside)).is_err());
    }

    #[test]
    fn missing_identity_is_reported_without_creating_a_replacement() {
        let directory =
            std::env::temp_dir().join(format!("yami-testnet-missing-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let error = inspect_identity(&directory).unwrap_err();
        assert!(error.contains("missing"));
        assert!(!directory.exists());
    }

    #[test]
    fn malformed_identity_is_rejected() {
        let directory =
            std::env::temp_dir().join(format!("yami-testnet-malformed-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        // Keep valid private-file permissions while corrupting the contents,
        // so a permissive umask cannot make the permission gate mask this check.
        NodeIdentity::create(&directory).unwrap();
        let mut key = fs::File::create(directory.join(KEY_FILE)).unwrap();
        key.write_all(b"not a key").unwrap();
        drop(key);
        let error = inspect_identity(&directory).unwrap_err();
        assert!(error.contains("malformed") || error.contains("wrong length"));
        let _ = fs::remove_dir_all(&directory);
    }

    #[cfg(unix)]
    #[test]
    fn unsafe_permissions_are_rejected_and_fixable_by_the_operator() {
        use std::os::unix::fs::PermissionsExt;
        let directory =
            std::env::temp_dir().join(format!("yami-testnet-perms-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        NodeIdentity::create(&directory).unwrap();
        fs::set_permissions(directory.join(KEY_FILE), fs::Permissions::from_mode(0o644)).unwrap();
        let error = inspect_identity(&directory).unwrap_err();
        assert!(error.contains("too open"));
        fs::set_permissions(directory.join(KEY_FILE), fs::Permissions::from_mode(0o600)).unwrap();
        let info = inspect_identity(&directory).unwrap();
        assert_eq!(info.key_mode_ok, Some(true));
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn listener_check_is_opt_in_and_distinguishes_in_use_from_available() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = listener.local_addr().unwrap();
        let text = format!(
            "yaminet-config 1\nrole node\nnetwork loopback\nidentity {}\nstate {}\nsite {}\nlisten {endpoint}\n",
            if cfg!(windows) { "C:\\keys" } else { "/keys" },
            if cfg!(windows) { "C:\\state" } else { "/state" },
            if cfg!(windows) { "C:\\site" } else { "/site" },
        );
        let config = config(&text);
        assert_eq!(inspect_listener(&config, false).0, Status::NotRun);
        assert_eq!(inspect_listener(&config, true).0, Status::NotRun);
        drop(listener);
        assert_eq!(inspect_listener(&config, true).0, Status::Pass);
    }
}
