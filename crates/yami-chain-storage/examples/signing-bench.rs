//! Private storage-only workload: opaque 158-byte preimages, never actual votes.
use std::{fs, io, path::PathBuf, time::Instant};
use yami_chain_storage::{Journal, StorageFootprint};
fn preimage(h: u64) -> Vec<u8> {
    let mut b = vec![7; 158];
    b[..8].copy_from_slice(&h.to_be_bytes());
    b
}
fn footprint(j: &Journal) -> io::Result<StorageFootprint> {
    j.storage_footprint(4_000_000)
}
fn main() -> io::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err(io::Error::other(
            "signing-bench fresh|legacy|migrate|probe DIR COUNT",
        ));
    }
    let mode = args[0].as_str();
    let path = PathBuf::from(&args[1]);
    let count: u64 = args[2].parse().map_err(io::Error::other)?;
    if !["fresh", "legacy", "migrate", "probe"].contains(&mode) || !(1..=1_000_000).contains(&count)
    {
        return Err(io::Error::other("invalid signing benchmark workload"));
    }
    if mode == "probe" {
        let j = Journal::open(&path)?;
        let before = j
            .signing_anchor()?
            .ok_or_else(|| io::Error::other("segmented fixture required"))?;
        let old_ok = j.reserve_signature(1, 0, 2, &preimage(1)).is_ok();
        let error = j
            .reserve_signature(count + 1, 0, 2, b"new")
            .expect_err("pressure must refuse");
        let after = j
            .signing_anchor()?
            .ok_or_else(|| io::Error::other("missing signing anchor"))?;
        println!("{{\"passed\":{},\"idempotent_ok\":{old_ok},\"storage_full\":{},\"count_unchanged\":{}}}",old_ok&&error.kind()==io::ErrorKind::StorageFull&&before==after,error.kind()==io::ErrorKind::StorageFull,before==after);
        return Ok(());
    }
    if mode != "migrate" && path.exists() {
        return Err(io::Error::other(
            "fresh signing benchmark directory required",
        ));
    }
    let j = Journal::open(&path)?;
    j.bind_validator_index(0)?;
    j.replay(
        yami_chain::Genesis::development_schedule(&[(1, 4)]).map_err(io::Error::other)?,
        |_| Ok(()),
    )?;
    let before = footprint(&j)?;
    let mut migration_seconds = 0.0;
    let mut legacy_count = 0;
    let mut migrated_allocated = before.allocated_bytes;
    let mut migrated_entries = before.entries;
    if mode == "migrate" {
        let original: Vec<_> = fs::read_dir(&path)?
            .map(|e| e.map(|e| e.path()))
            .collect::<io::Result<_>>()?;
        let hashes: Vec<_> = original
            .into_iter()
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("signed-")
            })
            .map(|p| {
                Ok((
                    p.clone(),
                    yami_chain::hash(b"fixture-legacy", &fs::read(p)?),
                ))
            })
            .collect::<io::Result<_>>()?;
        let start = Instant::now();
        legacy_count = j.migrate_signatures()?.reservations;
        migration_seconds = start.elapsed().as_secs_f64();
        for (p, digest) in hashes {
            assert_eq!(yami_chain::hash(b"fixture-legacy", &fs::read(p)?), digest);
        }
        let middle = footprint(&j)?;
        migrated_allocated = middle.allocated_bytes;
        migrated_entries = middle.entries;
    }
    let start = Instant::now();
    for h in legacy_count + 1..=count {
        if mode == "legacy" {
            // Match M10's full pressure audit before each immutable per-file
            // reservation; the representation, fsync and filenames are unchanged.
            j.ensure_capacity(158)?;
            j.write_once(&format!("signed-{h:020}-0000000000-2.bin"), &preimage(h))?;
        } else {
            j.reserve_signature(h, 0, 2, &preimage(h))?;
        }
    }
    let generation = start.elapsed().as_secs_f64();
    let audits = j.accounting_audits()?;
    let anchor = j.signing_anchor()?;
    let after = footprint(&j)?;
    let stats = j.signing_stats()?;
    drop(j);
    let mut recovery = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        let reopened = Journal::open(&path)?;
        if let Some(a) = anchor {
            reopened.verify_signing_anchor(a)?;
        } else {
            // Baseline recovery timing explicitly includes checking EVERY known
            // preimage, unlike M10's directory-only Journal::open timing.
            for h in 1..=count {
                assert_eq!(
                    reopened.signature_reservation(h, 0, 2)?.unwrap(),
                    preimage(h)
                );
            }
        }
        recovery.push(start.elapsed().as_secs_f64());
        for h in [1, count / 2 + 1, count] {
            assert_eq!(
                reopened.signature_reservation(h, 0, 2)?.unwrap(),
                preimage(h)
            );
            if mode != "legacy" {
                assert!(reopened.reserve_signature(h, 0, 2, b"conflict").is_err());
            }
        }
    }
    let mean = recovery.iter().sum::<f64>() / recovery.len() as f64;
    let segments = stats.as_ref().map_or(0, |s| s.segments);
    let payload = stats.as_ref().map_or(count * 158, |s| s.preimage_bytes);
    let legacy = if mode == "legacy" {
        count
    } else {
        legacy_count
    };
    println!("{{\"mode\":\"{mode}\",\"reservations\":{count},\"preimage_bytes\":{payload},\"generation_seconds\":{generation},\"migration_seconds\":{migration_seconds},\"before_allocated_bytes\":{},\"before_entries\":{},\"migrated_allocated_bytes\":{migrated_allocated},\"migrated_entries\":{migrated_entries},\"allocated_bytes\":{},\"logical_bytes\":{},\"entries\":{},\"segments\":{segments},\"legacy_files_retained\":{legacy},\"accounting_audits\":{audits},\"recovery_mean_seconds\":{mean},\"restart_count\":5,\"passed\":true}}",before.allocated_bytes,before.entries,after.allocated_bytes,after.bytes,after.entries);
    Ok(())
}
