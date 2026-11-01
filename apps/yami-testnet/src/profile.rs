//! Offline, idempotent testnet profile creation and safe removal.
//!
//! `init` writes configuration, identities, a demo site and rendered unit files.
//! It never starts a listener, installs a unit, changes a firewall or contacts a
//! network. `remove` only deletes a directory carrying this module's marker.
use std::{
    fs,
    io::{self, Write},
    net::{Ipv4Addr, SocketAddrV4},
    path::{Component, Path, PathBuf},
};
use yami_identity::{NodeId, NodeIdentity};

use crate::render::{render, write_unit, UnitRole, UnitSpec};

pub const MARKER: &str = ".yaminet-testnet-profile";
const MARKER_TEXT: &str = "yaminet-testnet-profile 1\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Loopback,
    Public {
        bind: Ipv4Addr,
        public: Ipv4Addr,
    },
    /// Website host behind a router: no local bootstrap/relay and no listener.
    Join,
}

impl Mode {
    pub fn is_public(self) -> bool {
        matches!(self, Self::Public { .. } | Self::Join)
    }

    pub fn is_join(self) -> bool {
        matches!(self, Self::Join)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Loopback => "loopback",
            Self::Public { .. } => "public",
            Self::Join => "home",
        }
    }

    fn validate(self) -> Result<(), String> {
        let Self::Public { bind, public } = self else {
            return Ok(());
        };
        let bind_octets = bind.octets();
        if bind.is_unspecified()
            || bind.is_loopback()
            || bind.is_multicast()
            || bind.is_broadcast()
            || bind_octets[0] >= 224
        {
            return Err(
                "--bind-ip must be one specific unicast IPv4 address assigned to the VPS (never 0.0.0.0 or loopback)"
                    .into(),
            );
        }
        if !ynp_tls::is_public_ipv4(public) {
            return Err(
                "--public-ip must be an ordinary public unicast IPv4 address; documentation, private and reserved ranges are refused"
                    .into(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Ports {
    pub bootstrap: u16,
    pub relay: u16,
    pub site: u16,
}

impl Default for Ports {
    fn default() -> Self {
        Self {
            bootstrap: 17444,
            relay: 17550,
            site: 17445,
        }
    }
}

impl Ports {
    pub fn validate(self) -> Result<Self, String> {
        if [self.bootstrap, self.relay, self.site].contains(&0) {
            return Err("ports must be between 1 and 65535".into());
        }
        if self.bootstrap == self.relay || self.bootstrap == self.site || self.relay == self.site {
            return Err("bootstrap, relay and site ports must differ".into());
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct JoinPins {
    pub bootstrap: (NodeId, SocketAddrV4),
    pub relay: Option<(NodeId, SocketAddrV4)>,
}

#[derive(Debug, Clone)]
pub struct InitOptions {
    pub profile: PathBuf,
    pub mode: Mode,
    pub relay: bool,
    pub site: bool,
    pub client: bool,
    pub yns: bool,
    pub binary_dir: PathBuf,
    pub user: String,
    pub group: String,
    pub ports: Ports,
    pub join: Option<JoinPins>,
}

#[derive(Debug)]
pub struct InitReport {
    pub profile: PathBuf,
    pub mode: Mode,
    pub bootstrap_id: NodeId,
    pub bootstrap_endpoint: SocketAddrV4,
    pub relay: Option<(NodeId, SocketAddrV4)>,
    pub host: Option<NodeId>,
    pub host_url: Option<String>,
    pub client_config: Option<PathBuf>,
    pub created: usize,
    pub kept: usize,
}

#[derive(Debug)]
pub struct RemoveReport {
    pub removed: usize,
    pub leftover: Vec<PathBuf>,
    pub root_removed: bool,
}

fn absolute(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn clean_path(path: &Path) -> Result<PathBuf, String> {
    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err("profile path must not contain '..'".into());
    }
    let absolute = absolute(path).map_err(|_| "profile path is unavailable")?;
    let parent = absolute
        .parent()
        .ok_or_else(|| "profile needs a parent directory".to_string())?;
    let name = absolute
        .file_name()
        .ok_or_else(|| "profile needs a directory name".to_string())?;
    fs::create_dir_all(parent).map_err(|_| "profile parent directory could not be created")?;
    let parent =
        fs::canonicalize(parent).map_err(|_| "profile parent directory could not be resolved")?;
    Ok(parent.join(name))
}

fn regular_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err("profile must not be a symlink".into())
        }
        Ok(metadata) if !metadata.is_dir() => Err("profile path is not a directory".into()),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("profile directory could not be inspected".into()),
    }
}

fn create_private_dir(path: &Path) -> io::Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "testnet directories must not be symlinks",
            ));
        }
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path)
}

fn open_private(path: &Path, create: bool) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).read(true);
    if create {
        options.create_new(true);
    } else {
        options.truncate(true);
    }
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

