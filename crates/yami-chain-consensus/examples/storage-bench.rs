//! Offline real-signature storage campaign; no sockets, money or persistent keys.
use malachite::app::types::core as m;
use std::{io::Read as _, path::PathBuf, time::Instant};
use yami_chain::{
    committee::{Epoch, Member, ValidatorSet},
    Genesis, Signer as _, SigningKey,
};
use yami_chain_consensus::*;
use yami_chain_storage::{FinalizedRecord, Journal};
fn main() -> eyre::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        eyre::bail!("storage-bench NEW_DIR HEIGHT TRANSACTIONS");
    }
    let path = PathBuf::from(&args[0]);
    let height: u64 = args[1].parse()?;
    let tx: usize = args[2].parse()?;
    if path.exists() || !(12..=4096).contains(&height) || tx > 64 {
        eyre::bail!("fresh directory, height 12..4096, tx 0..64");
    }
    std::fs::create_dir_all(&path)?;
    let mut random = std::fs::File::open("/dev/urandom")?;
    let mut keys = Vec::new();
    let mut members = Vec::new();
    for _ in 0..7 {
        let mut bytes = [0; 64];
        random.read_exact(&mut bytes)?;
        let c = SigningKey::from_bytes(&bytes[..32].try_into()?);
        let t = SigningKey::from_bytes(&bytes[32..].try_into()?);
        members.push(Member {
            consensus_key: c.verifying_key().to_bytes(),
            transport_key: t.verifying_key().to_bytes(),
            power: 1,
        });
        keys.push(c);
    }
    let mut genesis = Genesis::local();
    genesis.epochs = [(1, 4), (height / 3, 7), (height * 2 / 3, 4)]
        .into_iter()
        .map(|(first, n)| {
            Ok(Epoch {
                first_height: first,
                validators: ValidatorSet::new(members[..n].to_vec())?,
            })
        })
        .collect::<yami_chain::Result<_>>()?;
    let j = Journal::open(&path.join("archive"))?;
    let mut state = j.replay(genesis.clone(), |_| Ok(()))?;
    let start = Instant::now();
    for h in 1..=height {
        let block = state.development_workload_block(tx)?;
        let mut signatures = Vec::new();
        for member in genesis.committee_at(h)?.members() {
            let key = keys
                .iter()
                .find(|k| k.verifying_key().to_bytes() == member.consensus_key)
                .unwrap();
            let vote = Vote {
                height: Height(h),
                round: m::Round::ZERO,
                value: m::NilOrVal::Val(Value(block.id()?)),
                step: m::VoteType::Precommit,
                address: Address(yami_chain::Digest(member.consensus_key)),
                extension: None,
            };
            // Build the same canonical v2 vote preimage as the production signer.
            let mut w = yami_chain::codec::Writer::default();
            w.raw(b"YC-CONSENSUS-2");
            w.raw(&genesis.id().0);
            w.raw(&genesis.committee_at(h)?.hash().0);
            w.u8(1);
            w.u64(h);
            w.u8(1);
            w.u32(0);
            w.u8(2);
            w.u8(1);
            w.raw(&block.id()?.0);
            w.raw(&member.consensus_key);
            if key.verifying_key() == keys[0].verifying_key() {
                j.reserve_signature(h, 0, 2, &w.0)?;
            }
            signatures.push(m::CommitSignature::new(
                vote.address,
                key.sign(&w.0).to_bytes(),
            ));
        }
        let cert = m::CommitCertificate {
            height: Height(h),
            round: m::Round::ZERO,
            value_id: Value(block.id()?),
            commit_signatures: signatures,
        };
        let record = FinalizedRecord {
            block: block.clone(),
            certificate: encode_certificate(&genesis, &cert)?,
        };
        verify_record(&genesis, &record)?;
        j.append(&record)?;
        state = state.execute(&block)?;
    }
    let generation = start.elapsed().as_secs_f64();
    let start = Instant::now();
    assert_eq!(replay_certified(&j, &genesis)?, state);
    let archive_restart = start.elapsed().as_secs_f64();
    let archive_bytes = j.storage_footprint(100_000)?.bytes;
    let start = Instant::now();
    let snapshot = export_compact_checkpoint(&j, &genesis)?;
    let export = start.elapsed().as_secs_f64();
    let fresh = Journal::open(&path.join("fresh"))?;
    let start = Instant::now();
    assert_eq!(
        import_compact_checkpoint(&fresh, &genesis, snapshot.bytes(), snapshot.anchor())?,
        state
    );
    let import = start.elapsed().as_secs_f64();
    let start = Instant::now();
    assert_eq!(replay_certified(&fresh, &genesis)?, state);
    let compact_restart = start.elapsed().as_secs_f64();
    let fresh_bytes = fresh.storage_footprint(100_000)?.bytes;
    let start = Instant::now();
    assert_eq!(prune_certified_before(&j, &genesis, height - 8)?, state);
    let pruning = start.elapsed().as_secs_f64();
    let after = j.storage_footprint(100_000)?;
    let start = Instant::now();
    assert_eq!(replay_certified(&j, &genesis)?, state);
    let pruned_restart = start.elapsed().as_secs_f64();
    println!("{{\"height\":{height},\"transactions_per_block\":{tx},\"generation_seconds\":{generation},\"archive_replay_seconds\":{archive_restart},\"archive_bytes\":{archive_bytes},\"snapshot_bytes\":{},\"export_seconds\":{export},\"import_seconds\":{import},\"compact_restart_seconds\":{compact_restart},\"fresh_bytes\":{fresh_bytes},\"prune_seconds\":{pruning},\"pruned_bytes\":{},\"retained_signing_bytes\":{},\"pruned_restart_seconds\":{pruned_restart},\"retained_tail\":8,\"passed\":true}}",snapshot.bytes().len(),after.bytes,after.signing_bytes);
    Ok(())
}
