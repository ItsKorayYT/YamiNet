//! Offline signed/certified histories, not a live consensus-engine benchmark.
//! Fixture signers never become live identities; reservations are retained.
use malachite::app::types::core as m;
use std::{hint::black_box, path::PathBuf, time::Instant};
use yami_chain::snapshot_v2::{store, ExperimentalState};
use yami_chain::{
    codec::Writer, development_key, hash, Block, Genesis, Signer as _, SigningKey, State,
    Transaction,
};
use yami_chain_consensus::*;
use yami_chain_storage::{FinalizedRecord, Journal};
fn certificate(genesis: &Genesis, block: &Block, signers: &[Journal]) -> eyre::Result<Vec<u8>> {
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
        signers[i as usize].reserve_signature(h, 0, 2, &w.0)?;
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
fn laboratory_genesis(count: usize, distinct: bool) -> yami_chain::Result<Genesis> {
    // Whitelisted cases have disjoint future-epoch domains, including shared vs distinct.
    if ![1024, 100_000, 250_000, 500_000].contains(&count) {
        return Err(yami_chain::Error::Limit);
    }
    Genesis::development_schedule(&[
        (1, 4),
        (800_000 + count as u64 / 10 + u64::from(distinct), 5),
    ])
}
fn fence_observer(journal: &Journal, genesis: &Genesis) -> eyre::Result<()> {
    // A VALID existing identity fence, for an identity absent from every fixture
    // epoch. Old binaries also refuse rebinding these observer-only archives.
    let observer = (yami_chain::committee::MAX_VALIDATORS - 1) as u8;
    eyre::ensure!(
        !genesis.has_consensus_key(
            &development_key("consensus", observer)
                .verifying_key()
                .to_bytes()
        ),
        "observer identity is scheduled"
    );
    journal.ensure_genesis(genesis)?;
    journal.bind_validator_index(observer)?;
    journal.write_once(
        "offline-observer.bin",
        b"YC-OFFLINE-CERTIFICATE-FIXTURE-NOT-A-SIGNING-IDENTITY",
    )?;
    Ok(())
}
fn digest(text: &str) -> eyre::Result<yami_chain::Digest> {
    eyre::ensure!(text.len() == 64 && text.is_ascii(), "hex digest");
    let mut b = [0; 32];
    for (i, value) in b.iter_mut().enumerate() {
        *value = u8::from_str_radix(&text[2 * i..2 * i + 2], 16)?;
    }
    Ok(yami_chain::Digest(b))
}
#[cfg(feature = "inactive-checkpoint-v2")]
fn inactive_profile(
    journal: &Journal,
    state: &State,
    path: &std::path::Path,
) -> eyre::Result<String> {
    use yami_chain::checkpoint_v2::{self as codec, InactiveState, StateAnchor};
    use yami_chain_consensus::checkpoint_v2::{envelope, verify_context};
    std::fs::create_dir_all(path)?;
    let source = path.join("inactive-v2");
    let received = path.join("inactive-v2-received");
    let at = Instant::now();
    let anchor = codec::store::save(&InactiveState::from_verified_state(state)?, &source)?;
    let manifest = codec::Manifest::decode(&std::fs::read(source.join("manifest.bin"))?, anchor)?;
    let record = journal
        .record(state.height())?
        .ok_or_else(|| eyre::eyre!("terminal missing"))?;
    let bytes = envelope(state.genesis(), &record, &manifest)?;
    std::fs::write(path.join("inactive-envelope.bin"), &bytes)?;
    let save = at.elapsed().as_secs_f64();
    let initial = State::new(state.genesis().clone())?;
    let minimum = StateAnchor {
        genesis: state.genesis().id(),
        height: 0,
        tip: initial.tip(),
        root: initial.root(),
    };
    let at = Instant::now();
    // Offline fixture: fail the benchmark on any archive read error. Own one
    // encoded record at a time, not an allocated full-history ancestry vector.
    let context = verify_context(
        state.genesis(),
        minimum,
        &bytes,
        (1..=state.height()).map(|h| {
            journal
                .record(h)
                .expect("ancestry read")
                .expect("ancestry gap")
                .canonical()
                .expect("ancestry encoding")
        }),
    )?;
    let finality = at.elapsed().as_secs_f64();
    let at = Instant::now();
    assert_eq!(context.recover(&source)?.anchor(), anchor.state);
    let recovery = at.elapsed().as_secs_f64();
    let at = Instant::now();
    assert_eq!(context.transfer(&source, &received)?.anchor(), anchor.state);
    let transfer = at.elapsed().as_secs_f64();
    Ok(format!("{{\"save_seconds\":{save},\"certified_ancestry_seconds\":{finality},\"recovery_seconds\":{recovery},\"transfer_seconds\":{transfer},\"entries\":{},\"height\":{},\"root\":\"{}\",\"production_active\":false}}", state.names().len(), state.height(), state.root()))
}
fn main() -> eyre::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() == 4 && args[0] == "fence" {
        let genesis = laboratory_genesis(args[2].parse()?, args[3] == "distinct")?;
        let journal = Journal::open(std::path::Path::new(&args[1]))?;
        fence_observer(&journal, &genesis)?;
        println!("{{\"observer_only\":true,\"identity_fenced\":true}}");
        return Ok(());
    }
    if args.len() == 6 && args[0] == "live-audit" {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        let net = network::IsolatedNetwork::load(std::path::Path::new(&args[2]), 0, now)?;
        let journal = Journal::open(std::path::Path::new(&args[1]))?;
        let state = replay_certified(&journal, net.genesis())?;
        eyre::ensure!(
            state.height().to_string() == args[3]
                && state.tip().to_string() == args[4]
                && state.root().to_string() == args[5],
            "independent live anchor mismatch"
        );
        let lab = ExperimentalState::from_verified_state(&state)?;
        let path = std::path::Path::new(&args[1]).with_extension("experimental-v2-audit");
        let at = Instant::now();
        let a = store::save(&lab, &path)?;
        let save = at.elapsed().as_secs_f64();
        let at = Instant::now();
        assert_eq!(
            store::recover(&path, net.genesis().clone(), a)?.anchor(),
            lab.anchor()
        );
        let recovery = at.elapsed().as_secs_f64();
        #[cfg(feature = "inactive-checkpoint-v2")]
        let inactive = inactive_profile(
            &journal,
            &state,
            &std::path::Path::new(&args[1]).with_extension("inactive-v2-audit"),
        )?;
        #[cfg(not(feature = "inactive-checkpoint-v2"))]
        let inactive = "null";
        println!("{{\"inactive_v2\":{inactive},\"live_engine_history_verified\":true,\"names\":{},\"accounts\":{},\"v2_save_verify_seconds\":{save},\"v2_recovery_seconds\":{recovery},\"v2_not_production_recovery\":true}}",lab.names(),lab.accounts());
        return Ok(());
    }
    if (args.len() == 8 && args[0] == "v2-recover") || (args.len() == 9 && args[0] == "v2-transfer")
    {
        let offset = usize::from(args[0] == "v2-transfer");
        let genesis =
            laboratory_genesis(args[2 + offset].parse()?, args[3 + offset] == "distinct")?;
        let anchor = yami_chain::snapshot_v2::Anchor {
            state: yami_chain::snapshot_v2::StateAnchor {
                genesis: genesis.id(),
                height: args[4 + offset].parse()?,
                tip: digest(&args[5 + offset])?,
                root: digest(&args[6 + offset])?,
            },
            manifest: digest(&args[7 + offset])?,
        };
        let state = if offset == 0 {
            store::recover(std::path::Path::new(&args[1]), genesis, anchor)?
        } else {
            store::transfer(
                std::path::Path::new(&args[1]),
                std::path::Path::new(&args[2]),
                genesis,
                anchor,
            )?
        };
        println!(
            "{{\"height\":{},\"names\":{},\"accounts\":{},\"root\":\"{}\",\"manifest\":\"{}\"}}",
            anchor.state.height,
            state.names(),
            state.accounts(),
            state.anchor().root,
            anchor.manifest
        );
        return Ok(());
    }
    // A fresh observer-only historical prefix, never a recovered signing identity.
    if args.len() == 6 && args[0] == "prefix" {
        let genesis = laboratory_genesis(args[3].parse()?, args[4] == "distinct")?;
        let height: u64 = args[5].parse()?;
        eyre::ensure!(
            height > 0 && height <= 1_000_000 && !std::path::Path::new(&args[2]).exists(),
            "bounded fresh observer prefix required"
        );
        let source = Journal::open(std::path::Path::new(&args[1]))?;
        let dest = Journal::open(std::path::Path::new(&args[2]))?;
        fence_observer(&dest, &genesis)?;
        dest.migrate_archive(genesis.clone(), |r| verify_record(&genesis, r))?;
        for h in 1..=height {
            let r = source
                .record(h)?
                .ok_or_else(|| eyre::eyre!("missing certified prefix record"))?;
            verify_record(&genesis, &r)?;
            dest.append(&r)?;
        }
        let state = replay_certified(&dest, &genesis)?;
        eyre::ensure!(state.height() == height, "incomplete prefix");
        println!(
            "{{\"height\":{},\"names\":{},\"tip\":\"{}\",\"root\":\"{}\",\"observer_only\":true}}",
            height,
            state.names().len(),
            state.tip(),
            state.root()
        );
        return Ok(());
    }
    if args.len() == 7 && args[0] == "replay" {
        let genesis = laboratory_genesis(args[2].parse()?, args[3] == "distinct")?;
        let journal = Journal::open(std::path::Path::new(&args[1]))?;
        let at = Instant::now();
        let state = replay_certified(&journal, &genesis)?;
        eyre::ensure!(
            state.height().to_string() == args[4]
                && state.tip().to_string() == args[5]
                && state.root().to_string() == args[6],
            "independent archive anchor mismatch"
        );
        println!(
            "{{\"height\":{},\"tip\":\"{}\",\"root\":\"{}\",\"names\":{},\"replay_seconds\":{}}}",
            state.height(),
            state.tip(),
            state.root(),
            state.names().len(),
            at.elapsed().as_secs_f64()
        );
        return Ok(());
    }
    if args.len() != 3 {
        eyre::bail!(
            "large-state-bench NEW_DIR ENTRIES shared|distinct; replay ARCHIVE ENTRIES shared|distinct HEIGHT TIP ROOT"
        );
    }
    let path = PathBuf::from(&args[0]);
    let count: usize = args[1].parse()?;
    let distinct = match args[2].as_str() {
        "shared" => false,
        "distinct" => true,
        _ => eyre::bail!("pattern"),
    };
    if path.exists() || ![1024, 100_000, 250_000, 500_000].contains(&count) {
        eyre::bail!("fresh path and 1024|100000|250000|500000 entries");
    }
    let future_epoch = 800_000 + count as u64 / 10 + u64::from(distinct);
    let genesis = laboratory_genesis(count, distinct)?;
    let journal = Journal::open(&path.join("archive"))?;
    fence_observer(&journal, &genesis)?;
    journal.migrate_archive(genesis.clone(), |r| verify_record(&genesis, r))?;
    let signers: Vec<_> = (0..4)
        .map(|i| {
            let j = Journal::open(&path.join(format!("fixture-signer-{i}")))?;
            j.ensure_genesis(&genesis)?;
            j.bind_validator_index(i)?;
            j.migrate_signatures()?;
            Ok::<_, std::io::Error>(j)
        })
        .collect::<std::io::Result<_>>()?;
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
            certificate: certificate(&genesis, &block, &signers)?,
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
    let v1_bytes = match snapshot {
        Ok(bytes) => {
            assert!(count <= 100_000);
            bytes.len()
        }
        Err(yami_chain::Error::Limit) => {
            assert!(count > 100_000);
            0
        }
        Err(e) => return Err(e.into()),
    };
    let at = Instant::now();
    let mut branches = Vec::new();
    for i in 0..8 {
        let tx = Transaction::sign(
            &format!("test-retained-fork-{i}"),
            0,
            state.height() + 1,
            &development_key("registry", i + 10),
        )?;
        let block = state.propose(vec![tx])?;
        branches.push(state.execute(&block)?);
    }
    let fork_seconds = at.elapsed().as_secs_f64();
    assert!(branches
        .iter()
        .all(|s| s.tip() != state.tip() && s.names().len() == count + 1));
    assert_eq!(state.names().len(), count);
    let at = Instant::now();
    for _ in 0..3 {
        let empty = state.propose(Vec::new())?;
        assert_ne!(empty.header.state, root);
    }
    let hash_seconds = at.elapsed().as_secs_f64() / 3.0;
    let (uncached_admission, cached_admission) =
        profile_candidate_retransmission(&state, &path.join("proposal-profile"))?;
    let lab = ExperimentalState::from_verified_state(&state)?;
    let at = Instant::now();
    let anchor = store::save(&lab, &path.join("v2"))?;
    let v2_save = at.elapsed().as_secs_f64();
    let at = Instant::now();
    let recovered = store::recover(&path.join("v2"), genesis.clone(), anchor)?;
    assert_eq!(recovered.anchor(), lab.anchor());
    let v2_recover = at.elapsed().as_secs_f64();
    drop(recovered);
    let at = Instant::now();
    let copied = store::transfer(
        &path.join("v2"),
        &path.join("v2-received"),
        genesis.clone(),
        anchor,
    )?;
    assert_eq!(copied.anchor(), lab.anchor());
    let v2_transfer = at.elapsed().as_secs_f64();
    drop(copied);
    #[cfg(feature = "inactive-checkpoint-v2")]
    let inactive = inactive_profile(&journal, &state, &path)?;
    #[cfg(not(feature = "inactive-checkpoint-v2"))]
    let inactive = "null";
    drop(branches);
    drop(journal);
    let at = Instant::now();
    let journal = Journal::open(&path.join("archive"))?;
    assert_eq!(replay_certified(&journal, &genesis)?, state);
    let replay = at.elapsed().as_secs_f64();
    let footprint = journal.storage_footprint(1_000_000)?;
    println!("{{\"inactive_v2\":{inactive},\"entries\":{count},\"future_epoch\":{future_epoch},\"accounts\":{},\"pattern\":\"{}\",\"height\":{},\"root\":\"{root}\",\"tip\":\"{}\",\"v1_bytes\":{v1_bytes},\"generation_seconds\":{generation},\"propose_seconds\":{propose},\"execute_seconds\":{execute},\"certificate_persistence_seconds\":{persistence},\"cached_root_seconds\":{first_root},\"repeated_roots_100_seconds\":{repeated_root},\"clones_8_seconds\":{clone_seconds},\"lookups_1024_seconds\":{lookups},\"snapshot_serialization_seconds\":{serialization},\"retained_divergent_forks_8_seconds\":{fork_seconds},\"empty_proposal_hash_seconds\":{hash_seconds},\"uncached_retransmission_mean_seconds\":{uncached_admission},\"cached_retransmission_mean_seconds\":{cached_admission},\"v2_save_verify_seconds\":{v2_save},\"v2_recovery_seconds\":{v2_recover},\"v2_transfer_verify_seconds\":{v2_transfer},\"v2_manifest\":\"{}\",\"archive_replay_seconds\":{replay},\"archive_allocated_bytes\":{},\"archive_entries\":{},\"certificate_signers\":4,\"transactions_genuinely_signed\":true,\"certificates_verified\":true,\"live_engine_generated\":false,\"passed\":true}}",if distinct { count } else {1},args[2],state.height(),state.tip(),anchor.manifest,footprint.allocated_bytes,footprint.entries);
    Ok(())
}
