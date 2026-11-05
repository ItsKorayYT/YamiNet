//! M23 SOFTWARE protocol worker, never hardware attestation. No local-anchor fallback.
use std::io::{self, BufRead, Write};
use yamichain_tee_m20::{witness::*, Result};
fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: witnessed-worker PRIVATE_CONFIG")?;
    let config: WorkerConfig = serde_json::from_slice(&std::fs::read(path)?)?;
    if !config.software_protocol_only {
        return Err("worker is SOFTWARE PROTOCOL ONLY; hardware activation is closed".into());
    }
    let mut replica = WitnessedReplica::load(&config.snapshot, config.client()?, config.keys())?;
    let input = io::stdin();
    let mut output = io::BufWriter::new(io::stdout().lock());
    for line in input.lock().lines() {
        let line = line?;
        let attempt = (|| -> Result<WorkerReply> {
            Ok(match serde_json::from_str::<WorkerCommand>(&line)? {
                WorkerCommand::Prepare(p) => WorkerReply::Approval(Box::new(replica.prepare(&p)?)),
                WorkerCommand::PrepareFault { payment, stage } => {
                    if !config.fault_injection {
                        return Err("fault harness disabled".into());
                    }
                    WorkerReply::Approval(Box::new(replica.prepare_with_hook(
                        &payment,
                        |point| {
                            if point == stage {
                                Err("injected interruption".into())
                            } else {
                                Ok(())
                            }
                        },
                    )?))
                }
                WorkerCommand::Commit(qc) => {
                    replica.commit(&qc)?;
                    WorkerReply::Committed
                }
                WorkerCommand::CommitFault { certificate, stage } => {
                    if !config.fault_injection {
                        return Err("fault harness disabled".into());
                    }
                    replica.commit_with_hook(&certificate, |point| {
                        if point == stage {
                            Err("injected interruption".into())
                        } else {
                            Ok(())
                        }
                    })?;
                    WorkerReply::Committed
                }
                WorkerCommand::Resume => WorkerReply::Approval(Box::new(replica.resume_vote()?)),
                WorkerCommand::Head => WorkerReply::Head(replica.head()?),
                WorkerCommand::Query(q) => WorkerReply::Balance(replica.query(&q)?),
                WorkerCommand::ExportSince { index, after } => {
                    WorkerReply::Recovery(Box::new(replica.export_recovery_since(index, after)?))
                }
                WorkerCommand::Export(i) => {
                    WorkerReply::Recovery(Box::new(replica.export_recovery(i)?))
                }
                WorkerCommand::Recover(bundle) => {
                    replica.recover(&bundle)?;
                    WorkerReply::Committed
                }
            })
        })();
        let reply = attempt.unwrap_or_else(|e| WorkerReply::Rejected(e.to_string()));
        serde_json::to_writer(&mut output, &reply)?;
        writeln!(output)?;
        output.flush()?;
    }
    Ok(())
}
