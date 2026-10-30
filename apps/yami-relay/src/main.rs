use std::{
    error::Error,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use yami_identity::NodeIdentity;
use yami_relay::{HostEvent, Limits, Relay, Route};
const USAGE: &str = "yami-relay --config <file> | check-config <file> | status --config <file>\n\
Reverse relay (explicit public opt-in; experimental, no anonymity)\n\
yami-relay serve <loopback-IP:port> --identity <relay-key-directory>\n\
yami-relay serve <specific-IPv4:port> --identity <relay-key-directory> --public --advertise <public-IPv4:port>\n\
yami-relay host <relay-ID> <loopback-IP:port> <site-directory> --identity <host-key-directory>\n\
yami-relay fetch <relay-ID> <loopback-IP:port> <destination-ID> <document-key>\n\
Create persistent identities first with yami-node identity new --dir <directory>.\n\
Host opens outbound connections only. Relay authentication and destination authentication are separate.";
fn configured(config: &yami_config::Config, stop: &AtomicBool) -> Result<(), Box<dyn Error>> {
    config.expect_role(yami_config::Role::Relay)?;
    let identity = Arc::new(NodeIdentity::load(
        config.identity.as_ref().ok_or("missing relay identity")?,
    )?);
    let id = *identity.id();
    let writer = yami_config::status::StatusWriter::open(&config.state)?;
    let endpoint = config.listen.ok_or("missing relay listener")?;
    let relay = if config.public_listen {
        Relay::bind_public(endpoint, identity, Limits::default())?
    } else {
        Relay::bind(endpoint, identity, Limits::default())?
    };
    eprintln!(
        "Relay listening on {} as {id}; explicit public mode {}; advertised {:?}; state {}",
        relay.local_addr()?,
        config.public_listen,
        config.advertised,
        config.state.display()
    );
    relay.run_with_observer(stop, &|stats, running| {
        let status = yami_config::status::RelayStatus {
            id,
            observed: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |t| t.as_secs()),
            running,
            connections: stats.connections,
            registrations: stats.registrations,
            tunnels: stats.tunnels,
            forwarded: stats.forwarded_bytes,
        };
        if writer.write(&status).is_err() {
            eprintln!("Relay diagnostic snapshot could not be saved");
        }
    })?;
    Ok(())
}
fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let values: Vec<_> = args.iter().map(String::as_str).collect();
    let stop = Arc::new(AtomicBool::new(false));
    let signal = stop.clone();
    match values.as_slice() {
        ["check-config", path] => {
            let config = yami_config::Config::load(path)?;
            config.expect_role(yami_config::Role::Relay)?;
            println!(
                "Valid relay configuration; public listener opt-in: {}; state {}",
                config.public_listen,
                config.state.display()
            );
        }
        ["--config", path] => {
            let config = yami_config::Config::load(path)?;
            config.expect_role(yami_config::Role::Relay)?;
            ctrlc::set_handler(move || signal.store(true, Ordering::SeqCst))?;
            configured(&config, &stop)?;
        }
        ["status", "--config", path] => {
            let config = yami_config::Config::load(path)?;
            config.expect_role(yami_config::Role::Relay)?;
            let status = yami_config::status::RelayStatus::read(&config.state)?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs();
            let age = now.saturating_sub(status.observed);
            println!("Relay {}: {} (sample age {}s); connections {}; registrations {}; tunnels {}; forwarded bytes {}. Local diagnostics, not a reachability proof.",status.id,if status.running&&age<=10&&status.observed<=now {"recently running"}else{"stopped or stale"},age,status.connections,status.registrations,status.tunnels,status.forwarded);
        }
        ["--help"] | [] => println!("{USAGE}"),
        ["serve", endpoint, "--identity", directory] => {
            let identity = Arc::new(NodeIdentity::load(directory)?);
            let relay = Relay::bind(endpoint.parse()?, identity.clone(), Limits::default())?;
            ctrlc::set_handler(move || signal.store(true, Ordering::SeqCst))?;
            eprintln!(
                "Relay listening on {} as {} (loopback simulation only)",
                relay.local_addr()?,
                identity.id()
            );
            relay.run(&stop)?;
        }
        ["serve", endpoint, "--identity", directory, "--public", "--advertise", advertised] => {
            let advertised: std::net::SocketAddrV4 = advertised.parse()?;
            if advertised.port() == 0 || !ynp_tls::is_public_ipv4(*advertised.ip()) {
                return Err(
                    "advertised endpoint must be a public unicast IPv4 address and nonzero port"
                        .into(),
                );
            }
            let identity = Arc::new(NodeIdentity::load(directory)?);
            let relay = Relay::bind_public(endpoint.parse()?, identity.clone(), Limits::default())?;
            ctrlc::set_handler(move || signal.store(true, Ordering::SeqCst))?;
            eprintln!("Relay listening on {} as {} (PUBLIC, experimental; independent security review required)", relay.local_addr()?, identity.id());
            eprintln!(
                "Advertised endpoint {advertised}; operator must verify routing and firewall"
            );
            relay.run(&stop)?;
        }
        ["host", relay, endpoint, root, "--identity", directory] => {
            let identity = Arc::new(NodeIdentity::load(directory)?);
            let route = Route {
                relay: relay.parse()?,
                endpoint: endpoint.parse()?,
                destination: *identity.id(),
            };
            let site = yami_node::Site::open(root)?;
            ctrlc::set_handler(move || signal.store(true, Ordering::SeqCst))?;
            yami_relay::run_host(
                route,
                identity.clone(),
                &stop,
                &|connection| yami_node::serve_connection(connection, &site).map(|_| ()),
                &|event| match event {
                    HostEvent::Registered => eprintln!(
                        "Registered outbound host {} at relay {}",
                        identity.id(),
                        route.relay
                    ),
                    HostEvent::Unregistered => {}
                    HostEvent::Served => eprintln!("Served document through end-to-end TLS"),
                    HostEvent::Retrying => eprintln!(
                        "Relay connection ended or unavailable; retrying with bounded backoff"
                    ),
                },
            )?;
        }
        ["fetch", relay, endpoint, destination, key] => {
            let route = Route {
                relay: relay.parse()?,
                endpoint: endpoint.parse()?,
                destination: destination.parse()?,
            };
            let data = route.fetch(key)?;
            eprintln!(
                "Verified destination {} over end-to-end TLS 1.3 through relay {}",
                route.destination, route.relay
            );
            for ch in data.source.chars() {
                if ch == '\n' || ch == '\r' || !ch.is_control() {
                    print!("{ch}");
                } else {
                    print!("{}", ch.escape_default());
                }
            }
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("Relay error: {error}");
        std::process::exit(1);
    }
}