fn ensure_profile(root: &Path) -> Result<bool, String> {
    regular_directory(root)?;
    let marker = root.join(MARKER);
    match marker.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err("profile marker must not be a symlink".into())
        }
        Ok(metadata) if !metadata.is_file() => {
            return Err("profile marker must be a regular file".into())
        }
        Ok(_) => {
            let text = fs::read_to_string(&marker)
                .map_err(|_| "profile marker could not be read".to_string())?;
            if text != MARKER_TEXT {
                return Err("profile marker is not a supported testnet profile".into());
            }
            return Ok(false);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err("profile marker could not be inspected".into()),
    }
    if root.exists() {
        let mut entries =
            fs::read_dir(root).map_err(|_| "profile directory could not be listed".to_string())?;
        if entries.next().is_some() {
            return Err(
                "directory is not empty and has no testnet marker; choose a new profile directory"
                    .into(),
            );
        }
    }
    create_private_dir(root).map_err(|_| "profile directory could not be created".to_string())?;
    let mut marker_file = open_private(&marker, true).map_err(|error| {
        format!(
            "profile marker {} could not be written: {error}",
            marker.display()
        )
    })?;
    marker_file
        .write_all(MARKER_TEXT.as_bytes())
        .and_then(|()| marker_file.sync_all())
        .map_err(|_| "profile marker could not be written".to_string())?;
    Ok(true)
}

/// Reproduce exactly, or refuse. Configuration and unit files are never
/// silently rewritten after an operator has inspected or edited them.
fn ensure_file(path: &Path, text: &str) -> Result<bool, String> {
    if let Ok(metadata) = path.symlink_metadata() {
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "{} must not be a symlink; inspect the profile",
                path.display()
            ));
        }
        let existing =
            fs::read_to_string(path).map_err(|_| format!("{} is unreadable", path.display()))?;
        if existing == text {
            return Ok(false);
        }
        return Err(format!(
            "{} already exists with different content; inspect it or choose a new profile",
            path.display()
        ));
    }
    if let Some(parent) = path.parent() {
        create_private_dir(parent)
            .map_err(|_| format!("{} could not be created", parent.display()))?;
    }
    let mut file =
        open_private(path, true).map_err(|_| format!("{} could not be written", path.display()))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|_| format!("{} could not be written", path.display()))?;
    Ok(true)
}

fn write_generated(path: &Path, text: &str, mode: u32) -> Result<(), String> {
    #[cfg(not(unix))]
    let _ = mode;
    if let Ok(existing) = fs::read_to_string(path) {
        if existing == text {
            return Ok(());
        }
    }
    if let Some(parent) = path.parent() {
        create_private_dir(parent)
            .map_err(|_| format!("{} could not be created", parent.display()))?;
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, mode);
    let mut file = options
        .open(&temporary)
        .map_err(|_| format!("{} could not be written", path.display()))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|_| format!("{} could not be written", path.display()))?;
    drop(file);
    fs::rename(&temporary, path)
        .map_err(|_| format!("{} could not be replaced", path.display()))?;
    Ok(())
}

fn add(created: &mut usize, kept: &mut usize, was_created: bool) {
    if was_created {
        *created += 1;
    } else {
        *kept += 1;
    }
}

fn ensure_identity(path: &Path) -> Result<(NodeId, bool), String> {
    regular_directory(path)?;
    create_private_dir(path).map_err(|_| "identity directory could not be created".to_string())?;
    let (identity, created) = NodeIdentity::load_or_create(path).map_err(|error| {
        format!(
            "identity in {} could not be loaded or created: {error}",
            path.display()
        )
    })?;
    Ok((*identity.id(), created))
}

fn ensure_site(path: &Path) -> Result<bool, String> {
    if path.exists() {
        crate::config::inspect_site(path)?;
        return Ok(false);
    }
    let parent = path
        .parent()
        .ok_or_else(|| "site needs a parent directory".to_string())?;
    create_private_dir(parent).map_err(|_| "site parent could not be created".to_string())?;
    yami_node::authoring::new_site(path, "personal")
        .map_err(|error| format!("demo site could not be created: {error}"))?;
    Ok(true)
}

fn pin(id: NodeId, endpoint: SocketAddrV4) -> String {
    format!("{id}@{endpoint}")
}

fn config_path(root: &Path, component: &str) -> PathBuf {
    root.join(component).join(format!("{component}.conf"))
}

fn port_for(mode: Mode, address: u16) -> SocketAddrV4 {
    let ip = match mode {
        Mode::Loopback => Ipv4Addr::LOCALHOST,
        Mode::Public { bind, .. } => bind,
        Mode::Join => Ipv4Addr::LOCALHOST,
    };
    SocketAddrV4::new(ip, address)
}

