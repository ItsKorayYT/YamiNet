use std::{
    error::Error,
    fs::File,
    io::{self, Read},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
};
use yami_identity::{default_dir, NodeId, NodeIdentity, KEY_FILE};
use yami_node::{
    serve_once, serve_with_handler_until, sign_site, site_manifest, verify_site, Reply, Site,
    Store, SIGNATURE_FILE,
};
use ynp::{DocumentData, Incoming, LocalListener, LoopbackTcp};
use ynp_tls::{TlsListener, TlsTransport};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const USAGE: &str = "yami-node demo
yami-node tls-demo <identity-directory-1> <identity-directory-2>
yami-node serve <loopback-IP:port> <key> <document.ylang> <style.ysl>
yami-node serve-dir <loopback-IP:port> <site-directory> [--workers 1..32] [--identity <identity-directory> [--tls]]
yami-node serve-dir <private-LAN-IPv4:port> <site-directory> --identity <identity-directory> --tls --lan [--discoverable] [--workers 1..32]
yami-node serve-dir <specific-IPv4:port> <site-directory> --identity <identity-directory> --tls --public [--workers 1..32]
yami-node peer add <node-id> <numeric-IP:port> [--dir <peer-directory>]
yami-node peer list [--dir <peer-directory>]
yami-node peer remove <node-id> [--dir <peer-directory>]
yami-node peer connect <node-id> [--dir <peer-directory>]
yami-node dht bootstrap <node-id> <IPv4:port> [--dir <peer-directory>] [--local]
yami-node dht peers [--dir <peer-directory>] [--local]
yami-node dht status [--dir <peer-directory>] [--local]
yami-node dht lookup <node-id> [--dir <peer-directory>] [--identity <identity-directory>] [--local]
yami-node dht fetch <node-id> <key> [--dir <peer-directory>] [--identity <identity-directory>] [--local]
serve-dir DHT options: --dht [--dht-local] [--dht-dir <peer/state-directory>] [--dht-endpoint <advertised-IPv4:port> ...]
Maintenance opt-in: --dht-maintain [--dht-renew-secs 30..43200] [--dht-health-secs 5..3600] [--dht-endpoints-file <file>]
yami-node fetch <loopback-IP:port> <key>
yami-node fetch <ynps://node-id@numeric-IP:port/key>
yami-node identity new [--dir <identity-directory>]
yami-node identity show [--dir <identity-directory>]
yami-node identity inspect <node-id>
yami-node identity sign-site <site-directory> [--dir <identity-directory>]
yami-node identity verify-site <site-directory> <node-id>
EXPERIMENTAL: ynp:// and commands without --tls are plaintext and unauthenticated.
ynps://, --tls and tls-demo use TLS 1.3 to a pinned node ID. Neither is reviewed or anonymous.
--lan listens on one private IPv4 address (10/8, 172.16/12, 192.168/16) and requires --tls.
--public requires --tls and opts into public clients on ONE specific IPv4 address; never automatic exposure.
--lan and --public are mutually exclusive. --discoverable requires --tls --lan (never --public).
Discovery defaults to 239.255.89.78:39817/UDP; configure YAMINET_DISCOVERY_GROUP/PORT on both computers.
Discovery exposes your public ID and address to LAN observers; it provides no anonymity.";
const IDENTITY_NOTE: &str = "EXPERIMENTAL: node signatures attribute metadata to a key. They do not encrypt, hide or anonymize anything.";
#[cfg(unix)]
const PROTECTION: &str = "owner-only permissions (0600), checked on every load";
#[cfg(not(unix))]
const PROTECTION: &str =
    "inherited from the directory's access control list; not checked (see docs/specs/v0.1/identity.md)";
