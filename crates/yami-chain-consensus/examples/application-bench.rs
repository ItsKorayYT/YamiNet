//! Deterministic non-financial application-state and certified recovery profile.
use malachite::app::types::core as m;
use std::{hint::black_box, path::PathBuf, time::Instant};
use yami_chain::{
    codec::Writer, development_key, hash, Block, Genesis, Signer as _, SigningKey, State,
    Transaction,
};
use yami_chain_consensus::*;
use yami_chain_storage::{FinalizedRecord, Journal};
fn certificate(genesis: &Genesis, block: &Block) -> eyre::Result<Vec<u8>> {
    let h = block.header.height;
    let mut signatures = Vec::new();
    for i in 0..4 {
        let key = development_key("consensus", i);
        let member = key.verifying_key().to_bytes();
        let mut w = Writer::default();
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
        w.raw(&member);
        signatures.push(m::CommitSignature::new(
            Address(yami_chain::Digest(member)),
            key.sign(&w.0).to_bytes(),
        ));
    }
    let cert = m::CommitCertificate {
        height: Height(h),
        round: m::Round::ZERO,
        value_id: Value(block.id()?),
        commit_signatures: signatures,
    };
    Ok(encode_certificate(genesis, &cert)?)
}
fn main() -> eyre::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        eyre::bail!("application-bench NEW_DIR ENTRIES shared|distinct");
    }
    let path = PathBuf::from(&args[0]);
    let count: usize = args[1].parse()?;
    let distinct = match args[2].as_str() {
        "shared" => false,
        "distinct" => true,
        _ => eyre::bail!("pattern"),
    };
    if path.exists() || !(1..=100_001).contains(&count) {
        eyre::bail!("fresh path and 1..100001 entries");
    }
    let genesis = Genesis::development_schedule(&[(1, 4)])?;
    let journal = Journal::open(&path.join("archive"))?;
    journal.migrate_archive(genesis.clone(), |r| verify_record(&genesis, r))?;
    let mut state = State::new(genesis.clone())?;
    let mut propose = 0.0;
    let mut execute = 0.0;
    let mut persistence = 0.0;
    let started = Instant::now();
    for first in (0..count).step_by(64) {
        let height = state.height() + 1;
        let mut txs = Vec::new();
        for n in first..count.min(first + 64) {
            let owner = if distinct {
                SigningKey::from_bytes(&hash(b"YC-STATE-BENCH-KEY-1", &(n as u64).to_be_bytes()).0)
            } else {
                development_key("registry", 0)
            };
            txs.push(Transaction::sign(
                &format!("test-state-{n:06}"),
                if distinct { 0 } else { n as u64 },
                height,
                &owner,
            )?);
        }
        let at = Instant::now();
        let block = state.propose(txs)?;
        propose += at.elapsed().as_secs_f64();
        let at = Instant::now();
        state = state.execute(&block)?;
        execute += at.elapsed().as_secs_f64();
        let at = Instant::now();
        let record = FinalizedRecord {
            certificate: certificate(&genesis, &block)?,
            block,
        };
        verify_record(&genesis, &record)?;
        journal.append(&record)?;
        persistence += at.elapsed().as_secs_f64();
    }
    let generation = started.elapsed().as_secs_f64();
    let at = Instant::now();
    let root = state.root();
    let first_root = at.elapsed().as_secs_f64();
    let at = Instant::now();
    for _ in 0..100 {
        black_box(&state).root();
    }
    let repeated_root = at.elapsed().as_secs_f64();
    let at = Instant::now();
    let forks: Vec<_> = (0..8).map(|_| black_box(state.clone())).collect();
    let clone_seconds = at.elapsed().as_secs_f64();
    for fork in &forks {
        assert_eq!(fork.root(), root);
    }
    drop(forks);
    let at = Instant::now();
    let mut found = 0;
    for n in 0..1024 {
        let label = format!("test-state-{:06}", (n * 7919) % count);
        if black_box(state.names().get(&label)).is_some() {
            found += 1;
        }
    }
    let lookups = at.elapsed().as_secs_f64();
    assert_eq!(found, 1024);
    let at = Instant::now();
    let snapshot = state.snapshot_bytes();
    let serialization = at.elapsed().as_secs_f64();
    if count > 100_000 {
        assert!(snapshot.is_err());
        println!("{{\"entries\":{count},\"pattern\":\"{}\",\"height\":{},\"root\":\"{root}\",\"tip\":\"{}\",\"snapshot_limit_preserved\":true,\"generation_seconds\":{generation},\"passed\":true}}",args[2],state.height(),state.tip());
        return Ok(());
    }
    let snapshot = snapshot?;
    let snapshot_digest = hash(b"YC-STATE-BENCH-SNAPSHOT-1", &snapshot);
    let at = Instant::now();
    for _ in 0..3 {
        assert_eq!(
            State::restore_checkpoint(
                genesis.clone(),
                state.height(),
                state.tip(),
                root,
                &snapshot
            )?,
            state
        );
    }
    let restore = at.elapsed().as_secs_f64() / 3.0;
    let at = Instant::now();
    let checkpoint = export_compact_checkpoint(&journal, &genesis)?;
    let export = at.elapsed().as_secs_f64();
    yami_chain_storage::publish_checkpoint_export(
        &path.join("checkpoint.bin"),
        checkpoint.bytes(),
    )?;
    let fresh = Journal::open(&path.join("fresh"))?;
    let at = Instant::now();
    assert_eq!(
        import_compact_checkpoint(&fresh, &genesis, checkpoint.bytes(), checkpoint.anchor())?,
        state
    );
    let import = at.elapsed().as_secs_f64();
    drop(fresh);
    let at = Instant::now();
    for _ in 0..3 {
        let j = Journal::open(&path.join("fresh"))?;
        assert_eq!(replay_certified(&j, &genesis)?, state);
    }
    let compact_restart = at.elapsed().as_secs_f64() / 3.0;
    drop(journal);
    let at = Instant::now();
    let journal = Journal::open(&path.join("archive"))?;
    assert_eq!(replay_certified(&journal, &genesis)?, state);
    let replay = at.elapsed().as_secs_f64();
    let footprint = journal.storage_footprint(1_000_000)?;
    println!("{{\"entries\":{count},\"pattern\":\"{}\",\"height\":{},\"root\":\"{root}\",\"tip\":\"{}\",\"snapshot_digest\":\"{snapshot_digest}\",\"snapshot_bytes\":{},\"checkpoint_bytes\":{},\"generation_seconds\":{generation},\"propose_seconds\":{propose},\"execute_seconds\":{execute},\"persistence_seconds\":{persistence},\"first_root_seconds\":{first_root},\"repeated_roots_100_seconds\":{repeated_root},\"clones_8_seconds\":{clone_seconds},\"lookups_1024_seconds\":{lookups},\"snapshot_serialization_seconds\":{serialization},\"snapshot_restore_mean_seconds\":{restore},\"checkpoint_export_seconds\":{export},\"checkpoint_import_seconds\":{import},\"compact_restart_mean_seconds\":{compact_restart},\"archive_replay_seconds\":{replay},\"archive_allocated_bytes\":{},\"archive_entries\":{},\"passed\":true}}",args[2],state.height(),state.tip(),snapshot.len(),checkpoint.bytes().len(),footprint.allocated_bytes,footprint.entries);
    Ok(())
}