fn advertised_for(mode: Mode, address: u16) -> SocketAddrV4 {
    match mode {
        Mode::Loopback | Mode::Join => SocketAddrV4::new(Ipv4Addr::LOCALHOST, address),
        Mode::Public { public, .. } => SocketAddrV4::new(public, address),
    }
}

fn bootstrap_config(
    mode: Mode,
    identity: &Path,
    state: &Path,
    site: &Path,
    port: u16,
    yns: bool,
) -> String {
    format!(
        "yaminet-config 1\nrole node\nnetwork {network}\npublic-listen {public}\nidentity {identity}\nstate {state}\nsite {site}\nlisten {listen}\nadvertise {advertise}\ndht yes\n{naming}workers 4\nrenew-secs 21600\nhealth-secs 300\n",
        naming = if yns { "yns yes\n" } else { "" },
        network = if mode.is_public() { "public" } else { "loopback" },
        public = if mode.is_public() { "yes" } else { "no" },
        identity = identity.display(),
        state = state.display(),
        site = site.display(),
        listen = port_for(mode, port),
        advertise = advertised_for(mode, port),
    )
}

fn relay_config(mode: Mode, identity: &Path, state: &Path, port: u16) -> String {
    match mode {
        Mode::Loopback => format!(
            "yaminet-config 1\nrole relay\nnetwork loopback\nidentity {}\nstate {}\nlisten {}\n",
            identity.display(),
            state.display(),
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)
        ),
        Mode::Public { bind, .. } => format!(
            "yaminet-config 1\nrole relay\nnetwork public\npublic-listen yes\nidentity {}\nstate {}\nlisten {}\nadvertise {}\n",
            identity.display(),
            state.display(),
            SocketAddrV4::new(bind, port),
            advertised_for(mode, port)
        ),
        Mode::Join => unreachable!("home profiles do not create a relay"),
    }
}

#[allow(clippy::too_many_arguments)]
fn host_config(
    mode: Mode,
    identity: &Path,
    state: &Path,
    site: &Path,
    bootstrap: (NodeId, SocketAddrV4),
    relay: Option<(NodeId, SocketAddrV4)>,
    site_port: u16,
) -> String {
    let mut text = format!(
        "yaminet-config 1\nrole node\nnetwork {}\nidentity {}\nstate {}\nsite {}\n",
        if mode.is_public() {
            "public"
        } else {
            "loopback"
        },
        identity.display(),
        state.display(),
        site.display()
    );
    match relay {
        Some((id, endpoint)) => {
            text.push_str(&format!("relay {}\n", pin(id, endpoint)));
            text.push_str(&format!("bootstrap {}\n", pin(bootstrap.0, bootstrap.1)));
        }
        None => {
            text.push_str(&format!(
                "public-listen {}\nlisten {}\nadvertise {}\ndht yes\nbootstrap {}\n",
                if mode.is_public() { "yes" } else { "no" },
                port_for(mode, site_port),
                advertised_for(mode, site_port),
                pin(bootstrap.0, bootstrap.1)
            ));
        }
    }
    text.push_str("renew-secs 21600\nhealth-secs 300\n");
    text
}

fn client_config(
    mode: Mode,
    state: &Path,
    bootstrap: (NodeId, SocketAddrV4),
    relay: bool,
) -> String {
    format!(
        "yaminet-config 1\nrole browser\nnetwork {}\nstate {}\nbootstrap {}\nrelay-fallback {}\n",
        if mode.is_public() {
            "public"
        } else {
            "loopback"
        },
        state.display(),
        pin(bootstrap.0, bootstrap.1),
        if relay { "yes" } else { "no" }
    )
}

