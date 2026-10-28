use std::{
    error::Error,
    fs::File,
    io::{self, Read},
    net::SocketAddr,
    thread,
};
use yami_node::{serve_once, Store};
use ynp::{DocumentData, LocalListener, LoopbackTcp};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const USAGE: &str = "yami-node demo\nyami-node serve <loopback-IP:port> <key> <document.ylang> <style.ysl>\nyami-node fetch <loopback-IP:port> <key>\nEXPERIMENTAL: plaintext and unauthenticated; not secure for sensitive use.";
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
fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args == ["--help"] {
        println!("{USAGE}");
        return Ok(());
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
