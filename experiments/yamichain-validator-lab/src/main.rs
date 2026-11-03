//! Loopback-only executable. Never binds a public interface or accepts real keys.
use eyre::{eyre, Result};
use std::{io::Write, net::Ipv4Addr, path::Path, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Semaphore,
};
use yami_chain::{
    codec::{Reader, Writer},
    development_key,
    validators::{AddressPolicy, Announcement, CandidateDirectory},
    Digest,
};
use yami_chain_consensus::{inspect_with_genesis, run_node, status_json, NodeOptions};
use yami_chain_storage::{read_bounded, Journal};
use yamichain_validator_lab::{plan_digest, Plan, MAX_PLAN_BYTES};

fn digest(s: &str) -> Result<Digest> {
    if s.len() != 64 || !s.is_ascii() {
        return Err(eyre!("expected 64 hex characters"));
    }
    let mut b = [0; 32];
    for (i, v) in b.iter_mut().enumerate() {
        *v = u8::from_str_radix(&s[2 * i..2 * i + 2], 16)?;
    }
    Ok(Digest(b))
}
fn load(path: &str, pin: &str) -> Result<(Plan, yami_chain::Genesis)> {
    let bytes = read_bounded(Path::new(path), MAX_PLAN_BYTES)?;
    let plan = Plan::decode(&bytes, digest(pin)?)?;
    let genesis = plan.compile()?;
    Ok((plan, genesis))
}
async fn serve(listener: TcpListener, plan: Arc<Plan>) -> Result<()> {
    let permits = Arc::new(Semaphore::new(8));
    let mut window = tokio::time::Instant::now();
    let mut requests = 0;
    loop {
        let (mut stream, _) = listener.accept().await?;
        if window.elapsed() >= Duration::from_secs(1) {
            window = tokio::time::Instant::now();
            requests = 0;
        }
        requests += 1;
        if requests > 64 {
            continue;
        }
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            continue;
        };
        let plan = plan.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let _ = tokio::time::timeout(Duration::from_secs(1), async {
                let mut request = [0u8; 9];
                stream.read_exact(&mut request).await?;
                let epoch = u64::from_be_bytes(request[..8].try_into().expect("fixed length"));
                let Some(round) = plan.rounds.iter().find(|r| r.epoch == epoch) else {
                    return Err(std::io::Error::other("unknown epoch"));
                };
                let start = usize::from(request[8]) * 16;
                if start >= 128 {
                    return Err(std::io::Error::other("page cap"));
                }
                let mut w = Writer::default();
                let page: Vec<_> = round.announcements.iter().skip(start).take(16).collect();
                w.u32(page.len() as u32);
                for record in page {
                    w.bytes(record).map_err(std::io::Error::other)?;
                }
                stream.write_u32(w.0.len() as u32).await?;
                stream.write_all(&w.0).await?;
                Ok::<_, std::io::Error>(())
            })
            .await;
        });
    }
}
async fn fetch(port: u16, scope: Digest, epoch: u64) -> Result<CandidateDirectory> {
    let mut directory = CandidateDirectory::new(scope, epoch, AddressPolicy::LoopbackSimulation)?;
    for page in 0..8u8 {
        let mut stream = tokio::time::timeout(
            Duration::from_millis(300),
            TcpStream::connect((Ipv4Addr::LOCALHOST, port)),
        )
        .await??;
        let mut request = epoch.to_be_bytes().to_vec();
        request.push(page);
        let payload = tokio::time::timeout(Duration::from_secs(1), async {
            stream.write_all(&request).await?;
            let len = stream.read_u32().await? as usize;
            if len > 8192 {
                return Err(std::io::Error::other("discovery response cap"));
            }
            let mut bytes = vec![0; len];
            stream.read_exact(&mut bytes).await?;
            Ok::<_, std::io::Error>(bytes)
        })
        .await??;
        let mut r = Reader::new(&payload, 8192)?;
        let count = r.u32()? as usize;
        if count > 16 {
            return Err(eyre!("discovery page count"));
        }
        for _ in 0..count {
            directory.apply(Announcement::decode(r.bytes(320)?)?)?;
        }
        r.end()?;
        if count < 16 {
            return Ok(directory);
        }
    }
    Ok(directory)
}
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("validator-lab: {e:#}");
        std::process::exit(1);
    }
}
async fn run() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    if a.first().is_some_and(|s| s == "fixture") && a.len() == 3 {
        let plan = Plan::fixture(a[2].parse()?)?;
        let bytes = serde_json::to_vec_pretty(&plan)?;
        let mut f = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&a[1])?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        println!("{}", plan_digest(&bytes));
        return Ok(());
    }
    if a.len() < 3 {
        return Err(eyre!("isolated loopback simulation: fixture FILE BASE | describe FILE PIN | inspect FILE PIN DIR | run FILE PIN DIR INDEX BASE PEER_BASE DISCOVERY_BASE HEIGHT TIMEOUT EPOCH [START_AT_MS]"));
    }
    let (plan, genesis) = load(&a[1], &a[2])?;
    let indices: Vec<_> = (0..100u8)
        .filter(|i| {
            genesis.has_consensus_key(&development_key("consensus", *i).verifying_key().to_bytes())
        })
        .collect();
    if a[0] == "describe" && a.len() == 3 {
        let committees: Vec<Vec<u8>> = genesis
            .epochs
            .iter()
            .map(|e| {
                (0..100u8)
                    .filter(|i| {
                        e.validators
                            .member(&development_key("consensus", *i).verifying_key().to_bytes())
                            .is_some()
                    })
                    .collect()
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({"genesis":genesis.id().to_string(),"scope":Digest(plan.scope).to_string(),"indices":indices,"committees":committees,"simulation":true})
        );
        return Ok(());
    }
    if a[0] == "inspect" && a.len() == 4 {
        println!(
            "{}",
            status_json(&inspect_with_genesis(Path::new(&a[3]), genesis)?)
        );
        return Ok(());
    }
    if a[0] == "probe" && a.len() == 5 {
        let directory = fetch(a[3].parse()?, Digest(plan.scope), a[4].parse()?).await?;
        println!("{}", directory.commitment());
        return Ok(());
    }
    if a[0] != "run" || !(a.len() == 11 || a.len() == 12) {
        return Err(eyre!("invalid lab command/arity"));
    }
    let directory = std::path::PathBuf::from(&a[3]);
    let index: u8 = a[4].parse()?;
    let base: u16 = a[5].parse()?;
    let peer_base: u16 = a[6].parse()?;
    let discovery_base: u16 = a[7].parse()?;
    let height: u64 = a[8].parse()?;
    let timeout: u64 = a[9].parse()?;
    // Last argument is an explicit freshness pin for the discovery epoch.
    let epoch: u64 = a[10].parse()?;
    let start_at = if let Some(ms) = a.get(11) {
        Some(
            std::time::UNIX_EPOCH
                .checked_add(Duration::from_millis(ms.parse()?))
                .ok_or_else(|| eyre!("startup time"))?,
        )
    } else {
        None
    };
    if !indices.contains(&index)
        || !(1024..=65400).contains(&base)
        || !(1024..=64500).contains(&peer_base)
        || !(1024..=65400).contains(&discovery_base)
        || height == 0
        || height > 1000
        || !(5..=600).contains(&timeout)
        || !plan.rounds.iter().any(|r| r.epoch == epoch)
    {
        return Err(eyre!("lab bounds"));
    }
    // Validate advertised endpoints against the actual fixed loopback topology.
    for round in &plan.rounds {
        for bytes in &round.announcements {
            let record = Announcement::decode(bytes)?;
            let expected = (0..8u8)
                .find(|i| {
                    development_key("consensus", *i).verifying_key().to_bytes() == record.consensus
                })
                .ok_or_else(|| eyre!("unknown fixture identity"))?;
            if record.endpoint.port() != base + u16::from(expected) {
                return Err(eyre!("fixture endpoint mismatch"));
            }
        }
    }
    let mut expected =
        CandidateDirectory::new(Digest(plan.scope), epoch, AddressPolicy::LoopbackSimulation)?;
    for record in &plan
        .rounds
        .iter()
        .find(|r| r.epoch == epoch)
        .ok_or_else(|| eyre!("epoch"))?
        .announcements
    {
        expected.apply(Announcement::decode(record)?)?;
    }
    {
        let journal = Journal::open(&directory)?;
        journal.write_once("simulation-plan.bin", &digest(&a[2])?.0)?;
        match read_bounded(&directory.join(format!("discovery-{epoch:020}.bin")), 65536) {
            Ok(bytes) => {
                CandidateDirectory::restore(
                    &bytes,
                    expected.commitment(),
                    Digest(plan.scope),
                    epoch,
                    AddressPolicy::LoopbackSimulation,
                )?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    let listener =
        TcpListener::bind((Ipv4Addr::LOCALHOST, discovery_base + u16::from(index))).await?;
    let server = tokio::spawn(serve(listener, Arc::new(plan.clone())));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let discovered = loop {
        let mut found = None;
        for peer in indices.iter().filter(|i| **i != index) {
            if let Ok(d) = fetch(discovery_base + u16::from(*peer), Digest(plan.scope), epoch).await
            {
                if d.commitment() == expected.commitment() && !d.page(None, 16)?.is_empty() {
                    found = Some(d);
                    break;
                }
            }
        }
        if let Some(d) = found {
            break d;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(eyre!("no authenticated discovery response"));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    {
        let journal = Journal::open(&directory)?;
        journal.write_once("simulation-plan.bin", &digest(&a[2])?.0)?;
        journal.write_once(
            &format!("discovery-{epoch:020}.bin"),
            &discovered.snapshot(),
        )?;
        // Replay the exact retained snapshot with its independently derived pin.
        CandidateDirectory::restore(
            &read_bounded(&directory.join(format!("discovery-{epoch:020}.bin")), 65536)?,
            discovered.commitment(),
            Digest(plan.scope),
            epoch,
            AddressPolicy::LoopbackSimulation,
        )?;
    }
    println!(
        "{}",
        serde_json::json!({"discovery_verified":true,"candidates":discovered.page(None,16)?.len(),"epoch":epoch})
    );
    let state = run_node(NodeOptions {
        directory,
        genesis,
        index,
        base_port: base,
        peer_base_port: peer_base,
        test_transactions: 0,
        sparse_peers: false,
        start_at,
        round_timeout: Duration::from_millis(800),
        target_height: height,
        timeout: Duration::from_secs(timeout),
        linger: Duration::from_secs(12),
        crash: None,
    })
    .await?;
    server.abort();
    println!("{}", status_json(&state));
    Ok(())
}
