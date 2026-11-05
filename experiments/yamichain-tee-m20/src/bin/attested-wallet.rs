//! Isolated wallet-side local process. TRUSTED_CONFIG belongs to the wallet, not the executor host.
//! Two stdin lines: signed operation, then raw evidence for the emitted challenges.
use serde::Deserialize;
use std::{
    io::{self, BufRead, Read, Write},
    path::PathBuf,
};
use yamichain_tee_m20::{attestation::*, quorum::*, *};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    qvl_library: PathBuf,
    qvl_sha256: Id,
    committee: Committee,
    genesis: Head,
    policies: [SoftwarePolicy; 4],
}
#[derive(Deserialize)]
enum Operation {
    Payment { transfer: Signed, previous: Head },
    Query(Query),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    round: Id,
    evidence: [Evidence; 4],
}
fn line(reader: &mut impl BufRead, limit: u64) -> Result<String> {
    let mut line = String::new();
    let count = reader.take(limit + 1).read_line(&mut line)?;
    if count == 0 || count as u64 > limit || !line.ends_with('\n') {
        return Err("missing or oversized wallet input frame".into());
    }
    Ok(line)
}
fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: attested-wallet TRUSTED_CONFIG")?;
    let config: Config = serde_json::from_slice(&std::fs::read(path)?)?;
    let qvl = IntelQvl::load(&config.qvl_library, config.qvl_sha256)?;
    let mut gate = WalletGate::new(qvl, config.committee, config.genesis, config.policies)?;
    let input = io::stdin();
    let mut reader = input.lock();
    let operation: Operation = serde_json::from_str(&line(&mut reader, 64 * 1024)?)?;
    let challenge = match operation {
        Operation::Payment { transfer, previous } => gate.begin_payment(transfer, previous)?,
        Operation::Query(q) => gate.begin_query(q)?,
    };
    let mut output = io::BufWriter::new(io::stdout().lock());
    serde_json::to_writer(&mut output, &challenge)?;
    writeln!(output)?;
    output.flush()?;
    let response: Response = serde_json::from_str(&line(&mut reader, 32 * 1024 * 1024)?)?;
    gate.finish_and_submit(response.round, &response.evidence, |request| {
        match request {
            WalletRequest::Payment(p) => {
                serde_json::to_writer(&mut output, &serde_json::json!({"Payment":p.packet()}))?
            }
            WalletRequest::Queries(q) => {
                serde_json::to_writer(&mut output, &serde_json::json!({"Queries":q.envelopes()}))?
            }
        }
        writeln!(output)?;
        output.flush()?;
        Ok(())
    })
}
