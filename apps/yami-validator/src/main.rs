use eyre::{eyre, Result};
use std::{path::PathBuf, time::Duration};
use yami_chain_consensus::{
    inspect_with_genesis, run_node_with_storage, status_json, CrashPoint, NodeOptions,
    StorageOptions,
};
const HELP:&str="yami-validator 0.2 — NOT production-ready; no real money\n\nIsolated configurable-committee Malachite prototype. Public development keys; loopback only.\nAll YAMI payments and value-bearing protocol actions are disabled.\n\nUsage:\n  yami-validator run --dir PATH --index N [--base-port 28100] [--height 10] [--timeout 60]\n  yami-validator status --dir PATH\n  yami-validator inspect --dir PATH\n\nrun starts one independent process, binding only 127.0.0.1:base-port+index.\nLaunch the scheduled public development identities with separate stores.\n--committee-plan 1:4,5:7,9:5 pins sizes at activation heights; default 1:4.\nSupply the same plan for run/status/inspect; changing it refuses the existing store.\nstatus/inspect replay and authenticate durable finalized history under an exclusive\nstore lock; stop that validator before inspecting it. No wallet, public RPC or mint.\n\nTest workloads: --test-transactions 0..64; zero preserves the original workload.\nLab timing: --start-at-ms UNIX_MS (at most 60s ahead); --round-timeout-ms 800..60000.\nLocal topology: --sparse-peers true uses authenticated ring/chord neighbors.\nLoopback fault proxy: --peer-base-port PORT changes only outbound loopback ports.\nIsolated labs: init-isolated --dir NEW_PATH --endpoints PRIVATE_IP:PORT,...\nUse --isolated-network PATH for run/status/inspect. Roster is pinned locally;\nonly signed, short-lived private IPv4 endpoint hints become dial targets.\nMount only the own identity-N.bin and public roster/bootstrap files per node.\nNever expose lab listeners or development keys to public networks.\nStorage: default --storage-mode archive preserves full history.\nOpt-in --storage-mode pruned --checkpoint-interval 64 retains a certified compact base.\nOffline: export-checkpoint --dir PATH --out NEW_FILE; prune --dir PATH.\nFresh import: import-checkpoint --dir FRESH --snapshot FILE --anchor HEIGHT:BLOCK:STATE.\nObtain the anchor independently of the snapshot supplier; never restore a used identity\nwithout its original WAL, signing reservations and safety evidence.\n--max-store-bytes BYTES sets the refusal budget (default 18 GB, 64 MiB reserve).\nOptional --linger SECONDS serves certified history at the terminal demo height.\nTest fault injection: --crash-after proposal|vote|before-commit|epoch (run only).\nThe process exits 86 once per directory at the selected durable boundary.\nPreserve the entire directory, including signing records and WAL, when restarting.\n";
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    // Diagnostics are opt-in to keep benchmark stdout machine-readable.
    if std::env::var_os("YAMI_VALIDATOR_LOG").is_some() {
        tracing_subscriber::fmt()
            .with_env_filter(std::env::var("YAMI_VALIDATOR_LOG").unwrap_or_default())
            .with_writer(std::io::stderr)
            .with_ansi(false)
            .init();
    }
    if let Err(error) = run().await {
        eprintln!("yami-validator: {error:#}");
        std::process::exit(1);
    }
}
async fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() || args == ["--help"] || args == ["-h"] {
        print!("{HELP}");
        return Ok(());
    }
    let command = args[0].as_str();
    if command == "announce-isolated" {
        if args.len() != 7 || args[1] != "--dir" || args[3] != "--index" || args[5] != "--endpoint"
        {
            return Err(eyre!(
                "announce-isolated --dir CONFIG --index N --endpoint PRIVATE_IP:PORT"
            ));
        }
        return yami_chain_consensus::network::announce_isolated_endpoint(
            std::path::Path::new(&args[2]),
            args[4].parse()?,
            args[6].parse()?,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs(),
        );
    }
    if command == "init-isolated" {
        if args.len() != 5 || args[1] != "--dir" || args[3] != "--endpoints" {
            return Err(eyre!(
                "init-isolated --dir NEW_PATH --endpoints PRIVATE_IP:PORT,..."
            ));
        }
        let endpoints = args[4]
            .split(',')
            .map(str::parse)
            .collect::<std::result::Result<Vec<std::net::SocketAddrV4>, _>>()?;
        return yami_chain_consensus::network::initialize_isolated_network(
            std::path::Path::new(&args[2]),
            &endpoints,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs(),
        );
    }
    if ![
        "run",
        "status",
        "inspect",
        "export-checkpoint",
        "import-checkpoint",
        "prune",
        "storage-info",
    ]
    .contains(&command)
    {
        return Err(eyre!("unknown command; use --help"));
    }
    let mut dir = None;
    let mut output = None;
    let mut snapshot = None;
    let mut storage_fault = None;
    let mut anchor = None;
    let mut storage_mode = "archive".to_owned();
    let mut checkpoint_interval = 64;
    let mut storage_limits = yami_chain_storage::StorageLimits::default();
    let mut isolated = None;
    let mut index = None;
    let mut base_port = 28100;
    let mut height = 10;
    let mut peer_base_port = None;
    let mut test_transactions = 0;
    let mut sparse_peers = false;
    let mut start_at = None;
    let mut round_timeout = Duration::from_millis(800);
    let mut timeout = 60;
    let mut linger = 0;
    let mut crash = None;
    let mut plan = vec![(1, 4)];
    let mut seen = std::collections::BTreeSet::new();
    for pair in args[1..].chunks(2) {
        if pair.len() != 2 || !seen.insert(pair[0].as_str()) {
            return Err(eyre!("missing or duplicate option; use --help"));
        }
        let value = &pair[1];
        match pair[0].as_str() {
            "--dir" => dir = Some(PathBuf::from(value)),
            "--storage-crash"
                if ["prune", "import-checkpoint", "export-checkpoint"].contains(&command) =>
            {
                if !["base", "unlink", "export-temp"].contains(&value.as_str()) {
                    return Err(eyre!("storage-crash: base|unlink|export-temp"));
                }
                storage_fault = Some(value.clone());
            }
            "--out" if command == "export-checkpoint" => output = Some(PathBuf::from(value)),
            "--snapshot" if command == "import-checkpoint" => snapshot = Some(PathBuf::from(value)),
            "--anchor" if command == "import-checkpoint" => anchor = Some(parse_anchor(value)?),
            "--storage-mode" if command == "run" => {
                if !["archive", "pruned"].contains(&value.as_str()) {
                    return Err(eyre!("storage-mode must be archive or pruned"));
                }
                storage_mode = value.clone();
            }
            "--checkpoint-interval" if command == "run" => checkpoint_interval = value.parse()?,
            "--max-store-bytes" if command == "run" => storage_limits.max_bytes = value.parse()?,
            "--isolated-network" => isolated = Some(PathBuf::from(value)),
            "--committee-plan" => {
                plan = value
                    .split(',')
                    .map(|item| {
                        let (height, count) = item
                            .split_once(':')
                            .ok_or_else(|| eyre!("committee plan: HEIGHT:SIZE,..."))?;
                        Ok((height.parse()?, count.parse()?))
                    })
                    .collect::<Result<Vec<_>>>()?;
            }
            "--index" => index = Some(value.parse()?),
            "--base-port" if command == "run" => base_port = value.parse()?,
            "--height" if command == "run" => height = value.parse()?,
            "--peer-base-port" if command == "run" => peer_base_port = Some(value.parse()?),
            "--round-timeout-ms" if command == "run" => {
                round_timeout = Duration::from_millis(value.parse()?)
            }
            "--start-at-ms" if command == "run" => {
                start_at = Some(
                    std::time::UNIX_EPOCH
                        .checked_add(Duration::from_millis(value.parse()?))
                        .ok_or_else(|| eyre!("startup time overflow"))?,
                )
            }
            "--sparse-peers" if command == "run" => {
                sparse_peers = match value.as_str() {
                    "true" => true,
                    "false" => false,
                    _ => return Err(eyre!("sparse-peers must be true or false")),
                }
            }
            "--test-transactions" if command == "run" => test_transactions = value.parse()?,
            "--timeout" if command == "run" => timeout = value.parse()?,
            "--linger" if command == "run" => linger = value.parse()?,
            "--crash-after" if command == "run" => {
                crash = Some(match value.as_str() {
                    "proposal" => CrashPoint::Proposal,
                    "vote" => CrashPoint::Vote,
                    "before-commit" => CrashPoint::BeforeCommit,
                    "epoch" => CrashPoint::BeforeEpoch,
                    _ => return Err(eyre!("unknown crash point")),
                })
            }
            _ => return Err(eyre!("unknown option for {command}; use --help")),
        }
    }
    let directory = dir.ok_or_else(|| eyre!("--dir is required"))?;
    let isolated_network = isolated
        .as_ref()
        .map(|path| {
            if plan != [(1, 4)] {
                return Err(eyre!(
                    "isolated roster cannot be combined with committee-plan"
                ));
            }
            yami_chain_consensus::network::IsolatedNetwork::load_with_store(
                path,
                index.unwrap_or_else(|| {
                    std::fs::read(directory.join("validator-index.bin"))
                        .ok()
                        .and_then(|b| b.first().copied())
                        .unwrap_or(0)
                }),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs(),
                &directory,
            )
        })
        .transpose()?;
    let genesis = isolated_network
        .as_ref()
        .map(|n| n.genesis().clone())
        .unwrap_or(yami_chain::Genesis::development_schedule(&plan)?);
    if [
        "export-checkpoint",
        "import-checkpoint",
        "prune",
        "storage-info",
    ]
    .contains(&command)
    {
        let journal = yami_chain_storage::Journal::open(&directory)?;
        if command == "import-checkpoint" {
            let bytes = yami_chain_storage::read_bounded(
                &snapshot.ok_or_else(|| eyre!("--snapshot required"))?,
                yami_chain_storage::MAX_COMPACT_BASE_BYTES,
            )?;
            let state = yami_chain_consensus::import_compact_checkpoint_observed(&journal,&genesis,&bytes,anchor.ok_or_else(||eyre!("--anchor HEIGHT:BLOCK:STATE required; obtain independently of snapshot peer"))?, |stage| storage_crash(&storage_fault,stage))?;
            println!("{}", status_json(&state));
        } else if command == "export-checkpoint" {
            let checkpoint = yami_chain_consensus::export_compact_checkpoint(&journal, &genesis)?;
            let path = output.ok_or_else(|| eyre!("--out required"))?;
            yami_chain_storage::publish_checkpoint_export_observed(
                &path,
                checkpoint.bytes(),
                || {
                    if storage_fault.as_deref() == Some("export-temp") {
                        std::process::exit(86);
                    }
                },
            )?;
            let a = checkpoint.anchor();
            println!("{{\"height\":{},\"block\":\"{}\",\"state_root\":\"{}\",\"anchor\":\"{}:{}:{}\",\"bytes\":{},\"trust\":\"obtain anchor independently; peer-provided metadata is not authority\"}}",a.height,a.block,a.state,a.height,a.block,a.state,checkpoint.bytes().len());
        } else if command == "prune" {
            println!(
                "{}",
                status_json(&yami_chain_consensus::prune_certified_before_observed(
                    &journal,
                    &genesis,
                    yami_chain_consensus::replay_certified(&journal, &genesis)?.height(),
                    |stage| storage_crash(&storage_fault, stage)
                )?)
            );
        } else {
            let state = yami_chain_consensus::replay_certified(&journal, &genesis)?;
            let f = journal.storage_footprint(yami_chain_storage::DEFAULT_ENTRY_BUDGET)?;
            println!("{{\"height\":{},\"history_min\":{},\"bytes\":{},\"entries\":{},\"signing_bytes\":{},\"evidence_bytes\":{},\"wal_bytes\":{},\"default_max_bytes\":18000000000}}",state.height(),yami_chain_consensus::history_min_height(&journal,&genesis)?,f.bytes,f.entries,f.signing_bytes,f.evidence_bytes,f.wal_bytes);
        }
        return Ok(());
    }
    let state = if command == "run" {
        if timeout == 0 || timeout > 3600 || linger > 60 {
            return Err(eyre!("timeout must be 1..3600 seconds and linger 0..60"));
        }
        eprintln!("NOT production-ready; no real money. Isolated test networks only; no public deployment.");
        run_node_with_storage(
            NodeOptions {
                directory,
                isolated_network,
                genesis,
                index: index.ok_or_else(|| eyre!("--index is required"))?,
                base_port,
                peer_base_port: peer_base_port.unwrap_or(base_port),
                test_transactions,
                sparse_peers,
                start_at,
                round_timeout,
                target_height: height,
                timeout: Duration::from_secs(timeout),
                linger: Duration::from_secs(linger),
                crash,
            },
            StorageOptions {
                checkpoint_interval: (storage_mode == "pruned").then_some(checkpoint_interval),
                storage_limits,
            },
        )
        .await?
    } else {
        inspect_with_genesis(&directory, genesis)?
    };
    println!("{}", status_json(&state));
    Ok(())
}

fn parse_anchor(text: &str) -> Result<yami_chain_consensus::CheckpointAnchor> {
    fn digest(text: &str) -> Result<yami_chain::Digest> {
        if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(eyre!("anchor digest must be 64 hex digits"));
        }
        let mut bytes = [0; 32];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16)?;
        }
        Ok(yami_chain::Digest(bytes))
    }
    let parts: Vec<_> = text.split(':').collect();
    if parts.len() != 3 {
        return Err(eyre!("anchor format HEIGHT:BLOCK:STATE; pin independently"));
    }
    Ok(yami_chain_consensus::CheckpointAnchor {
        height: parts[0].parse()?,
        block: digest(parts[1])?,
        state: digest(parts[2])?,
    })
}

fn storage_crash(fault: &Option<String>, stage: yami_chain_consensus::CompactStage) {
    if matches!(
        (fault.as_deref(), stage),
        (
            Some("base"),
            yami_chain_consensus::CompactStage::BaseDurable
        ) | (
            Some("unlink"),
            yami_chain_consensus::CompactStage::HistoryRecordRemoved
        )
    ) {
        std::process::exit(86);
    }
}