fn env_value(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn env_line(key: &str, value: impl AsRef<str>) -> String {
    format!("{key}={}\n", env_value(value.as_ref()))
}

fn init_join(options: &InitOptions, root: &Path) -> Result<InitReport, String> {
    let join = options
        .join
        .ok_or_else(|| "--home needs bootstrap and relay pins".to_string())?;
    if !options.site {
        return Err("--home needs --site to create a website profile".into());
    }
    if options.yns {
        return Err("--home is outbound-only and cannot run a direct YNS provider".into());
    }
    if options.relay {
        return Err(
            "--home does not create a local relay; pin an existing relay with --relay-pin".into(),
        );
    }
    let Some(relay) = join.relay else {
        return Err(
            "--home needs --relay-pin: an outbound website host behind a router requires an existing relay"
                .into(),
        );
    };
    let mut created = 0;
    let mut kept = 0;
    let host_dir = root.join("host");
    let identity = host_dir.join("identity");
    let state = host_dir.join("state");
    let site = host_dir.join("site");
    let (host_id, identity_created) = ensure_identity(&identity)?;
    create_private_dir(&state)
        .map_err(|_| "host state directory could not be created".to_string())?;
    add(&mut created, &mut kept, identity_created);
    add(&mut created, &mut kept, ensure_site(&site)?);
    add(
        &mut created,
        &mut kept,
        ensure_file(
            &config_path(root, "host"),
            &host_config(
                Mode::Join,
                &identity,
                &state,
                &site,
                join.bootstrap,
                Some(relay),
                options.ports.site,
            ),
        )?,
    );
    let host_url = format!("yami://{host_id}/home");
    let mut client_path = None;
    if options.client {
        let client_state = root.join("client").join("state");
        create_private_dir(&client_state)
            .map_err(|_| "client state directory could not be created".to_string())?;
        let path = config_path(root, "client");
        add(
            &mut created,
            &mut kept,
            ensure_file(
                &path,
                &client_config(Mode::Join, &client_state, join.bootstrap, true),
            )?,
        );
        client_path = Some(path);
    }

    let mut spec = UnitSpec::new(UnitRole::Host, config_path(root, "host"));
    spec.binary = options.binary_dir.join("yami-node");
    spec.user = options.user.clone();
    spec.group = options.group.clone();
    let unit = render(&spec)?;
    let unit_path = root.join("units").join(UnitRole::Host.unit_name());
    add(
        &mut created,
        &mut kept,
        write_unit(&unit_path, &unit, false)
            .map_err(|error| format!("{}: {error}", unit_path.display()))?,
    );

    let mut addresses = String::new();
    addresses.push_str("# YamiNet testnet home-host profile - public identifiers only.\n");
    addresses.push_str("# Source with: . ./addresses.env\n");
    addresses.push_str(&env_line("YAMINET_MODE", "home"));
    addresses.push_str(&env_line("YAMINET_PROFILE", root.display().to_string()));
    addresses.push_str(&env_line("BOOTSTRAP_ID", join.bootstrap.0.to_string()));
    addresses.push_str(&env_line(
        "BOOTSTRAP_ENDPOINT",
        join.bootstrap.1.to_string(),
    ));
    addresses.push_str(&env_line("RELAY_ID", relay.0.to_string()));
    addresses.push_str(&env_line("RELAY_ENDPOINT", relay.1.to_string()));
    addresses.push_str(&env_line("HOST_ID", host_id.to_string()));
    addresses.push_str(&env_line("HOST_URL", host_url.clone()));
    addresses.push_str(&env_line(
        "HOST_CONFIG",
        config_path(root, "host").display().to_string(),
    ));
    addresses.push_str(&env_line("HOST_MODE", "relay"));
    if let Some(path) = &client_path {
        addresses.push_str(&env_line("CLIENT_CONFIG", path.display().to_string()));
    }
    write_generated(&root.join("addresses.env"), &addresses, 0o644)?;

    let mut readme = String::new();
    readme.push_str("YamiNet testnet home-host profile\n================================\n\n");
    readme.push_str("This profile makes outbound connections only. It starts no listener and\n");
    readme.push_str("needs no router port forwarding.\n\n");
    readme.push_str(&format!("Bootstrap endpoint: {}\n", join.bootstrap.1));
    readme.push_str(&format!("Relay endpoint: {}\n", relay.1));
    readme.push_str(&format!("Website ID: {host_id}\n"));
    readme.push_str(&format!("Website address: {host_url}\n\n"));
    readme.push_str(
        "Next:\n1. Review host/host.conf and units/*.service.\n2. Install and start only the host unit on this computer.\n3. Open the website address from an independent Yami Browser client with relay fallback enabled.\n4. Run preflight on the independent client; it must report the exact destination and relay identities.\n",
    );
    write_generated(&root.join("README.txt"), &readme, 0o644)?;

    Ok(InitReport {
        profile: root.to_path_buf(),
        mode: Mode::Join,
        bootstrap_id: join.bootstrap.0,
        bootstrap_endpoint: join.bootstrap.1,
        relay: Some(relay),
        host: Some(host_id),
        host_url: Some(host_url),
        client_config: client_path,
        created,
        kept,
    })
}

pub fn init(options: &InitOptions) -> Result<InitReport, String> {
    if options.profile.as_os_str().is_empty() {
        return Err("profile directory is required".into());
    }
    if !options.binary_dir.is_absolute() {
        return Err("--binary-dir must be an absolute path".into());
    }
    options.mode.validate()?;
    let ports = options.ports.validate()?;
    let root = clean_path(&options.profile)?;
    ensure_profile(&root)?;
    if options.mode.is_join() {
        return init_join(options, &root);
    }
    if options.join.is_some() {
        return Err("join pins are only valid with --home".into());
    }
    let mut created = 0;
    let mut kept = 0;

    let bootstrap_dir = root.join("bootstrap");
    let bootstrap_identity = bootstrap_dir.join("identity");
    let bootstrap_state = bootstrap_dir.join("state");
    let bootstrap_site = bootstrap_dir.join("site");
    let (bootstrap_id, identity_created) = ensure_identity(&bootstrap_identity)?;
    create_private_dir(&bootstrap_state)
        .map_err(|_| "bootstrap state directory could not be created".to_string())?;
    add(&mut created, &mut kept, identity_created);
    add(&mut created, &mut kept, ensure_site(&bootstrap_site)?);
    add(
        &mut created,
        &mut kept,
        ensure_file(
            &config_path(&root, "bootstrap"),
            &bootstrap_config(
                options.mode,
                &bootstrap_identity,
                &bootstrap_state,
                &bootstrap_site,
                ports.bootstrap,
                options.yns,
            ),
        )?,
    );
    let bootstrap_endpoint = if options.mode.is_public() {
        advertised_for(options.mode, ports.bootstrap)
    } else {
        port_for(options.mode, ports.bootstrap)
    };

    let mut relay = None;
    if options.relay {
        let relay_dir = root.join("relay");
        let identity = relay_dir.join("identity");
        let state = relay_dir.join("state");
        let (relay_id, identity_created) = ensure_identity(&identity)?;
        create_private_dir(&state)
            .map_err(|_| "relay state directory could not be created".to_string())?;
        add(&mut created, &mut kept, identity_created);
        add(
            &mut created,
            &mut kept,
            ensure_file(
                &config_path(&root, "relay"),
                &relay_config(options.mode, &identity, &state, ports.relay),
            )?,
        );
        let endpoint = if options.mode.is_public() {
            advertised_for(options.mode, ports.relay)
        } else {
            port_for(options.mode, ports.relay)
        };
        relay = Some((relay_id, endpoint));
    }

    let mut host = None;
    let mut host_url = None;
    if options.site {
        let host_dir = root.join("host");
        let identity = host_dir.join("identity");
        let state = host_dir.join("state");
        let site = host_dir.join("site");
        let (host_id, identity_created) = ensure_identity(&identity)?;
        create_private_dir(&state)
            .map_err(|_| "host state directory could not be created".to_string())?;
        add(&mut created, &mut kept, identity_created);
        add(&mut created, &mut kept, ensure_site(&site)?);
        add(
            &mut created,
            &mut kept,
            ensure_file(
                &config_path(&root, "host"),
                &host_config(
                    options.mode,
                    &identity,
                    &state,
                    &site,
                    (bootstrap_id, bootstrap_endpoint),
                    relay,
                    ports.site,
                ),
            )?,
        );
        host = Some(host_id);
        host_url = Some(format!("yami://{host_id}/home"));
    }

    let mut client_path = None;
    if options.client {
        let client_dir = root.join("client");
        let state = client_dir.join("state");
        create_private_dir(&state)
            .map_err(|_| "client state directory could not be created".to_string())?;
        let path = config_path(&root, "client");
        add(
            &mut created,
            &mut kept,
            ensure_file(
                &path,
                &client_config(
                    options.mode,
                    &state,
                    (bootstrap_id, bootstrap_endpoint),
                    relay.is_some(),
                ),
            )?,
        );
        client_path = Some(path);
    }

    let binary_dir = &options.binary_dir;
    let mut components = vec![(UnitRole::Bootstrap, "bootstrap")];
    if options.relay {
        components.push((UnitRole::Relay, "relay"));
    }
    if options.site {
        components.push((UnitRole::Host, "host"));
    }
    for (role, component) in components {
        let config = config_path(&root, component);
        let mut spec = UnitSpec::new(role, config);
        spec.binary = binary_dir.join(match role {
            UnitRole::Relay => "yami-relay",
            _ => "yami-node",
        });
        spec.user = options.user.clone();
        spec.group = options.group.clone();
        let unit = render(&spec)?;
        let unit_path = root.join("units").join(role.unit_name());
        add(
            &mut created,
            &mut kept,
            write_unit(&unit_path, &unit, false)
                .map_err(|error| format!("{}: {error}", unit_path.display()))?,
        );
    }

    let mut addresses = String::new();
    addresses.push_str("# YamiNet testnet profile - public identifiers and operator paths only.\n");
    addresses.push_str("# Source with: . ./addresses.env\n");
    addresses.push_str(&env_line("YAMINET_MODE", options.mode.as_str()));
    addresses.push_str(&env_line("YAMINET_PROFILE", root.display().to_string()));
    addresses.push_str(&env_line("BOOTSTRAP_ID", bootstrap_id.to_string()));
    addresses.push_str(&env_line(
        "BOOTSTRAP_ENDPOINT",
        bootstrap_endpoint.to_string(),
    ));
    addresses.push_str(&env_line("BOOTSTRAP_PORT", ports.bootstrap.to_string()));
    addresses.push_str(&env_line(
        "BOOTSTRAP_CONFIG",
        config_path(&root, "bootstrap").display().to_string(),
    ));
    if options.yns {
        addresses.push_str(&env_line(
            "YNS_PROVIDER_PIN",
            pin(bootstrap_id, bootstrap_endpoint),
        ));
        addresses.push_str(&env_line(
            "YNS_STATE",
            bootstrap_state.join("yns").display().to_string(),
        ));
    }
    if let Some((id, endpoint)) = relay {
        addresses.push_str(&env_line("RELAY_ID", id.to_string()));
        addresses.push_str(&env_line("RELAY_ENDPOINT", endpoint.to_string()));
        addresses.push_str(&env_line("RELAY_PORT", ports.relay.to_string()));
        addresses.push_str(&env_line(
            "RELAY_CONFIG",
            config_path(&root, "relay").display().to_string(),
        ));
    }
    if let Some(id) = host {
        addresses.push_str(&env_line("HOST_ID", id.to_string()));
        addresses.push_str(&env_line("HOST_URL", host_url.clone().unwrap_or_default()));
        addresses.push_str(&env_line(
            "HOST_CONFIG",
            config_path(&root, "host").display().to_string(),
        ));
        addresses.push_str(&env_line(
            "HOST_MODE",
            if relay.is_some() { "relay" } else { "direct" },
        ));
        addresses.push_str(&env_line("SITE_PORT", ports.site.to_string()));
    }
    if let Some(path) = &client_path {
        addresses.push_str(&env_line("CLIENT_CONFIG", path.display().to_string()));
    }
    write_generated(&root.join("addresses.env"), &addresses, 0o644)?;

    let mut readme = String::new();
    readme.push_str("YamiNet testnet profile\n========================\n\n");
    readme.push_str("This directory was created by `yami-testnet init`.\n");
    readme.push_str("Nothing is running and no firewall or service was changed.\n\n");
    readme.push_str(&format!("Mode: {}\n", options.mode.as_str()));
    readme.push_str(&format!("Bootstrap ID: {bootstrap_id}\n"));
    readme.push_str(&format!("Bootstrap endpoint: {bootstrap_endpoint}\n"));
    if let Some((id, endpoint)) = relay {
        readme.push_str(&format!("Relay ID: {id}\nRelay endpoint: {endpoint}\n"));
    }
    if let Some(id) = host {
        readme.push_str(&format!("Website ID: {id}\n"));
        readme.push_str(&format!(
            "Website address: {}\n",
            host_url.as_deref().unwrap_or_default()
        ));
    }
    readme.push_str(
        "\nNext:\n1. Review every *.conf file and every file in units/.\n2. See docs/public-testnet.md for firewall approval and service installation.\n3. Run `yami-testnet check-config <config>` before starting anything.\n4. Run `yami-testnet preflight ...` from a separate client for end-to-end checks.\n\nNever share node.key. Back up identities before deleting this profile.\n",
    );
    write_generated(&root.join("README.txt"), &readme, 0o644)?;

    Ok(InitReport {
        profile: root,
        mode: options.mode,
        bootstrap_id,
        bootstrap_endpoint,
        relay,
        host,
        host_url,
        client_config: client_path,
        created,
        kept,
    })
}

fn marker_for(root: &Path) -> Result<(), String> {
    let marker = root.join(MARKER);
    match fs::symlink_metadata(&marker) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err("profile marker must not be a symlink; refusing to remove".into())
        }
        Ok(metadata) if !metadata.is_file() => {
            return Err("profile marker must be a regular file; refusing to remove".into())
        }
        Ok(_) => {}
        Err(_) => {
            return Err("profile marker is missing; refusing to remove this directory".to_string())
        }
    }
    let text = fs::read_to_string(&marker)
        .map_err(|_| "profile marker could not be read; refusing to remove".to_string())?;
    if text != MARKER_TEXT {
        return Err("profile marker is not a supported testnet profile; refusing to remove".into());
    }
    Ok(())
}

