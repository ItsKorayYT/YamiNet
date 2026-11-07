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
        "validator-key" => {
            let consensus = SigningKey::generate(&mut OsRng);
            let transport = SigningKey::generate(&mut OsRng);
            serde_json::json!({"consensus":hex::encode(consensus.verifying_key().to_bytes()),"transport":hex::encode(transport.verifying_key().to_bytes()),"consensus_secret":consensus.to_bytes(),"transport_secret":transport.to_bytes()})
        }
        "registration" => {
            use ed25519_dalek::Signer;
            let owner: Id = hex::decode(v["owner"].as_str().ok_or("owner")?)?
                .try_into()
                .map_err(|_| "owner")?;
            let c = SigningKey::from_bytes(&serde_json::from_value(
                v["validator"]["consensus_secret"].clone(),
            )?);
            let t = SigningKey::from_bytes(&serde_json::from_value(
                v["validator"]["transport_secret"].clone(),
            )?);
            let msg = economics::registration_message(
                v["chain"].as_u64().unwrap_or(24),
                owner,
                v["nonce"].as_u64().ok_or("nonce")?,
                c.verifying_key().to_bytes(),
                t.verifying_key().to_bytes(),
            );
            serde_json::to_value(economics::Action::Register {
                consensus: c.verifying_key().to_bytes(),
                transport: t.verifying_key().to_bytes(),
                consensus_proof: c.sign(&msg).to_bytes().to_vec(),
                transport_proof: t.sign(&msg).to_bytes().to_vec(),
            })?
        }
        "development-validator-wallet" => {
            let i = u8::try_from(v["index"].as_u64().ok_or("index")?)?;
            if i >= 4 {
                return Err("fixture index".into());
            }
            let k = yami_chain::development_key("consensus", i);
            serde_json::json!({"secret":k.to_bytes(),"address":hex::encode(k.verifying_key().to_bytes()),"insecure_public_fixture":true})
        }
        "reward-addresses" => serde_json::json!((0..4)
            .map(|i| yami_chain::development_key("consensus", i)
                .verifying_key()
                .to_bytes())
            .collect::<Vec<_>>()),
        "init" => {
            let c: WorkerConfig = serde_json::from_value(v["config"].clone())?;
            let allocations: Vec<(Id, u64)> = serde_json::from_value(v["allocations"].clone())?;
            let accounts = allocations
                .into_iter()
                .map(|(id, balance)| (id, Account { balance, nonce: 0 }))
                .collect();
            let mut s = State {
                economics: None,

                chain: c.policy.committee.chain,
                epoch: 0,
                accounts,
            };
            if let Some(policy) = c.economics.clone() {
                s.enable_economics(policy)?;
            }
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
            let mut state = State {
                chain: v["chain"].as_u64().unwrap_or(24),
                epoch: 0,
                accounts,
                economics: None,
            };
            if let Some(policy) =
                serde_json::from_value::<Option<economics::Policy>>(v["economics"].clone())?
            {
                state.enable_economics(policy)?;
            }
            serde_json::to_value(state_head(&state, root))?
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
                    finance: serde_json::from_value(v["finance"].clone())?,

                    sender: key.verifying_key().to_bytes(),
                    recipient,
                    amount: v["amount"].as_u64().ok_or("amount")?,
                    nonce: v["nonce"].as_u64().ok_or("nonce")?,
                    chain: p.committee.chain,
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
            serde_json::to_value(verify_portfolio(
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
        "validate-certificate" => {
            let payload: Vec<u8> = serde_json::from_value(v["payload"].clone())?;
            if payload.len() > 30000 {
                return Err("certificate byte limit".into());
            }
            let qc: WitnessedCertificate = serde_json::from_slice(&payload)?;
            if bytes(&qc) != payload {
                return Err("noncanonical certificate".into());
            }
            let policy: WitnessPolicy = serde_json::from_value(v["policy"].clone())?;
            let height = v["height"].as_u64().filter(|h| *h > 0).ok_or("height")?;
            let previous = if height == 1 {
                policy.genesis.clone()
            } else {
                let raw: Vec<u8> = serde_json::from_value(v["previous_record"].clone())?;
                let record = yami_chain_storage::FinalizedRecord::decode(&raw)?;
                yami_chain_consensus::verify_record(&yami_chain::Genesis::local(), &record)?;
                if record.block.header.height.checked_add(1) != Some(height)
                    || record.block.transactions.len() != 1
                {
                    return Err("previous record height".into());
                }
                let yami_chain::Action::PrivatePaymentCertificateV1 { certificate } =
                    &record.block.transactions[0].action
                else {
                    return Err("previous action".into());
                };
                let old: WitnessedCertificate = serde_json::from_slice(certificate)?;
                old.verify(&policy, &old.certificate.statement.previous)?;
                old.certificate.statement.next
            };
            if previous.epoch.checked_add(1) != Some(height) {
                return Err("certificate sequence".into());
            }
            qc.verify(&policy, &previous)?;
            serde_json::json!({"valid":true})
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
            let mut accepted = None;
            for t in &record.block.transactions {
                if let yami_chain::Action::PrivatePaymentCertificateV1 { certificate } = &t.action {
                    let candidate: WitnessedCertificate = serde_json::from_slice(certificate)?;
                    candidate.verify(&policy, &qc.certificate.statement.previous)?;
                    if candidate.certificate.statement == qc.certificate.statement {
                        accepted = Some(candidate);
                        break;
                    }
                }
            }
            let accepted = accepted.ok_or("native transition certificate absent")?;
            serde_json::json!({"height":height,"block":record.block.id()?.to_string(),"action":"PrivatePaymentCertificateV1","certificate":accepted,"record":raw,"consensus":"YamiChain Malachite commit certificate"})
        }
        _ => return Err("unknown operation".into()),
    };
    println!("{}", out);
    Ok(())
}
