//! Offline lab data only. Fixtures have independent flat roots, NOT BFT finality.
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::Instant,
};
use yami_chain::{
    codec::Writer,
    hash,
    snapshot_v2::{store, Anchor, ExperimentalState, Manifest, StateAnchor},
    Digest, Error, Genesis, SigningKey,
};
fn digest(text: &str) -> Result<Digest, Box<dyn std::error::Error>> {
    if text.len() != 64 {
        return Err("64 hex digits required".into());
    }
    let mut bytes = [0; 32];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16)?;
    }
    Ok(Digest(bytes))
}
fn expected(args: &[String]) -> Result<Anchor, Box<dyn std::error::Error>> {
    Ok(Anchor {
        state: StateAnchor {
            genesis: Genesis::local().id(),
            height: args[0].parse()?,
            tip: digest(&args[1])?,
            root: digest(&args[2])?,
        },
        manifest: digest(&args[3])?,
    })
}
fn fixture(n: usize) -> Result<ExperimentalState, Box<dyn std::error::Error>> {
    let owners: Vec<_> = (0..n)
        .map(|i| {
            SigningKey::from_bytes(&hash(b"YC-V2-FIXTURE-KEY-1", &(i as u64).to_be_bytes()).0)
                .verifying_key()
                .to_bytes()
        })
        .collect();
    let mut sorted = owners.clone();
    sorted.sort();
    let genesis = Genesis::local();
    let height = (n as u64).div_ceil(64);
    let tip = hash(b"YC-V2-FIXTURE-TIP-1", &(n as u64).to_be_bytes());
    // Independent reference: the unchanged flat v1 root preimage, including both
    // sorted maps. This buffer belongs to the fixture author, never the decoder.
    let mut w = Writer::default();
    w.raw(&genesis.id().0);
    w.u64(height);
    w.u32(n as u32);
    for key in &sorted {
        w.raw(key);
        w.u64(1);
    }
    w.u32(n as u32);
    for (i, key) in owners.iter().enumerate() {
        w.bytes(format!("test-v2-{i:055}").as_bytes())?;
        w.raw(key);
    }
    let root = hash(b"yamichain-state-v1", &w.0);
    drop(w);
    let state = ExperimentalState::from_ordered_records(
        genesis,
        StateAnchor {
            genesis: Genesis::local().id(),
            height,
            tip,
            root,
        },
        sorted.into_iter().map(|k| (k, 1)),
        owners
            .into_iter()
            .enumerate()
            .map(|(i, k)| (format!("test-v2-{i:055}"), k)),
    )?;
    Ok(state)
}
fn print_state(state: &ExperimentalState, anchor: Anchor) {
    println!("{{\"height\":{},\"tip\":\"{}\",\"root\":\"{}\",\"manifest\":\"{}\",\"accounts\":{},\"names\":{},\"experimental_only\":true}}",anchor.state.height,anchor.state.tip,anchor.state.root,anchor.manifest,state.accounts(),state.names());
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("recover") && args.len() == 6 {
        let anchor = expected(&args[2..])?;
        let state = store::recover(Path::new(&args[1]), Genesis::local(), anchor)?;
        print_state(&state, anchor);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("transfer") && (args.len() == 7 || args.len() == 9)
    {
        let anchor = expected(&args[3..7])?;
        let state = store::transfer_observed(
            Path::new(&args[1]),
            Path::new(&args[2]),
            Genesis::local(),
            anchor,
            |file, stage| {
                if args.len() == 9 && file == args[7] && format!("{stage:?}") == args[8] {
                    std::process::exit(86);
                }
            },
        )?;
        print_state(&state, anchor);
        return Ok(());
    }
    if args.len() != 3 || args[0] != "build" {
        return Err("snapshot-v2-bench build NEW_DIR 100000|250000|500000; recover DIR HEIGHT TIP ROOT MANIFEST; transfer SOURCE DEST HEIGHT TIP ROOT MANIFEST [FILE STAGE]".into());
    }
    let path = PathBuf::from(&args[1]);
    let n: usize = args[2].parse()?;
    if path.exists() || ![100_000, 250_000, 500_000].contains(&n) {
        return Err("new path and requested capacity required".into());
    }
    fs::create_dir_all(&path)?;
    let at = Instant::now();
    let state = fixture(n)?;
    let fixture_seconds = at.elapsed().as_secs_f64();
    let at = Instant::now();
    for _ in 0..3 {
        assert_eq!(state.uncached_root(), state.anchor().root);
    }
    let root_seconds = at.elapsed().as_secs_f64() / 3.;
    let at = Instant::now();
    let mut next = None;
    for _ in 0..3 {
        let value = state.next_empty_block_root()?;
        assert_ne!(value, state.anchor().root);
        if let Some(expected) = next {
            assert_eq!(value, expected);
        }
        next = Some(value);
    }
    let empty_proposal = at.elapsed().as_secs_f64() / 3.;
    let v1 = state.v1_snapshot();
    let v1_bytes = if n <= 100_000 {
        let bytes = v1?;
        let restored = yami_chain::State::restore_checkpoint(
            Genesis::local(),
            state.anchor().height,
            state.anchor().tip,
            state.anchor().root,
            &bytes,
        )?;
        assert_eq!(restored.root(), state.anchor().root);
        bytes.len()
    } else {
        assert_eq!(v1, Err(Error::Limit));
        0
    };
    let at = Instant::now();
    let anchor = store::save(&state, &path.join("source"))?;
    let save_seconds = at.elapsed().as_secs_f64();
    let manifest = Manifest::decode(&fs::read(path.join("source/manifest.bin"))?, anchor)?;
    let at = Instant::now();
    let transferred = store::transfer(
        &path.join("source"),
        &path.join("received"),
        Genesis::local(),
        anchor,
    )?;
    let transfer_seconds = at.elapsed().as_secs_f64();
    assert_eq!(transferred.anchor(), anchor.state);
    drop(transferred);
    let at = Instant::now();
    for _ in 0..3 {
        let recovered = store::recover(&path.join("received"), Genesis::local(), anchor)?;
        assert_eq!(recovered.anchor(), anchor.state);
        assert_eq!(recovered.accounts(), n);
        assert_eq!(recovered.names(), n);
        assert_eq!(recovered.v1_snapshot().is_ok(), n <= 100_000);
    }
    let recovery = at.elapsed().as_secs_f64() / 3.;
    let logical: u64 = fs::read_dir(path.join("source"))?
        .map(|e| e.and_then(|e| e.metadata()).map(|m| m.len()))
        .sum::<io::Result<u64>>()?;
    #[cfg(unix)]
    let allocated: u64 = {
        use std::os::unix::fs::MetadataExt;
        fs::read_dir(path.join("source"))?
            .map(|e| e.and_then(|e| e.metadata()).map(|m| m.blocks() * 512))
            .sum::<io::Result<u64>>()?
    };
    #[cfg(not(unix))]
    let allocated = logical;
    // Verify all representative queries against their known fixture owners.
    let owner = SigningKey::from_bytes(&hash(b"YC-V2-FIXTURE-KEY-1", &0u64.to_be_bytes()).0)
        .verifying_key()
        .to_bytes();
    assert_eq!(state.name(&format!("test-v2-{:055}", 0)), Some(&owner));
    assert_eq!(state.nonce(&owner), 1);
    assert_eq!(state.name("test-absent"), None);
    println!("{{\"entries\":{n},\"accounts\":{n},\"label_bytes\":63,\"height\":{},\"tip\":\"{}\",\"root\":\"{}\",\"manifest\":\"{}\",\"fixture_seconds\":{fixture_seconds},\"uncached_root_mean_seconds\":{root_seconds},\"next_empty_proposal_mean_seconds\":{empty_proposal},\"save_verify_seconds\":{save_seconds},\"transfer_verify_seconds\":{transfer_seconds},\"recovery_mean_seconds\":{recovery},\"payload_bytes\":{},\"chunks\":{},\"manifest_bytes\":{},\"logical_store_bytes\":{logical},\"allocated_store_bytes\":{allocated},\"v1_bytes\":{v1_bytes},\"v1_limits_preserved\":true,\"experimental_only\":true,\"bft_finality_claimed\":false,\"passed\":true}}",anchor.state.height,anchor.state.tip,anchor.state.root,anchor.manifest,manifest.payload_bytes(),manifest.chunks(),manifest.canonical().len());
    Ok(())
}