fn home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn unsafe_root(root: &Path) -> bool {
    let mut components = root.components();
    let depth = components.clone().count();
    if depth < 3 {
        return true;
    }
    if components.any(|component| component == Component::ParentDir) {
        return true;
    }
    let protected = [
        "/", "/etc", "/usr", "/var", "/var/lib", "/home", "/root", "/tmp", "/opt",
    ];
    if protected.iter().any(|path| root == Path::new(path)) {
        return true;
    }
    home_directory().is_some_and(|home| root == home)
}

pub fn remove(profile: &Path, confirm: bool, purge_data: bool) -> Result<RemoveReport, String> {
    if !confirm {
        return Err("removal requires --confirm after reviewing every path".into());
    }
    if !purge_data {
        return Err(
            "removal deletes identities and the demo site; pass --purge-data after making a backup"
                .into(),
        );
    }
    let absolute = absolute(profile).map_err(|_| "profile path is unavailable")?;
    match fs::symlink_metadata(&absolute) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err("profile must not be a symlink".into())
        }
        Ok(_) => {}
        Err(_) => return Err("profile directory does not exist or cannot be resolved".into()),
    }
    let root = fs::canonicalize(&absolute)
        .map_err(|_| "profile directory does not exist or cannot be resolved".to_string())?;
    if unsafe_root(&root) {
        return Err("profile path is too broad or protected; refusing removal".into());
    }
    marker_for(&root)?;

    let known = [
        "bootstrap",
        "relay",
        "host",
        "client",
        "units",
        "addresses.env",
        "README.txt",
    ];
    let mut removed = 0;
    for name in known {
        let path = root.join(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                fs::remove_file(&path)
                    .map_err(|_| format!("{} could not be removed", path.display()))?;
                removed += 1;
            }
            Ok(_) => {
                fs::remove_dir_all(&path)
                    .map_err(|_| format!("{} could not be removed", path.display()))?;
                removed += 1;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(format!("{} could not be inspected", path.display())),
        }
    }
    let marker = root.join(MARKER);
    fs::remove_file(&marker).map_err(|_| "profile marker could not be removed".to_string())?;
    removed += 1;
    let leftover: Vec<PathBuf> = fs::read_dir(&root)
        .map_err(|_| "profile directory could not be listed".to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    let root_removed = if leftover.is_empty() {
        fs::remove_dir(&root).map_err(|_| "profile directory could not be removed".to_string())?;
        true
    } else {
        false
    };
    Ok(RemoveReport {
        removed,
        leftover,
        root_removed,
    })
}

