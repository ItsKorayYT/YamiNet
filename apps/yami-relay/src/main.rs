use std::{
    error::Error,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use yami_identity::NodeIdentity;
use yami_relay::{HostEvent, Limits, Relay, Route};
const USAGE: &str = "Reverse relay (explicit public opt-in; experimental, no anonymity)\n\
yami-relay serve <loopback-IP:port> --identity <relay-key-directory>\n\
yami-relay serve <specific-IPv4:port> --identity <relay-key-directory> --public --advertise <public-IPv4:port>\n\
yami-relay host <relay-ID> <loopback-IP:port> <site-directory> --identity <host-key-directory>\n\
yami-relay fetch <relay-ID> <loopback-IP:port> <destination-ID> <document-key>\n\
Create persistent identities first with yami-node identity new --dir <directory>.\n\
Host opens outbound connections only. Relay authentication and destination authentication are separate.";
fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let values: Vec<_> = args.iter().map(String::as_str).collect();
    let stop = Arc::new(AtomicBool::new(false));
    let signal = stop.clone();
    match values.as_slice() {
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
