use std::{
    error::Error,
    fs::File,
    io::{self, Read},
    net::SocketAddr,
    path::{Path, PathBuf},
    thread,
};
use yami_identity::{default_dir, NodeId, NodeIdentity, KEY_FILE};
use yami_node::{
    serve_concurrently, serve_once, sign_site, site_manifest, verify_site, Reply, Site, Store,
    SIGNATURE_FILE,
};
use ynp::{DocumentData, LocalListener, LoopbackTcp};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const USAGE: &str = "yami-node demo
yami-node serve <loopback-IP:port> <key> <document.ylang> <style.ysl>
yami-node serve-dir <loopback-IP:port> <site-directory> [--workers 1..32] [--identity <identity-directory>]
yami-node fetch <loopback-IP:port> <key>
yami-node identity new [--dir <identity-directory>]
yami-node identity show [--dir <identity-directory>]
yami-node identity inspect <node-id>
yami-node identity sign-site <site-directory> [--dir <identity-directory>]
yami-node identity verify-site <site-directory> <node-id>
EXPERIMENTAL: plaintext and unauthenticated; not secure for sensitive use.";
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
const DEFAULT_WORKERS: usize = 4;
fn serve_dir(
    address: SocketAddr,
    root: &str,
    workers: usize,
    identity: Option<PathBuf>,
) -> Result<()> {
    let site = open_site(root)?;
    if let Some(dir) = identity {
        let (identity, created) = NodeIdentity::load_or_create(&dir)
            .map_err(|error| format!("node identity in {}: {error}", dir.display()))?;
        let action = if created { "Created" } else { "Loaded" };
        eprintln!("{action} node identity {}", identity.id());
        // Checked once at startup; the site may still change while serving.
        match verify_site(&site, identity.id()) {
            Ok(()) => eprintln!("{SIGNATURE_FILE}: valid for the current pages"),
            Err(error) => eprintln!("{SIGNATURE_FILE}: {error}"),
        }
    }
    let pages = site.pages()?;
    let listener = LocalListener::bind(address)?;
    let address = listener.local_addr()?;
    // Pages and the stylesheet are read on every request; edits need no restart.
    eprintln!("Serving site {root} via YNP on {address} with {workers} workers");
    if pages.is_empty() {
        eprintln!("No .ylang pages yet; add some and they are served immediately.");
    }
    for key in &pages {
        eprintln!("  ynp://{address}/{key}");
    }
    serve_concurrently(&listener, &site, workers, &|outcome| match outcome {
        Ok(Reply::Document(key)) => eprintln!("Served {key}"),
        Ok(Reply::NotFound(key)) => eprintln!("NotFound {key}"),
        Err(error) => eprintln!("Connection rejected: {error}"),
    })?;
    Ok(())
}
fn open_site(root: &str) -> Result<Site> {
    Site::open(root).map_err(|error| format!("cannot open site {root}: {error}").into())
}
fn serve_dir_options(options: &[String]) -> Result<(usize, Option<PathBuf>)> {
    let (mut workers, mut identity) = (None, None);
    for pair in options.chunks(2) {
        match pair {
            [flag, count] if flag == "--workers" && workers.is_none() => {
                workers = Some(count.parse()?)
            }
            [flag, dir] if flag == "--identity" && identity.is_none() => {
                identity = Some(PathBuf::from(dir))
            }
            _ => return Err(USAGE.into()),
        }
    }
    Ok((workers.unwrap_or(DEFAULT_WORKERS), identity))
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
    eprintln!("{}", ynp::WARNING);
    match args[0].as_str() {
        "demo" if args.len() == 1 => demo(),
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
            let (workers, identity) = serve_dir_options(&args[3..])?;
            serve_dir(address, &args[2], workers, identity)
        }
        "fetch" if args.len() == 3 => {
            let doc = ynp::fetch(&LoopbackTcp, args[1].parse()?, &args[2])?;
            // Print data with escaped controls; fetching does not interpret it.
            for line in doc.source.lines() {
                println!("{}", line.escape_debug());
            }
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("yami-node: {error}");
        std::process::exit(1);
    }
}