fn read_file(path: &str, limit: usize) -> io::Result<String> {
    let mut text = String::new();
    File::open(path)?
        .take((limit + 1) as u64)
        .read_to_string(&mut text)?;
    if text.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds protocol limit",
        ));
    }
    Ok(text)
}
fn sample(key: &str, source: &str) -> DocumentData {
    DocumentData {
        key: key.into(),
        source: source.into(),
        style: include_str!("../../../examples/default.ysl").into(),
    }
}
fn demo() -> Result<()> {
    let first = LocalListener::bind("127.0.0.1:0".parse()?)?;
    let second = LocalListener::bind("127.0.0.1:0".parse()?)?;
    let address_one = first.local_addr()?;
    let address_two = second.local_addr()?;
    let welcome = sample("welcome", include_str!("../../../examples/welcome.ylang"));
    let about = sample("about", include_str!("../../../examples/about.ylang"));
    let mut one = Store::default();
    one.insert(welcome.clone())?;
    let mut two = Store::default();
    two.insert(about.clone())?;
    // Each node thread both serves its own document and fetches its peer's.
    let a = thread::spawn(move || -> io::Result<DocumentData> {
        let received = ynp::fetch(&LoopbackTcp, address_two, "about")?;
        serve_once(&first, &one)?;
        Ok(received)
    });
    let b = thread::spawn(move || -> io::Result<DocumentData> {
        serve_once(&second, &two)?;
        ynp::fetch(&LoopbackTcp, address_one, "welcome")
    });
    let received_two = a
        .join()
        .map_err(|_| io::Error::other("node one thread failed"))??;
    let received_one = b
        .join()
        .map_err(|_| io::Error::other("node two thread failed"))??;
    if received_two != about || received_one != welcome {
        return Err("document exchange mismatch".into());
    }
    println!(
        "Node one received about from node two via YNP ({} bytes).",
        received_two.source.len()
    );
    println!(
        "Node two received welcome from node one via YNP ({} bytes).",
        received_one.source.len()
    );
    Ok(())
}
fn load_or_create(dir: &Path) -> Result<(Arc<NodeIdentity>, bool)> {
    let (identity, created) = NodeIdentity::load_or_create(dir)
        .map_err(|error| format!("node identity in {}: {error}", dir.display()))?;
    Ok((Arc::new(identity), created))
}
/// Two nodes with their own persistent identities fetch each other's document
/// over TLS 1.3, each pinning the other's node ID. Then a client that expects
/// node one is sent to node two's address and must be refused.
fn tls_demo(dir_one: &Path, dir_two: &Path) -> Result<()> {
    let (identity_one, created_one) = load_or_create(dir_one)?;
    let (identity_two, created_two) = load_or_create(dir_two)?;
    let (one, two) = (*identity_one.id(), *identity_two.id());
    if one == two {
        return Err("both directories hold the same identity; give each node its own".into());
    }
    let action = |created| if created { "created" } else { "loaded" };
    println!(
        "Node one  {one} ({} in {})",
        action(created_one),
        dir_one.display()
    );
    println!(
        "Node two  {two} ({} in {})",
        action(created_two),
        dir_two.display()
    );
    let first = TlsListener::bind("127.0.0.1:0".parse()?, identity_one)?;
    let second = TlsListener::bind("127.0.0.1:0".parse()?, identity_two)?;
    let (address_one, address_two) = (first.local_addr()?, second.local_addr()?);
    let welcome = sample("welcome", include_str!("../../../examples/welcome.ylang"));
    let about = sample("about", include_str!("../../../examples/about.ylang"));
    let mut store_one = Store::default();
    store_one.insert(welcome.clone())?;
    let mut store_two = Store::default();
    store_two.insert(about.clone())?;
    let (trust_one, trust_two) = (TlsTransport::new(one)?, TlsTransport::new(two)?);
    // As in `demo`, each node thread both serves its own document and fetches
    // its peer's, in an order that cannot deadlock.
    let (received_two, received_one) = thread::scope(|scope| {
        let a = scope.spawn(|| -> io::Result<DocumentData> {
            let received = ynp::fetch(&trust_two, address_two, "about")?;
            serve_once(&first, &store_one)?;
            Ok(received)
        });
        let b = scope.spawn(|| -> io::Result<DocumentData> {
            serve_once(&second, &store_two)?;
            ynp::fetch(&trust_one, address_one, "welcome")
        });
        let join = |node: thread::ScopedJoinHandle<'_, io::Result<DocumentData>>| {
            node.join()
                .map_err(|_| io::Error::other("node thread failed"))?
        };
        Ok::<_, io::Error>((join(a)?, join(b)?))
    })?;
    if received_two != about || received_one != welcome {
        return Err("document exchange mismatch".into());
    }
    println!(
        "Node one received about from {} over TLS 1.3 ({} bytes).",
        ynp_tls::format_url(&two, address_two, "about"),
        received_two.source.len()
    );
    println!(
        "Node two received welcome from {} over TLS 1.3 ({} bytes).",
        ynp_tls::format_url(&one, address_one, "welcome"),
        received_one.source.len()
    );
    let refused = thread::scope(|scope| {
        scope.spawn(|| serve_once(&second, &store_two));
        ynp::fetch(&trust_one, address_two, "about")
    });
    match refused {
        Ok(_) => Err("a client expecting node one accepted node two".into()),
        Err(error) => {
            println!("Refused node two at {address_two} when expecting node one: {error}");
            Ok(())
        }
    }
}
const DEFAULT_WORKERS: usize = 4;
fn serve_dir(address: SocketAddr, root: &str, options: ServeOptions) -> Result<()> {
    if !(1..=yami_node::MAX_WORKERS).contains(&options.workers) {
        return Err("worker count must be 1..32".into());
    }
    if !options.lan && !options.public && !address.ip().is_loopback() {
        return Err(format!(
            "only loopback addresses are allowed without --lan or --public; serving on {address} needs --identity <identity-directory> --tls and either --lan (private only) or --public (explicit remote service)"
        )
        .into());
    }
    if options.lan && !ynp_tls::is_private_lan(address.ip()) {
        return Err(format!(
            "--lan needs one specific private IPv4 address (10.x.x.x, 172.16-31.x.x or 192.168.x.x); {address} is not one, and 0.0.0.0 is never allowed"
        )
        .into());
    }
    if options.public {
        ynp_tls::Scope::Internet.check(address)?;
        if !address.is_ipv4() {
            return Err("--public requires one specific IPv4 address".into());
        }
    }
    if options.dht {
        if options.dht_local
            && (!address.ip().is_loopback() || !address.is_ipv4() || options.public || options.lan)
        {
            return Err(
                "--dht-local requires isolated IPv4 loopback TLS, without --lan/--public".into(),
            );
        }
        if !options.dht_local && !options.public {
            return Err(
                "public DHT requires --tls --public (or explicitly --dht-local for simulation)"
                    .into(),
            );
        }
    }
    let site = open_site(root)?;
    let mut identity = None;
    if let Some(dir) = &options.identity {
        let (loaded, created) = load_or_create(dir)?;
        let action = if created { "Created" } else { "Loaded" };
        eprintln!("{action} node identity {}", loaded.id());
        // Checked once at startup; the site may still change while serving.
        match verify_site(&site, loaded.id()) {
            Ok(()) => eprintln!("{SIGNATURE_FILE}: valid for the current pages"),
            Err(error) => eprintln!("{SIGNATURE_FILE}: {error}"),
        }
        identity = Some(loaded);
    }
    match identity {
        Some(identity) if options.tls => {
            let id = *identity.id();
            let dht_identity = Arc::clone(&identity);
            let listener = if options.public {
                TlsListener::bind_public(address, identity)?
            } else if options.lan {
                TlsListener::bind_lan(address, identity)?
            } else {
                TlsListener::bind(address, identity)?
            };
            let address = listener.local_addr()?;
            let shutdown = yami_node::shutdown::Shutdown::default();
            if options.dht_maintain {
                let signal = shutdown.clone();
                ctrlc::set_handler(move || signal.stop())?;
                listener.set_nonblocking(true)?;
            }
            let mut maintenance_worker = None;
            let dht = if options.dht {
                let network = if options.dht_local {
                    yami_dht::Network::Loopback
                } else {
                    yami_dht::Network::Public
                };
                let dir = options
                    .dht_dir
                    .clone()
                    .map_or_else(yami_node::peers::default_dir, Ok)?;
                let service =
                    yami_dht::Service::open(dht_path(&dir, network, "server"), id, network)?;
                let endpoints = if let Some(path) = &options.dht_endpoints_file {
                    yami_node::maintenance::read_endpoints(path, network)
                        .map_err(|_| "invalid or unavailable DHT endpoint file")?
                } else if options.dht_endpoints.is_empty() {
                    let SocketAddr::V4(endpoint) = address else {
                        return Err("DHT endpoints require IPv4".into());
                    };
                    vec![endpoint]
                } else {
                    options.dht_endpoints.clone()
                };
                let record = if options.dht_maintain {
                    let engine = yami_node::maintenance::Maintainer::new(
                        service.clone(),
                        dht_identity,
                        endpoints,
                        options.maintenance_config,
                    )?;
                    let record = engine.record().clone();
                    maintenance_worker = Some(yami_node::maintenance::Worker::start(
                        engine,
                        dir,
                        options.dht_endpoints_file.clone(),
                        shutdown.clone(),
                    )?);
                    record
                } else {
                    let record = service.prepare_self(&dht_identity, endpoints, service.now()?)?;
                    let seeds = dht_seeds(&dir, network)?
                        .into_iter()
                        .filter(|c| c.id != id)
                        .collect::<Vec<_>>();
                    let publisher = service.clone();
                    let published_record = record.clone();
                    thread::Builder::new().name("dht-startup".into()).spawn(move || {
                        match yami_dht::publish(&publisher, &published_record, &seeds) {
                            Ok(count) => eprintln!("DHT startup publication complete: {count} authenticated bootstrap(s)"),
                            Err(_) => eprintln!("DHT startup publication failed: peer or state unavailable"),
                        }
                    })?;
                    record
                };
                eprintln!("DHT v1 enabled, {:?}; signed sequence {} expires {}. Endpoints are claims until TLS authenticates the node.", network, record.sequence(), record.expires());
                Some(service)
            } else {
                None
            };
            // Keep the guard alive alongside the TLS listener. Without the flag
            // no discovery socket is created, even on an opted-in LAN listener.
            let _discovery = if options.discoverable {
                let SocketAddr::V4(endpoint) = address else {
                    return Err("discovery requires private IPv4".into());
                };
                let interface = ynp_discovery::interface(*endpoint.ip())?;
                let config = ynp_discovery::Config::from_env()?;
                let pages = site.pages()?;
                let home = pages
                    .iter()
                    .find(|key| *key == "home")
                    .or(pages.first())
                    .ok_or("--discoverable needs at least one page to advertise")?;
                let advert = ynp_discovery::Advertisement::new(id, endpoint, home)?;
                let responder = ynp_discovery::Responder::start(interface, config, advert)?;
                eprintln!("Discoverable on {} via {}:{}/UDP (TTL 1). Public ID and address are visible to LAN observers.", endpoint.ip(), config.group, config.port);
                Some(responder)
            } else {
                None
            };
            eprintln!("Serving site {root} via YNP over TLS 1.3 on {address} as {id}");
            if options.public {
                eprintln!("PUBLIC MODE: unauthenticated internet clients may read every page. Experimental, not security-reviewed. Configure any firewall/port forwarding manually. No automatic exposure; DHT requires its separate opt-in.");
            }
            if options.lan {
                eprintln!(
                    "LAN: computers on this private network can connect to {address}; nothing listens on other interfaces. Allow inbound TCP port {} for Private networks only.",
                    address.port()
                );
            }
            let result = serve_site(
                &listener,
                &site,
                options.workers,
                options.public,
                dht.as_ref(),
                &|key| ynp_tls::format_url(&id, address, key),
                &shutdown,
            );
            shutdown.stop();
            if let Some(worker) = &mut maintenance_worker {
                worker.stop();
            }
            result
        }
        _ => {
            let listener = LocalListener::bind(address)?;
            let address = listener.local_addr()?;
            eprintln!("Serving site {root} via YNP on {address} (plaintext)");
            serve_site(
                &listener,
                &site,
                options.workers,
                false,
                None,
                &|key| format!("ynp://{address}/{key}"),
                &yami_node::shutdown::Shutdown::default(),
            )
        }
    }
}
fn serve_site(
    listener: &(impl Incoming + Sync),
    site: &Site,
    workers: usize,
    public: bool,
    dht: Option<&yami_dht::Service>,
    url: &dyn Fn(&str) -> String,
    shutdown: &yami_node::shutdown::Shutdown,
) -> Result<()> {
    let pages = site.pages()?;
    // Pages and the stylesheet are read on every request; edits need no restart.
    eprintln!("Using {workers} workers");
    if pages.is_empty() {
        eprintln!("No .ylang pages yet; add some and they are served immediately.");
    }
    for key in &pages {
        eprintln!("  {}", url(key));
    }
    if let Some(start) = pages.iter().find(|key| *key == "home").or(pages.first()) {
        eprintln!("Open in Yami Browser: {}", url(start));
    }
    let reports = std::sync::Mutex::new((std::time::Instant::now(), 0usize));
    serve_with_handler_until(
        listener,
        workers,
        &|connection| {
            if let Some(dht) = dht {
                yami_node::serve_connection_with_dht(connection, site, dht)
            } else {
                yami_node::serve_connection(connection, site)
            }
        },
        &|outcome| {
            if public {
                let Ok(mut budget) = reports.lock() else {
                    return;
                };
                if budget.0.elapsed() >= std::time::Duration::from_secs(1) {
                    *budget = (std::time::Instant::now(), 0);
                }
                if budget.1 >= 20 {
                    return;
                }
                budget.1 += 1;
            }
            match outcome {
                Ok(Reply::Document(key)) => eprintln!("Served {key}"),
                Ok(Reply::NotFound(key)) => eprintln!("NotFound {key}"),
                Err(error) => eprintln!("Connection rejected: {error}"),
                Ok(Reply::Dht) => eprintln!("Served bounded DHT RPC"),
            }
        },
        shutdown,
    )?;
    Ok(())
}
fn open_site(root: &str) -> Result<Site> {
    Site::open(root).map_err(|error| format!("cannot open site {root}: {error}").into())
}
struct ServeOptions {
    workers: usize,
    identity: Option<PathBuf>,
    tls: bool,
    lan: bool,
    discoverable: bool,
    public: bool,
    dht: bool,
    dht_local: bool,
    dht_dir: Option<PathBuf>,
    dht_endpoints: Vec<std::net::SocketAddrV4>,
    dht_endpoints_file: Option<PathBuf>,
    dht_maintain: bool,
    maintenance_config: yami_node::maintenance::Config,
}
fn serve_dir_options(options: &[String]) -> Result<ServeOptions> {
    let (mut workers, mut identity, mut tls, mut lan) = (None, None, false, false);
    let mut discoverable = false;
    let mut public = false;
    let (mut dht, mut dht_local, mut dht_dir, mut dht_endpoints) = (false, false, None, Vec::new());
    let (mut dht_maintain, mut renew_secs, mut health_secs, mut dht_endpoints_file) =
        (false, None, None, None);
    let mut options = options.iter();
    while let Some(flag) = options.next() {
        match flag.as_str() {
            "--workers" if workers.is_none() => {
                workers = Some(options.next().ok_or(USAGE)?.parse()?)
            }
            "--identity" if identity.is_none() => {
                identity = Some(PathBuf::from(options.next().ok_or(USAGE)?))
            }
            "--tls" if !tls => tls = true,
            "--lan" if !lan => lan = true,
            "--discoverable" if !discoverable => discoverable = true,
            "--public" if !public => public = true,
            "--dht" if !dht => dht = true,
            "--dht-maintain" if !dht_maintain => dht_maintain = true,
            "--dht-renew-secs" if renew_secs.is_none() => {
                renew_secs = Some(options.next().ok_or(USAGE)?.parse()?)
            }
            "--dht-health-secs" if health_secs.is_none() => {
                health_secs = Some(options.next().ok_or(USAGE)?.parse()?)
            }
            "--dht-endpoints-file" if dht_endpoints_file.is_none() => {
                dht_endpoints_file = Some(PathBuf::from(options.next().ok_or(USAGE)?))
            }
            "--dht-local" if !dht_local => dht_local = true,
            "--dht-dir" if dht_dir.is_none() => {
                dht_dir = Some(PathBuf::from(options.next().ok_or(USAGE)?))
            }
            "--dht-endpoint" if dht_endpoints.len() < 4 => {
                dht_endpoints.push(options.next().ok_or(USAGE)?.parse()?)
            }
            _ => return Err(USAGE.into()),
        }
    }
    if !dht_maintain
        && (renew_secs.is_some() || health_secs.is_some() || dht_endpoints_file.is_some())
    {
        return Err("maintenance options require --dht-maintain".into());
    }
    if dht_maintain && !dht {
        return Err("--dht-maintain requires --dht --tls".into());
    }
    if dht_endpoints_file.is_some() && !dht_endpoints.is_empty() {
        return Err("choose --dht-endpoint or --dht-endpoints-file".into());
    }
    let defaults = yami_node::maintenance::Config::default();
    let maintenance_config = yami_node::maintenance::Config {
        renew_secs: renew_secs.unwrap_or(defaults.renew_secs),
        health_secs: health_secs.unwrap_or(defaults.health_secs),
    }
    .validate()?;
    if public && lan {
        return Err("--public and --lan are mutually exclusive".into());
    }
    if dht && !tls {
        return Err("--dht requires --tls and an identity; plaintext DHT is forbidden".into());
    }
    if !dht && (dht_local || dht_dir.is_some() || !dht_endpoints.is_empty()) {
        return Err("DHT options require explicit --dht".into());
    }
    if public && !tls {
        return Err("--public needs --tls; plaintext internet YNP is forbidden".into());
    }
    if discoverable && (!tls || !lan || public) {
        return Err("--discoverable needs --tls --lan and a selected private LAN interface".into());
    }
    if lan && !tls {
        return Err("--lan needs --tls: YNP is never served in plaintext beyond loopback".into());
    }
    if tls && identity.is_none() {
        return Err(
            "--tls needs --identity <identity-directory>: the node proves that identity to clients"
                .into(),
        );
    }
    Ok(ServeOptions {
        workers: workers.unwrap_or(DEFAULT_WORKERS),
        identity,
        tls,
        lan,
        discoverable,
        public,
        dht,
        dht_local,
        dht_dir,
        dht_endpoints,
        dht_endpoints_file,
        dht_maintain,
        maintenance_config,
    })
}
fn identity_dir(options: &[String]) -> Result<PathBuf> {
    match options {
        [] => default_dir()
            .ok_or_else(|| "no default identity directory; pass --dir <identity-directory>".into()),
        [flag, dir] if flag == "--dir" => Ok(dir.into()),
        _ => Err(USAGE.into()),
    }
}
fn load_identity(dir: &Path) -> Result<NodeIdentity> {
    NodeIdentity::load(dir)
        .map_err(|error| format!("node identity in {}: {error}", dir.display()).into())
}
fn peer_command(args: &[String]) -> Result<()> {
    use yami_node::peers::{self, PeerStore};
    let (args, dir) = if args.len() >= 2 && args[args.len() - 2] == "--dir" {
        (
            &args[..args.len() - 2],
            PathBuf::from(&args[args.len() - 1]),
        )
    } else {
        (args, peers::default_dir()?)
    };
    // Validate command shape and user-provided values before opening storage.
    match args {
        [command] if command == "list" => {}
        [command, id] if command == "remove" || command == "connect" => {
            id.parse::<NodeId>()?;
        }
        [command, id, address] if command == "add" => {
            id.parse::<NodeId>()?;
            address.parse::<SocketAddr>()?;
        }
        _ => return Err(USAGE.into()),
    }
    let mut store = PeerStore::open(&dir)?;
    match args {
        [command] if command == "list" => {
            println!("Manual bootstrap peers (status = last attempt, not a live connection). Last success is Unix seconds.");
            for peer in store.entries() {
                println!(
                    "{}\t{}\t{}\t{}",
                    peer.id,
                    peer.address,
                    peer.last_success
                        .map_or_else(|| "never".into(), |s| s.to_string()),
                    peer.status.label()
                );
            }
        }
        [command, id, address] if command == "add" => {
            store.add(id.parse()?, address.parse()?)?;
            println!("Added unverified manual peer {id} at {address}. No connection was made.");
        }
        [command, id] if command == "remove" => {
            store.remove(id.parse()?)?;
            println!("Removed peer {id}.");
        }
        [command, id] if command == "connect" => {
            eprintln!("{}", ynp_tls::WARNING);
            store
                .connect(id.parse()?)
                .map_err(|e| format!("Peer authentication/connection failed for {id}: {e}"))?;
            println!("Authenticated node {id} over pinned TLS 1.3. Connection closed; last success saved.");
        }
        _ => unreachable!(),
    }
    Ok(())
}
fn dht_path(dir: &Path, network: yami_dht::Network, role: &str) -> PathBuf {
    let mode = if network == yami_dht::Network::Public {
        "public"
    } else {
        "local"
    };
    dir.join(format!("dht-{mode}-{role}.cache"))
}
fn dht_seeds(dir: &Path, network: yami_dht::Network) -> Result<Vec<yami_dht::Contact>> {
    Ok(yami_node::maintenance::read_seeds(dir, network)?)
}
fn dht_command(args: &[String]) -> Result<()> {
    use yami_dht::{Network, Service};
    let (mut dir, mut identity, mut local, mut values) = (None, None, false, Vec::new());
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dir" if dir.is_none() => dir = Some(PathBuf::from(args.next().ok_or(USAGE)?)),
            "--identity" if identity.is_none() => {
                identity = Some(PathBuf::from(args.next().ok_or(USAGE)?))
            }
            "--local" if !local => local = true,
            flag if flag.starts_with("--") => return Err(USAGE.into()),
            _ => values.push(arg.as_str()),
        }
    }
    let network = if local {
        Network::Loopback
    } else {
        Network::Public
    };
    let dir = dir.map_or_else(yami_node::peers::default_dir, Ok)?;
    match values.as_slice() {
        ["bootstrap", id, endpoint] => {
            let id: NodeId = id.parse()?;
            let endpoint: std::net::SocketAddrV4 = endpoint.parse()?;
            network.check(endpoint)?;
            yami_node::peers::PeerStore::open(&dir)?.add(id, endpoint.into())?;
            println!("Added manual bootstrap pin {id} at {endpoint}; unverified until TLS. No query was sent.");
            Ok(())
        }
        ["status"] => {
            let state = Service::read_snapshot(&dht_path(&dir, network, "server"), network)
                .map_err(|_| {
                    "DHT server state is unavailable; start a DHT node with this directory/network"
                })?;
            let at = yami_dht::now()?;
            let expires = state
                .records()
                .iter()
                .find(|r| r.id() == state.local)
                .map_or(0, |r| r.expires());
            let status = yami_node::maintenance::read_status(
                &yami_node::maintenance::status_path(&dir, network),
                network,
            )
            .ok()
            .filter(|s| s.id == state.local);
            println!(
                "Known peers: {}",
                state
                    .records()
                    .iter()
                    .filter(|r| r.id() != state.local && state.get(r.id(), at).is_some())
                    .count()
            );
            if let Some(status) = status {
                let age = at.saturating_sub(status.observed);
                println!(
                    "Recently verified peers: {} (sample age {}s)",
                    if age < 900 {
                        status.recently_verified
                    } else {
                        0
                    },
                    age
                );
                println!(
                    "Last successful publication: {}",
                    status
                        .last_publication
                        .map_or_else(|| "none recorded".into(), |time| time.to_string())
                );
                println!(
                    "Maintenance worker: {} (last reported state)",
                    if status.running { "running" } else { "stopped" }
                );
                println!(
                    "Last maintenance error: {}",
                    status.last_error.map_or("none", |error| error.message())
                );
            } else {
                println!("Recently verified peers: unknown (no maintenance sample)");
                println!("Last successful publication: unknown");
                println!(
                    "Last maintenance error: unknown (maintenance disabled or status unavailable)"
                );
            }
            println!("Local record expires in: {}s", expires.saturating_sub(at));
            Ok(())
        }
        ["peers"] => {
            let peers = yami_node::peers::PeerStore::open(&dir)?;
            for peer in peers.entries() {
                println!(
                    "bootstrap {} {} manual-status={}",
                    peer.id,
                    peer.address,
                    peer.status.label()
                );
            }
            for role in ["server", "client"] {
                match Service::read_snapshot(&dht_path(&dir, network, role), network) {
                    Ok(state) => {
                        for record in state.records() {
                            if state.get(record.id(), yami_dht::now()?).is_some() {
                                println!("{role} {} sequence={} expires={} signed-location-unconfirmed {:?}", record.id(), record.sequence(), record.expires(), record.endpoints());
                            } else {
                                println!("{role} {} expired; sequence {} retained only to reject rollback", record.id(), record.sequence());
                            }
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
            Ok(())
        }
        ["lookup", id] | ["fetch", id, _] => {
            let target: NodeId = id.parse()?;
            if values[0] == "fetch" {
                ynp::validate_key(values[2])?;
            }
            let identity_dir = identity
                .or_else(default_dir)
                .ok_or("no identity directory; pass --identity")?;
            let (identity, _) = load_or_create(&identity_dir)?;
            let service =
                Service::open(dht_path(&dir, network, "client"), *identity.id(), network)?;
            match Service::read_snapshot(&dht_path(&dir, network, "server"), network) {
                Ok(state) if state.local == *identity.id() => {
                    let at = yami_dht::now()?;
                    let current = service.snapshot()?;
                    let records = state
                        .records()
                        .iter()
                        .filter(|r| state.get(r.id(), at).is_some())
                        // The server snapshot may lag a newer record learned
                        // by this client. Never import a rollback/equivocation.
                        .filter(|r| {
                            current
                                .records()
                                .iter()
                                .find(|old| old.id() == r.id())
                                .is_none_or(|old| r.sequence() > old.sequence() || *r == old)
                        })
                        .cloned()
                        .collect();
                    service.accept_records(records, yami_dht::now()?)?;
                }
                Ok(_) => return Err("DHT server cache belongs to another identity".into()),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            let seeds = dht_seeds(&dir, network)?;
            let found = yami_dht::lookup(&service, target, &seeds)?;
            let record = found.record.ok_or_else(|| format!("DHT lookup found no current signed record after {} query attempt(s), {} failure(s)", found.queries, found.failures))?;
            eprintln!("Signed record for {}: sequence {}, expires {}. Location is not a TLS liveness proof.", record.id(), record.sequence(), record.expires());
            if values[0] == "lookup" {
                for endpoint in record.endpoints() {
                    println!(
                        "{}",
                        ynp_tls::format_url(&record.id(), (*endpoint).into(), "home")
                    );
                }
                return Ok(());
            }
            let key = values[2];
            ynp::validate_key(key)?;
            let mut last = None;
            for endpoint in record.endpoints() {
                record.verify(network, yami_dht::now()?)?;
                network.check(*endpoint)?;
                match TlsTransport::new(target)?
                    .connect_with_timeout((*endpoint).into(), std::time::Duration::from_secs(3))
                    .and_then(|mut connection| ynp::request(&mut *connection, key))
                {
                    Ok(doc) => {
                        eprintln!("Verified node {target} over TLS 1.3 after distributed lookup");
                        print_source(&doc);
                        return Ok(());
                    }
                    Err(error) => last = Some(error),
                }
            }
            Err(last
                .unwrap_or_else(|| io::Error::other("no reachable signed endpoint"))
                .into())
        }
        _ => Err(USAGE.into()),
    }
}
/// Prints only public values and file locations, never key material.
fn describe(dir: &Path, identity: &NodeIdentity) {
    println!("Node ID     {}", identity.id());
    println!("Public key  {}", identity.id().public_key_hex());
    println!(
        "Key file    {} (secret: never share, publish or commit)",
        dir.join(KEY_FILE).display()
    );
    println!("Protection  {PROTECTION}");
}
fn identity(args: &[String]) -> Result<()> {
    let Some((command, rest)) = args.split_first() else {
        return Err(USAGE.into());
    };
    match (command.as_str(), rest) {
        ("new", options) => {
            let dir = identity_dir(options)?;
            let identity = NodeIdentity::create(&dir)
                .map_err(|error| format!("node identity in {}: {error}", dir.display()))?;
            println!("Created a new node identity.");
            describe(&dir, &identity);
        }
        ("show", options) => {
            let dir = identity_dir(options)?;
            describe(&dir, &load_identity(&dir)?);
        }
        ("inspect", [id]) => {
            let id: NodeId = id.parse()?;
            println!("Node ID     {id}");
            println!("Format      version 1: base32(Ed25519 public key || 4-byte checksum)");
            println!("Public key  {}", id.public_key_hex());
        }
        ("sign-site", [root, options @ ..]) => {
            let identity = load_identity(&identity_dir(options)?)?;
            let site = open_site(root)?;
            let signed = sign_site(&site, &identity)?;
            println!(
                "Signed {} pages and the style of {root} as {}",
                site_manifest(&site)?.pages().len(),
                signed.signer
            );
            println!(
                "Wrote {}; sign again after every change.",
                Path::new(root).join(SIGNATURE_FILE).display()
            );
        }
        ("verify-site", [root, id]) => {
            let id: NodeId = id.parse()?;
            verify_site(&open_site(root)?, &id)?;
            println!("Valid: {root} matches {SIGNATURE_FILE} signed by {id}.");
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}
fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args == ["--help"] {
        println!("{USAGE}");
        return Ok(());
    }
    if args[0] == "identity" {
        eprintln!("{IDENTITY_NOTE}");
        return identity(&args[1..]);
    }
    if args[0] == "peer" {
        return peer_command(&args[1..]);
    }
    if args[0] == "dht" {
        return dht_command(&args[1..]);
    }
    let tls = args[0] == "tls-demo"
        || args
            .iter()
            .any(|arg| arg == "--tls" || arg.starts_with(ynp_tls::SCHEME));
    eprintln!("{}", if tls { ynp_tls::WARNING } else { ynp::WARNING });
    match args[0].as_str() {
        "demo" if args.len() == 1 => demo(),
        "tls-demo" if args.len() == 3 => tls_demo(Path::new(&args[1]), Path::new(&args[2])),
        "serve" if args.len() == 5 => {
            let address: SocketAddr = args[1].parse()?;
            let mut store = Store::default();
            store.insert(DocumentData {
                key: args[2].clone(),
                source: read_file(&args[3], ynp::MAX_DOCUMENT)?,
                style: read_file(&args[4], ynp::MAX_STYLE)?,
            })?;
            let listener = LocalListener::bind(address)?;
            eprintln!(
                "Serving one in-memory document via YNP on {}",
                listener.local_addr()?
            );
            loop {
                if let Err(error) = serve_once(&listener, &store) {
                    eprintln!("Connection rejected: {error}");
                }
            }
        }
        "serve-dir" if args.len() >= 3 => {
            let address: SocketAddr = args[1].parse()?;
            serve_dir(address, &args[2], serve_dir_options(&args[3..])?)
        }
        "fetch" if args.len() == 3 => {
            print_source(&ynp::fetch(&LoopbackTcp, args[1].parse()?, &args[2])?);
            Ok(())
        }
        "fetch" if args.len() == 2 => {
            let (id, address, key) = ynp_tls::parse_url(&args[1])?;
            let doc = ynp::fetch(&TlsTransport::new(id)?, address, &key)?;
            eprintln!("Verified node {id} over TLS 1.3");
            print_source(&doc);
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}
/// Print data with escaped controls; fetching does not interpret it.
fn print_source(doc: &DocumentData) {
    for line in doc.source.lines() {
        println!("{}", line.escape_debug());
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("yami-node: {error}");
        std::process::exit(1);
    }
}
