//! Configuration adapters reuse the existing CLI/lifecycle and peer store.
use std::io;
use yami_config::{Config, Role};
pub fn prepare_bootstraps(config: &Config) -> io::Result<()> {
    if config.bootstrap.is_empty() {
        return Ok(());
    }
    let mut peers = crate::peers::PeerStore::open(&config.state)?;
    for contact in &config.bootstrap {
        if let Some(old) = peers.entries().iter().find(|p| p.id == contact.id) {
            if old.address != contact.endpoint.into() {
                return Err(io::Error::new(io::ErrorKind::InvalidInput,"bootstrap pin conflicts with persisted endpoint; remove it explicitly before changing it"));
            }
        } else {
            peers.add(contact.id, contact.endpoint.into())?;
        }
    }
    Ok(())
}
pub fn node_args(config: &Config) -> io::Result<Vec<String>> {
    config.expect_role(Role::Node)?;
    let key = config
        .identity
        .as_ref()
        .ok_or_else(|| io::Error::other("missing identity"))?;
    // Configured unattended startup never silently creates a replacement identity.
    yami_identity::NodeIdentity::load(key).map_err(io::Error::other)?;
    crate::Site::open(
        config
            .site
            .as_ref()
            .ok_or_else(|| io::Error::other("missing site"))?,
    )?;
    prepare_bootstraps(config)?;
    let mut args = Vec::new();
    let mut push = |key: &str, value: String| {
        args.push(key.to_string());
        args.push(value);
    };
    if config.relays.is_empty() {
        push(
            "serve-dir",
            config
                .listen
                .ok_or_else(|| io::Error::other("missing listener"))?
                .to_string(),
        );
        args.push(config.site.as_ref().unwrap().display().to_string());
    } else {
        push(
            "host-relay",
            config.site.as_ref().unwrap().display().to_string(),
        );
    }
    args.extend(["--identity".into(), key.display().to_string()]);
    if config.relays.is_empty() {
        args.extend([
            "--tls".into(),
            "--workers".into(),
            config.workers.to_string(),
        ]);
        if config.public_listen {
            args.push("--public".into());
        }
        if config.yns {
            args.extend([
                "--yns-dir".into(),
                config.state.join("yns").display().to_string(),
            ]);
        }
    }
    for route in &config.relays {
        args.extend([
            "--relay".into(),
            format!("{}@{}", route.relay, route.endpoint),
        ]);
    }
    if config.dht || !config.relays.is_empty() {
        if config.relays.is_empty() {
            args.extend(["--dht".into(), "--dht-maintain".into()]);
        }
        args.extend([
            "--dht-dir".into(),
            config.state.display().to_string(),
            "--dht-renew-secs".into(),
            config.renew_secs.to_string(),
            "--dht-health-secs".into(),
            config.health_secs.to_string(),
        ]);
        if config.network == yami_dht::Network::Loopback {
            args.push("--dht-local".into());
        }
        for endpoint in &config.advertised {
            args.extend(["--dht-endpoint".into(), endpoint.to_string()]);
        }
    }
    Ok(args)
}
