//! Local wallet/bootstrap adapter for the INSECURE disposable devnet.
use ed25519_dalek::SigningKey;
use rand::{rngs::OsRng, RngCore};
use std::{collections::BTreeMap, io::Read};
use x25519_dalek::{PublicKey, StaticSecret};
use yamichain_tee_m20::{quorum::*, witness::*, *};
fn main() -> Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let v: serde_json::Value = serde_json::from_str(&input)?;
    let op = v["op"].as_str().ok_or("op")?;
    let out = match op {
        "key" => {
            let k = SigningKey::generate(&mut OsRng);
            let t = StaticSecret::random_from_rng(OsRng);
            serde_json::json!({"secret":k.to_bytes(),"address":hex::encode(k.verifying_key().to_bytes()),"transport":t.to_bytes(),"transport_public":PublicKey::from(&t).to_bytes()})
        }
        "init" => {
            let c: WorkerConfig = serde_json::from_value(v["config"].clone())?;
            let allocations: Vec<(Id, u64)> = serde_json::from_value(v["allocations"].clone())?;
            let accounts = allocations
                .into_iter()
                .map(|(id, balance)| (id, Account { balance, nonce: 0 }))
                .collect();
            let s = State {
                chain: 24,
                epoch: 0,
                accounts,
            };
            drop(WitnessedReplica::create(
                &c.snapshot,
                c.client()?,
                c.keys(),
                &s,
            )?);
            serde_json::json!(true)
        }
        "genesis" => {
            let allocations: Vec<(Id, u64)> = serde_json::from_value(v["allocations"].clone())?;
            let root: Id = serde_json::from_value(v["root"].clone())?;
            let accounts: BTreeMap<_, _> = allocations
                .into_iter()
                .map(|(id, balance)| (id, Account { balance, nonce: 0 }))
                .collect();
            serde_json::to_value(state_head(
                &State {
                    chain: 24,
                    epoch: 0,
                    accounts,
                },
                root,
            ))?
        }
        "payment" => {
            let p: WitnessPolicy = serde_json::from_value(v["policy"].clone())?;
            let head: Head = serde_json::from_value(v["head"].clone())?;
            let sk: Id = serde_json::from_value(v["secret"].clone())?;
            let recipient: Id = hex::decode(v["recipient"].as_str().ok_or("recipient")?)?
                .try_into()
                .map_err(|_| "address")?;
            let key = SigningKey::from_bytes(&sk);
            let tx = Signed::sign(
                Transfer {
                    sender: key.verifying_key().to_bytes(),
                    recipient,
                    amount: v["amount"].as_u64().ok_or("amount")?,
                    nonce: v["nonce"].as_u64().ok_or("nonce")?,
                    chain: 24,
                },
                &key,
            );
            if v["intent"].as_bool() == Some(true) {
                serde_json::json!({"Payment":{"transfer":tx,"previous":head}})
            } else {
                serde_json::to_value(Payment::software(&p.committee, &head, tx)?)?
            }
        }
        "query" => {
            let sk: Id = serde_json::from_value(v["secret"].clone())?;
            let key = SigningKey::from_bytes(&sk);
            let head: Head = serde_json::from_value(v["head"].clone())?;
            let policy: WitnessPolicy = serde_json::from_value(v["policy"].clone())?;
            let reply = StaticSecret::random_from_rng(OsRng);
            let mut challenge = [0; 32];
            OsRng.fill_bytes(&mut challenge);
            let mut q = Query {
                account: key.verifying_key().to_bytes(),
                reply_key: PublicKey::from(&reply).to_bytes(),
                head,
                challenge,
                signature: vec![],
            };
            q.sign(&key);
            if v["intent"].as_bool() == Some(true) {
                println!(
                    "{}",
                    serde_json::json!({"operation":{"Query":q},"query":q,"secret":reply.to_bytes()})
                );
                return Ok(());
            }
            let envelopes: Vec<_> = policy
                .committee
                .members
                .iter()
                .map(|m| encrypt(m.transport, &bytes(&q), QUERY_DOMAIN))
                .collect::<Result<_>>()?;
            serde_json::json!({"query":q,"secret":reply.to_bytes(),"envelopes":envelopes})
        }
        "balance" => {
            let p: WitnessPolicy = serde_json::from_value(v["policy"].clone())?;
            let q: Query = serde_json::from_value(v["query"].clone())?;
            let secret: Id = serde_json::from_value(v["secret"].clone())?;
            let replies: Vec<Envelope> = serde_json::from_value(v["replies"].clone())?;
            serde_json::to_value(verify_balance(
                &p.committee,
                &q,
                &StaticSecret::from(secret),
                &replies,
            )?)?
        }
        "certificate" => {
            let p: WitnessPolicy = serde_json::from_value(v["policy"].clone())?;
            let h: Head = serde_json::from_value(v["head"].clone())?;
            let votes: Vec<WitnessedApproval> = serde_json::from_value(v["votes"].clone())?;
            let qc = WitnessedCertificate::collect(&p, &h, votes)?;
            serde_json::json!({"certificate":qc,"payload":bytes(&qc)})
        }
        "audit-native" => {
            let mut state = yami_chain::State::insecure_devnet(
                yami_chain::Genesis::local(),
                &serde_json::to_vec(&v["policy"])?,
            )?;
            let records: Vec<Vec<u8>> = serde_json::from_value(v["records"].clone())?;
            for raw in records {
                let record = yami_chain_storage::FinalizedRecord::decode(&raw)?;
                yami_chain_consensus::verify_record(state.genesis(), &record)?;
                state = state.execute(&record.block)?;
            }
            let before = state.root();
            let accepted = if v["candidate"].is_null() {
                false
            } else {
                let owner = yami_chain::development_key("registry", 0);
                let tx = yami_chain::Transaction::devnet_certificate(
                    serde_json::to_vec(&v["candidate"])?,
                    state.nonce(&owner.verifying_key().to_bytes()),
                    state.height() + 1,
                    &owner,
                )?;
                state.propose(vec![tx]).is_ok()
            };
            serde_json::json!({"height":state.height(),"root":before.to_string(),"candidate_accepted":accepted,"state_unchanged":before==state.root()})
        }
        "finality" => {
            let height = v["height"].as_u64().ok_or("height")?;
            let raw: Vec<u8> = serde_json::from_value(v["record"].clone())?;
            let record = yami_chain_storage::FinalizedRecord::decode(&raw)?;
            if record.block.header.height != height {
                return Err("height mismatch".into());
            }
            yami_chain_consensus::verify_record(&yami_chain::Genesis::local(), &record)?;
            let qc: WitnessedCertificate = serde_json::from_value(v["certificate"].clone())?;
            let policy: WitnessPolicy = serde_json::from_value(v["policy"].clone())?;
            qc.verify(&policy, &qc.certificate.statement.previous)?;
            if !record.block.transactions.iter().any(|t| matches!(&t.action,yami_chain::Action::PrivatePaymentCertificateV1{certificate} if *certificate==bytes(&qc))) {return Err("native certificate absent".into());}
            serde_json::json!({"height":height,"block":record.block.id()?.to_string(),"action":"PrivatePaymentCertificateV1","certificate":qc,"record":raw,"consensus":"YamiChain Malachite commit certificate"})
        }
        _ => return Err("unknown operation".into()),
    };
    println!("{}", out);
    Ok(())
}