/// Re-read the profile's generated values so scripts can act without guessing.
pub fn bootstrap_config_for(root: &Path) -> PathBuf {
    config_path(root, "bootstrap")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Loaded;
    use yami_config::Config;

    fn base(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "yami-testnet-profile-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        path
    }

    fn options(path: &Path) -> InitOptions {
        let binary_dir = if cfg!(windows) {
            std::env::temp_dir().join("yaminet-testnet-bin")
        } else {
            PathBuf::from("/opt/yaminet")
        };
        InitOptions {
            profile: path.into(),
            mode: Mode::Loopback,
            relay: true,
            site: true,
            client: true,
            yns: false,
            binary_dir,
            user: "yaminet".into(),
            group: "yaminet".into(),
            ports: Ports::default(),
            join: None,
        }
    }

    #[test]
    fn init_is_idempotent_and_preserves_identities() {
        let path = base("idempotent");
        let first = init(&options(&path)).unwrap();
        let key = fs::read(path.join("bootstrap/identity/node.key")).unwrap();
        let second = init(&options(&path)).unwrap();
        assert_eq!(first.bootstrap_id, second.bootstrap_id);
        assert_eq!(
            fs::read(path.join("bootstrap/identity/node.key")).unwrap(),
            key
        );
        assert!(second.created <= 3, "existing files are kept");
        assert!(path
            .join("units/yaminet-testnet-bootstrap.service")
            .exists());
        assert!(path.join("addresses.env").exists());
        Loaded::load(&path.join("bootstrap/bootstrap.conf"), None).unwrap();
        Loaded::load(&path.join("relay/relay.conf"), None).unwrap();
        Loaded::load(&path.join("host/host.conf"), None).unwrap();
        Loaded::load(&path.join("client/client.conf"), None).unwrap();
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn init_refuses_unowned_nonempty_directories_and_changed_configs() {
        let path = base("refuse");
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("operator-file"), b"keep").unwrap();
        assert!(init(&options(&path)).is_err());
        assert!(path.join("operator-file").exists());
        let _ = fs::remove_dir_all(path);

        let path = base("changed");
        init(&options(&path)).unwrap();
        fs::write(path.join("bootstrap/bootstrap.conf"), b"yaminet-config 1\n").unwrap();
        assert!(init(&options(&path)).is_err());
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn home_init_creates_only_an_outbound_host_and_client() {
        let path = base("home");
        let mut options = options(&path);
        options.mode = Mode::Join;
        options.relay = false;
        options.join = Some(JoinPins {
            bootstrap: (
                *NodeIdentity::generate().unwrap().id(),
                "8.8.8.8:17444".parse().unwrap(),
            ),
            relay: Some((
                *NodeIdentity::generate().unwrap().id(),
                "8.8.8.8:17550".parse().unwrap(),
            )),
        });
        let report = init(&options).unwrap();
        assert_eq!(report.mode.as_str(), "home");
        assert!(path.join("host/host.conf").exists());
        assert!(path.join("client/client.conf").exists());
        assert!(!path.join("bootstrap").exists());
        assert!(!path.join("relay").exists());
        assert!(path.join("units/yaminet-testnet-host.service").exists());
        assert!(!path
            .join("units/yaminet-testnet-bootstrap.service")
            .exists());
        let host = Loaded::load(&path.join("host/host.conf"), None).unwrap();
        assert!(!host.config.public_listen);
        assert!(host.config.listen.is_none());
        assert_eq!(host.config.relays.len(), 1);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn removal_needs_marker_confirmation_and_purge_flag() {
        let path = base("remove");
        init(&options(&path)).unwrap();
        assert!(remove(&path, false, true).is_err());
        assert!(remove(&path, true, false).is_err());
        let report = remove(&path, true, true).unwrap();
        assert!(report.root_removed);
        assert!(!path.exists());
        assert!(remove(&path, true, true).is_err());
    }

    #[test]
    fn removal_refuses_directories_outside_the_marker_protocol() {
        let path = base("nomarker");
        fs::create_dir_all(&path).unwrap();
        assert!(remove(&path, true, true).is_err());
        assert!(path.exists());
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn removal_retains_unrecognized_operator_files() {
        let path = base("leftover");
        init(&options(&path)).unwrap();
        fs::write(path.join("operator-note.txt"), b"keep this").unwrap();
        let report = remove(&path, true, true).unwrap();
        assert!(!report.root_removed);
        assert!(path.exists());
        assert!(path.join("operator-note.txt").exists());
        assert!(report
            .leftover
            .iter()
            .any(|leftover| leftover.ends_with("operator-note.txt")));
        let _ = fs::remove_dir_all(path);
    }

    #[cfg(unix)]
    #[test]
    fn init_and_remove_refuse_symlinked_profiles() {
        use std::os::unix::fs::symlink;
        let real = base("symlink-real");
        let link = base("symlink-link");
        fs::create_dir_all(&real).unwrap();
        symlink(&real, &link).unwrap();
        assert!(init(&options(&link)).is_err());
        assert!(remove(&link, true, true).is_err());
        let _ = fs::remove_file(&link);
        let _ = fs::remove_dir_all(real);
    }

    #[test]
    fn public_init_validates_addresses_and_writes_public_configs() {
        let path = base("public");
        let mut options = options(&path);
        options.mode = Mode::Public {
            bind: "203.0.113.10".parse().unwrap(),
            public: "203.0.113.10".parse().unwrap(),
        };
        assert!(
            init(&options).is_err(),
            "documentation addresses are refused"
        );
        options.mode = Mode::Public {
            bind: "10.0.0.10".parse().unwrap(),
            public: "8.8.8.8".parse().unwrap(),
        };
        let report = init(&options).unwrap();
        assert_eq!(report.mode.as_str(), "public");
        let bootstrap =
            Config::parse(&fs::read_to_string(path.join("bootstrap/bootstrap.conf")).unwrap())
                .unwrap();
        assert!(bootstrap.public_listen);
        assert_eq!(bootstrap.advertised[0].ip().to_string(), "8.8.8.8");
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn public_mode_requires_a_non_wildcard_bind_and_public_advertise() {
        let path = base("validate");
        let mut options = options(&path);
        options.mode = Mode::Public {
            bind: Ipv4Addr::UNSPECIFIED,
            public: "8.8.8.8".parse().unwrap(),
        };
        assert!(init(&options).is_err());
        options.mode = Mode::Public {
            bind: "10.0.0.10".parse().unwrap(),
            public: "10.0.0.11".parse().unwrap(),
        };
        assert!(init(&options).is_err());
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn source_line_quoting_round_trips_shell_special_paths() {
        assert_eq!(env_value("a'b"), "'a'\\''b'");
        assert_eq!(env_value("/var/lib/a b"), "'/var/lib/a b'");
    }
    #[test]
    fn yns_profile_reuses_bootstrap_listener_and_writable_state_without_new_authority() {
        let path = base("yns");
        let mut opts = options(&path);
        opts.yns = true;
        let report = init(&opts).unwrap();
        let config = yami_config::Config::load(path.join("bootstrap/bootstrap.conf")).unwrap();
        assert!(config.yns);
        let args = yami_node::configuration::node_args(&config).unwrap();
        assert!(args.windows(2).any(|pair| pair[0] == "--yns-dir"
            && pair[1] == config.state.join("yns").display().to_string()));
        assert!(
            !yami_config::Config::load(path.join("host/host.conf"))
                .unwrap()
                .yns
        );
        let addresses = fs::read_to_string(path.join("addresses.env")).unwrap();
        assert!(addresses.contains(&format!(
            "YNS_PROVIDER_PIN='{}@{}'",
            report.bootstrap_id, report.bootstrap_endpoint
        )));
        assert!(
            !path.join("bootstrap/state/yns/yns.json").exists(),
            "offline init must not run the naming service"
        );
        assert_eq!(init(&opts).unwrap().bootstrap_id, report.bootstrap_id);
        fs::remove_dir_all(path).unwrap();
    }
}
