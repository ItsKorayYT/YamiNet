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
    serve_concurrently, serve_once, sign_site, site_manifest, verify_site, Reply, Site, Store,
    SIGNATURE_FILE,
};
use ynp::{DocumentData, Incoming, LocalListener, LoopbackTcp};
use ynp_tls::{TlsListener, TlsTransport};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const USAGE: &str = "yami-node demo
yami-node tls-demo <identity-directory-1> <identity-directory-2>
yami-node serve <loopback-IP:port> <key> <document.ylang> <style.ysl>
yami-node serve-dir <loopback-IP:port> <site-directory> [--workers 1..32] [--identity <identity-directory> [--tls]]
yami-node fetch <loopback-IP:port> <key>
yami-node fetch <ynps://node-id@loopback-IP:port/key>
yami-node identity new [--dir <identity-directory>]
yami-node identity show [--dir <identity-directory>]
yami-node identity inspect <node-id>
yami-node identity sign-site <site-directory> [--dir <identity-directory>]
yami-node identity verify-site <site-directory> <node-id>
EXPERIMENTAL: ynp:// and commands without --tls are plaintext and unauthenticated.
ynps://, --tls and tls-demo use TLS 1.3 to a pinned node ID. Neither is reviewed or anonymous.";
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
            let listener = TlsListener::bind(address, identity)?;
            let address = listener.local_addr()?;
            eprintln!("Serving site {root} via YNP over TLS 1.3 on {address} as {id}");
            serve_site(&listener, &site, options.workers, &|key| {
                ynp_tls::format_url(&id, address, key)
            })
        }
        _ => {
            let listener = LocalListener::bind(address)?;
            let address = listener.local_addr()?;
            eprintln!("Serving site {root} via YNP on {address} (plaintext)");
            serve_site(&listener, &site, options.workers, &|key| {
                format!("ynp://{address}/{key}")
            })
        }
    }
}
fn serve_site(
    listener: &(impl Incoming + Sync),
    site: &Site,
    workers: usize,
    url: &dyn Fn(&str) -> String,
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
    serve_concurrently(listener, site, workers, &|outcome| match outcome {
        Ok(Reply::Document(key)) => eprintln!("Served {key}"),
        Ok(Reply::NotFound(key)) => eprintln!("NotFound {key}"),
        Err(error) => eprintln!("Connection rejected: {error}"),
    })?;
    Ok(())
}
fn open_site(root: &str) -> Result<Site> {
    Site::open(root).map_err(|error| format!("cannot open site {root}: {error}").into())
}
struct ServeOptions {
    workers: usize,
    identity: Option<PathBuf>,
    tls: bool,
}
fn serve_dir_options(options: &[String]) -> Result<ServeOptions> {
    let (mut workers, mut identity, mut tls) = (None, None, false);
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
            _ => return Err(USAGE.into()),
        }
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
