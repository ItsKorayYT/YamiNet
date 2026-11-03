use std::{collections::BTreeSet, path::Path, time::Instant};
use yamichain_ypp_hidden::*;
fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}
fn rss() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|n| n.parse().ok())
        })
        .unwrap_or(0)
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|n| format!("{n:02x}")).collect()
}
fn serialized<T: ark_serialize::CanonicalSerialize>(v: &T) -> Vec<u8> {
    let mut b = Vec::new();
    v.serialize_compressed(&mut b).unwrap();
    b
}
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("verify") {
        let dir = Path::new(args.get(2).ok_or("missing public fixture directory")?);
        let now = Instant::now();
        let v = Verifier::decode(&std::fs::read(dir.join("verifier.bin"))?)?;
        let t = Transaction::decode(&std::fs::read(dir.join("transaction.bin"))?)?;
        let load = ms(now);
        let mut samples = Vec::new();
        for _ in 0..20 {
            let now = Instant::now();
            v.verify(&t)?;
            samples.push(ms(now));
        }
        println!("{{\"phase\":\"verifier_only\",\"load_ms\":{load},\"verification_ms\":{samples:?},\"peak_rss_kib\":{}}}",rss());
        return Ok(());
    }
    eprintln!("YPP2 synthetic private-record experiment: hidden paths, note-like credits, no consensus. Starting local trusted setup.");
    let now = Instant::now();
    let (p, v, params, constraints) = setup()?;
    let setup_ms = ms(now);
    eprintln!("Setup: {setup_ms:.0} ms; {constraints} constraints.");
    let wallets: Vec<_> = (0..4).map(|_| Wallet::new()).collect();
    let refs: Vec<_> = wallets.iter().collect();
    let (g, mut states) = synthetic_genesis(params, &refs, &[100; 4])?;
    let mut ledger = Ledger::from_genesis(&g)?;
    let now = Instant::now();
    let d = Decoder::default();
    let decoder_setup_ms = ms(now);
    let now = Instant::now();
    let mut scans: Vec<_> = wallets
        .iter()
        .map(|w| Scanner::new(&w.viewer(), &g, g.checkpoint(), &d))
        .collect::<Result<Vec<_>>>()?;
    let genesis_scan_ms = ms(now);
    let initial = ledger.snapshot();
    let mut proving = Vec::new();
    let mut verification = Vec::new();
    let mut scanning = Vec::new();
    let mut prepared = Vec::new();
    for (sender, recipient, amount) in [(0, 1, 7), (2, 3, 4), (1, 2, 2)] {
        let now = Instant::now();
        let (t, new) = p.transfer(
            &wallets[sender],
            &initial,
            &states[sender],
            None,
            &wallets[recipient].address(),
            amount,
        )?;
        proving.push(ms(now));
        states[sender] = new;
        prepared.push(t);
    }
    for t in prepared {
        assert_eq!(t.anchor, initial.root());
        let now = Instant::now();
        v.verify(&t)?;
        verification.push(ms(now));
        ledger.apply(&v, t)?;
        let now = Instant::now();
        for (i, s) in scans.iter_mut().enumerate() {
            s.advance(
                &wallets[i].viewer(),
                &v,
                ledger.records().last().unwrap(),
                &d,
            )?;
        }
        scanning.push(ms(now));
    }
    for (sender, recipient, amount, absorb) in [(1, 2, 1, true), (0, 1, 3, false), (2, 3, 2, true)]
    {
        let credit = if absorb {
            Some(scans[sender].credits()[0].clone())
        } else {
            None
        };
        let now = Instant::now();
        let (t, new) = p.transfer(
            &wallets[sender],
            &ledger.snapshot(),
            &states[sender],
            credit.as_ref(),
            &wallets[recipient].address(),
            amount,
        )?;
        proving.push(ms(now));
        states[sender] = new;
        let now = Instant::now();
        v.verify(&t)?;
        verification.push(ms(now));
        ledger.apply(&v, t)?;
        let now = Instant::now();
        for (i, s) in scans.iter_mut().enumerate() {
            s.advance(
                &wallets[i].viewer(),
                &v,
                ledger.records().last().unwrap(),
                &d,
            )?;
        }
        scanning.push(ms(now));
    }
    let total: u64 = scans
        .iter()
        .map(|s| {
            u64::from(s.vault().unwrap().amount())
                + s.credits()
                    .iter()
                    .map(|o| u64::from(o.amount()))
                    .sum::<u64>()
        })
        .sum();
    assert_eq!(total, 400);
    let identity = age::x25519::Identity::generate();
    let backup = wallets[1].backup(&identity.to_public())?;
    let now = Instant::now();
    let restored = Wallet::restore(&identity, &backup)?;
    let backup_restore_ms = ms(now);
    let now = Instant::now();
    let mut recovered = Scanner::new(&restored.viewer(), &g, g.checkpoint(), &d)?;
    for r in ledger.records() {
        recovered.advance(&restored.viewer(), &v, r, &d)?;
    }
    recovered.require_head(ledger.head())?;
    assert_eq!(recovered.vault()?.commitment(), states[1].commitment());
    let recovery_ms = ms(now);
    let mut nf = BTreeSet::new();
    let mut commitments = BTreeSet::new();
    let mut ephemerals = BTreeSet::new();
    let mut anchors = BTreeSet::new();
    let mut repeats = 0;
    let mut ephemeral_repeats = 0;
    let mut transcript = Vec::new();
    for r in ledger.records() {
        let t = &r.transaction;
        anchors.insert(serialized(&t.anchor));
        for n in t.nullifiers {
            if !nf.insert(n) {
                repeats += 1;
            }
        }
        for o in &t.outputs {
            commitments.insert(o.commitment);
            for c in &o.delivery {
                if !ephemerals.insert(serialized(&c.0)) {
                    ephemeral_repeats += 1;
                }
            }
        }
        let nullifiers = t.nullifiers.map(|n| format!("\"{}\"", hex(&n))).join(",");
        let outputs = t
            .outputs
            .iter()
            .map(|o| format!("\"{}\"", hex(&o.commitment)))
            .collect::<Vec<_>>()
            .join(",");
        transcript.push(format!("{{\"anchor\":\"{}\",\"nullifiers\":[{nullifiers}],\"output_commitments\":[{outputs}],\"packet_hex\":\"{}\"}}",hex(&serialized(&t.anchor)),hex(&t.encode())));
    }
    let direct_links = nf.intersection(&commitments).count();
    assert_eq!((repeats, ephemeral_repeats, direct_links), (0, 0, 0));
    if let Some(path) = args.get(2) {
        let dir = Path::new(path);
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join("verifier.bin"), v.encode())?;
        std::fs::write(
            dir.join("transaction.bin"),
            ledger.records()[0].transaction.encode(),
        )?;
    }
    eprintln!("Six real transfers, four wallets, three unchanged same-anchor proofs accepted, repeated Alice payments, offline receipts, backup restore and history recovery verified. No direct commitment/nullifier equality links.");
    println!("{{\"phase\":\"prover_and_recovery\",\"wallets\":4,\"transitions\":6,\"constraints\":{constraints},\"setup_ms\":{setup_ms},\"proving_ms\":{proving:?},\"verification_ms\":{verification:?},\"packet_bytes\":{PACKET_BYTES},\"proof_bytes\":192,\"scan_four_wallets_record_ms\":{scanning:?},\"decoder_setup_ms\":{decoder_setup_ms},\"genesis_four_wallets_scan_ms\":{genesis_scan_ms},\"backup_restore_ms\":{backup_restore_ms},\"recovery_one_wallet_ms\":{recovery_ms},\"leaves\":{},\"leaf_commitment_bytes\":{},\"retained_root_bytes\":{},\"nullifier_bytes\":{},\"history_packet_bytes\":{},\"history_root_head_bytes\":{},\"peak_rss_kib\":{},\"transcript_analysis\":{{\"direct_output_to_nullifier_links\":{direct_links},\"repeated_accepted_nullifiers\":{repeats},\"repeated_ephemeral_keys\":{ephemeral_repeats},\"distinct_anchors\":{},\"input_index_or_old_commitment_field\":false,\"output_roles_public\":true}},\"public_transcript\":[{}]}}",ledger.leaf_count(),ledger.leaf_count()*32,ROOT_WINDOW*32,ledger.marker_count()*32,ledger.records().len()*PACKET_BYTES,ledger.records().len()*64,rss(),anchors.len(),transcript.join(","));
    Ok(())
}
