//! Explicit bounded checks of locally configured pins, never discovered content.
use std::{
    io,
    time::{Duration, Instant},
};
use yami_config::{Config, Role};
use yami_identity::NodeIdentity;
pub fn run(config: &Config, connect: bool) -> io::Result<()> {
    println!(
        "Configuration role {:?}; network {:?}; state {}",
        config.role,
        config.network,
        config.state.display()
    );
    if let Some(site) = &config.site {
        let report = crate::authoring::check(site)?;
        println!("Local site directory: {}; valid pages {}; PNG assets {}. Directory is local operator information; redact paths before sharing reports.", site.display(), report.pages.len(), report.assets.len());
    }
    println!(
        "Hosting mode: {}",
        if !config.relays.is_empty() {
            "outbound relay; no home inbound listener"
        } else if config.public_listen {
            "explicit public listener"
        } else {
            "local loopback listener"
        }
    );
    if let Some(listen) = config.listen {
        println!("Configured listener: {listen}; public opt-in {}. This is configuration, not proof of a running or externally reachable socket.", config.public_listen);
    }
    println!("Firewall requirements: clients/relay hosts need outbound TCP to the exact configured peer and relay ports. A public relay/DHT listener needs explicit inbound TCP at its configured port on both OS and provider firewalls. No UDP, router or firewall changes are performed.");
    match crate::maintenance::read_status(&crate::maintenance::status_path(&config.state, config.network), config.network) {
        Ok(status) => println!("DHT snapshot: identity {}; observed {}; publication {:?}; expires {}; latest connection class {:?}. A cached advertisement is not external reachability proof.", status.id, status.observed, status.last_publication, status.expires, status.last_error),
        Err(_) => println!("DHT publication: no readable maintenance snapshot; not verified."),
    }
    if !config.relays.is_empty() {
        match crate::relay_host::read_health(&config.state) {
            Ok(health) => println!("Relay registration snapshot: identity {}; observed {}; registered {}/{}; failures {}; latest success {}. Snapshot may be stale; no external reachability proof.", health.id, health.observed, health.registered, health.configured, health.failures, health.last_success),
            Err(_) => println!("Relay registration: no readable local health snapshot."),
        }
        println!("Relay registration: not verified by preflight or TLS-only probes. Inspect live host registration logs and perform an independent end-to-end document/PNG probe.");
    }
    let local = if let Some(path) = &config.identity {
        let identity = NodeIdentity::load(path).map_err(io::Error::other)?;
        println!(
            "Persistent public identity: {} (loaded successfully; key bytes are never printed)",
            identity.id()
        );
        Some(*identity.id())
    } else {
        None
    };
    println!(
        "Configured bootstrap pins: {}; relay pins: {}; public listener opt-in: {}",
        config.bootstrap.len(),
        config.relays.len(),
        config.public_listen
    );
    let mut pins = config
        .relays
        .iter()
        .map(|r| (r.relay, std::net::SocketAddr::V4(r.endpoint)))
        .chain(config.bootstrap.iter().map(|p| (p.id, p.endpoint.into())))
        .collect::<Vec<_>>();
    if let Some(id) = local {
        if config.relays.is_empty() {
            if config.advertised.is_empty() {
                if let Some(listen) = config.listen {
                    pins.push((id, listen));
                }
            } else {
                pins.extend(config.advertised.iter().map(|e| (id, (*e).into())));
            }
        }
    }
    if config.role == Role::Relay {
        match yami_config::status::RelayStatus::read(&config.state){Ok(status)=>println!("Local relay snapshot: observed {}; running sample {}; connections {}; registrations {}; tunnels {}; forwarded {} bytes (not external reachability proof)",status.observed,status.running,status.connections,status.registrations,status.tunnels,status.forwarded),Err(_)=>println!("No readable local relay status snapshot")}
    }
    if !connect {
        println!("Read-only local check complete. Pass --connect to check up to eight explicitly configured TLS pins within fifteen seconds. No document or discovery requests were sent.");
        return Ok(());
    }
    if pins.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no explicit TLS pins are configured for connection diagnostics",
        ));
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut failed = 0;
    let mut attempted = 0;
    for (id, endpoint) in pins.iter().take(8) {
        let budget = Duration::from_secs(3).min(deadline.saturating_duration_since(Instant::now()));
        if budget.is_zero() {
            break;
        }
        attempted += 1;
        match ynp_tls::TlsTransport::new(*id)?.connect_with_timeout(*endpoint,budget){
            Ok(connection) if connection.protection()==ynp::Protection::Tls13PinnedServer=>println!("TLS pin authenticated: {id} at {endpoint}. Registration and document retrieval were not tested."),
            Ok(_)=>{failed+=1;println!("TLS protection rejected: {id} at {endpoint}");}
            Err(error)=>{failed+=1;let reason=if error.get_ref().and_then(|e|e.downcast_ref::<ynp_tls::IdentityMismatch>()).is_some(){"identity mismatch"}else if matches!(error.kind(),io::ErrorKind::TimedOut|io::ErrorKind::WouldBlock){"deadline or admission limit"}else{"unreachable, rejected or TLS handshake failed"};println!("TLS check failed: {id} at {endpoint}: {reason}; no plaintext fallback");}
        }
    }
    println!(
        "TLS checks: {attempted} attempted, {failed} failed, {} not attempted (bounded budget).",
        pins.len().saturating_sub(attempted)
    );
    if failed > 0 || attempted < pins.len() {
        return Err(io::Error::other(
            "one or more configured TLS pins failed or exceeded the diagnostic budget",
        ));
    }
    Ok(())
}
