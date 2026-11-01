use eyre::{eyre, Result};
use std::{path::PathBuf, time::Duration};
use yami_chain_consensus::{inspect, run_node, status_json, CrashPoint, NodeOptions};
const HELP:&str="yami-validator 0.1 — NOT production-ready; no real money\n\nIsolated four-validator Malachite prototype. Public development keys; loopback only.\nAll YAMI payments and value-bearing protocol actions are disabled.\n\nUsage:\n  yami-validator run --dir PATH --index 0..3 [--base-port 28100] [--height 10] [--timeout 60]\n  yami-validator status --dir PATH\n  yami-validator inspect --dir PATH\n\nrun starts one independent process, binding only 127.0.0.1:base-port+index.\nLaunch four processes with separate directories and the same base port.\nstatus/inspect replay and authenticate durable finalized history under an exclusive\nstore lock; stop that validator before inspecting it. No wallet, public RPC or mint.\n\nTest fault injection: --crash-after proposal|vote|before-commit (run only).\nThe process exits 86 once per directory at the selected durable boundary.\nPreserve the entire directory, including signing records and WAL, when restarting.\n";
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
    let mut timeout = 60;
    let mut crash = None;
    let mut seen = std::collections::BTreeSet::new();
    for pair in args[1..].chunks(2) {
        if pair.len() != 2 || !seen.insert(pair[0].as_str()) {
            return Err(eyre!("missing or duplicate option; use --help"));
        }
        let value = &pair[1];
        match pair[0].as_str() {
            "--dir" => dir = Some(PathBuf::from(value)),
            "--index" if command == "run" => index = Some(value.parse()?),
            "--base-port" if command == "run" => base_port = value.parse()?,
            "--height" if command == "run" => height = value.parse()?,
            "--timeout" if command == "run" => timeout = value.parse()?,
            "--crash-after" if command == "run" => {
                crash = Some(match value.as_str() {
                    "proposal" => CrashPoint::Proposal,
                    "vote" => CrashPoint::Vote,
                    "before-commit" => CrashPoint::BeforeCommit,
                    _ => return Err(eyre!("unknown crash point")),
                })
            }
            _ => return Err(eyre!("unknown option for {command}; use --help")),
        }
    }
    let directory = dir.ok_or_else(|| eyre!("--dir is required"))?;
    let state = if command == "run" {
        if timeout == 0 || timeout > 3600 {
            return Err(eyre!("timeout must be 1..3600 seconds"));
        }
        eprintln!("NOT production-ready; no real money. Public development keys; loopback only.");
        run_node(NodeOptions {
            directory,
            index: index.ok_or_else(|| eyre!("--index is required"))?,
            base_port,
            target_height: height,
            timeout: Duration::from_secs(timeout),
            crash,
        })
        .await?
    } else {
        inspect(&directory)?
    };
    println!("{}", status_json(&state));
    Ok(())
}
