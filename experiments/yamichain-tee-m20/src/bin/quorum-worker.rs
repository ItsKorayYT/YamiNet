//! SOFTWARE-ONLY worker. Keys/config are PRIVATE temporary fixtures supplied by test parent.
use ed25519_dalek::Signer;
use std::io::{self, BufRead, Write};
use yamichain_tee_m20::{quorum::*, software_harness::*, *};
fn main() -> Result<()> {
    let config = std::env::args()
        .nth(1)
        .ok_or("usage: quorum-worker PRIVATE_SOFTWARE_CONFIG")?;
    let c: SoftwareConfig = serde_json::from_slice(&std::fs::read(config)?)?;
    let mut replica = Replica::load(
        &c.snapshot,
        &c.trusted_anchor,
        c.committee.clone(),
        c.index,
        c.keys(),
    )?;
    let input = io::stdin();
    let mut output = io::BufWriter::new(io::stdout().lock());
    for line in input.lock().lines() {
        let line = line?;
        let reply = (|| -> Result<Reply> {
            Ok(match serde_json::from_str::<Command>(&line)? {
                Command::Prepare(payment) => Reply::Approval(replica.prepare(&payment)?),
                Command::Commit(qc) => {
                    replica.commit(&qc)?;
                    Reply::Committed
                }
                Command::Query(q) => Reply::Balance(replica.query(&q)?),
                Command::Head => Reply::Head(replica.head()?),
                Command::ExportRecovery(index) => Reply::Recovery(replica.export_recovery(index)?),
                Command::Recover(envelope) => {
                    replica.recover(&envelope)?;
                    Reply::Committed
                }
                Command::CompromisedBalance(q, account) => {
                    if !c.compromised {
                        return Err("attacker fixture disabled".into());
                    }
                    let mut response = BalanceReply {
                        portfolio: None,
                        committee: c.committee.id(),
                        member: c.committee.members[c.index].signing,
                        account,
                        query_hash: hash(&q.message()),
                        signature: vec![],
                    };
                    response.signature = c
                        .keys()
                        .signing
                        .sign(&response.message())
                        .to_bytes()
                        .to_vec();
                    Reply::Balance(encrypt(q.reply_key, &bytes(&response), &q.message())?)
                }
                Command::CompromisedSign(statement) => {
                    if !c.compromised {
                        return Err("attacker fixture disabled".into());
                    }
                    let signature = c
                        .keys()
                        .signing
                        .sign(&statement.message())
                        .to_bytes()
                        .to_vec();
                    Reply::Approval(Approval {
                        member: c.committee.members[c.index].signing,
                        statement,
                        signature,
                    })
                }
            })
        })();
        serde_json::to_writer(&mut output, &reply.unwrap_or(Reply::Rejected))?;
        writeln!(output)?;
        output.flush()?;
    }
    Ok(())
}
