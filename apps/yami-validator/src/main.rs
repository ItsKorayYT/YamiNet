use eyre::{eyre, Result};
use std::{path::PathBuf, time::Duration};
use yami_chain_consensus::{inspect_with_genesis, run_node, status_json, CrashPoint, NodeOptions};
const HELP:&str="yami-validator 0.2 — NOT production-ready; no real money\n\nIsolated configurable-committee Malachite prototype. Public development keys; loopback only.\nAll YAMI payments and value-bearing protocol actions are disabled.\n\nUsage:\n  yami-validator run --dir PATH --index N [--base-port 28100] [--height 10] [--timeout 60]\n  yami-validator status --dir PATH\n  yami-validator inspect --dir PATH\n\nrun starts one independent process, binding only 127.0.0.1:base-port+index.\nLaunch the scheduled public development identities with separate stores.\n--committee-plan 1:4,5:7,9:5 pins sizes at activation heights; default 1:4.\nSupply the same plan for run/status/inspect; changing it refuses the existing store.\nstatus/inspect replay and authenticate durable finalized history under an exclusive\nstore lock; stop that validator before inspecting it. No wallet, public RPC or mint.\n\nTest workloads: --test-transactions 0..64; zero preserves the original workload.\nLoopback fault proxy: --peer-base-port PORT changes only outbound loopback ports.\nOptional --linger SECONDS serves certified history at the terminal demo height.\nTest fault injection: --crash-after proposal|vote|before-commit|epoch (run only).\nThe process exits 86 once per directory at the selected durable boundary.\nPreserve the entire directory, including signing records and WAL, when restarting.\n";
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
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
    if !["run", "status", "inspect"].contains(&command) {
        return Err(eyre!("unknown command; use --help"));
    }
    let mut dir = None;
    let mut index = None;
    let mut base_port = 28100;
    let mut height = 10;
    let mut peer_base_port = None;
    let mut test_transactions = 0;
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
            "--index" if command == "run" => index = Some(value.parse()?),
            "--base-port" if command == "run" => base_port = value.parse()?,
            "--height" if command == "run" => height = value.parse()?,
            "--peer-base-port" if command == "run" => peer_base_port = Some(value.parse()?),
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
    let genesis = yami_chain::Genesis::development_schedule(&plan)?;
    let state = if command == "run" {
        if timeout == 0 || timeout > 3600 || linger > 60 {
            return Err(eyre!("timeout must be 1..3600 seconds and linger 0..60"));
        }
        eprintln!("NOT production-ready; no real money. Public development keys; loopback only.");
        run_node(NodeOptions {
            directory,
            genesis,
            index: index.ok_or_else(|| eyre!("--index is required"))?,
            base_port,
            peer_base_port: peer_base_port.unwrap_or(base_port),
            test_transactions,
            target_height: height,
            timeout: Duration::from_secs(timeout),
            linger: Duration::from_secs(linger),
            crash,
        })
        .await?
    } else {
        inspect_with_genesis(&directory, genesis)?
    };
    println!("{}", status_json(&state));
    Ok(())
}
