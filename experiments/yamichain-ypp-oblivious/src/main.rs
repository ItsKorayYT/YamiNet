use serde_json::{json, Value};
use std::{fs, time::Instant};
use yamichain_ypp_oblivious::*;

fn elapsed_us(start: Instant) -> u128 {
    start.elapsed().as_micros()
}

fn fixture(n: usize) -> (Vec<Wallet>, Vec<Cell>, StorageLab) {
    let wallets: Vec<_> = (0..n).map(Wallet::new).collect();
    let cells: Vec<_> = wallets
        .iter()
        .map(|w| {
            w.encrypt(&PlainState {
                version: 0,
                counter: 7,
            })
        })
        .collect();
    let lab = StorageLab::new(&cells);
    (wallets, cells, lab)
}

fn demo(directory: &str) -> Result<Value, Box<dyn std::error::Error>> {
    fs::create_dir_all(directory)?;
    let p = Parameters::default();
    let (wallets, cells, mut lab) = fixture(4);
    let next_a = wallets[0].encrypt(&PlainState {
        version: 1,
        counter: 8,
    });
    let next_b = wallets[3].encrypt(&PlainState {
        version: 1,
        counter: 9,
    });
    let (a, ma) = prepare(&p, 4, 0, delta(next_a, cells[0]))?;
    let (b, mb) = prepare(&p, 4, 3, delta(next_b, cells[3]))?;
    let mut reverse = lab.clone();
    lab.apply(&p, &a, &ma)?;
    lab.apply(&p, &b, &mb)?;
    reverse.apply(&p, &b, &mb)?;
    reverse.apply(&p, &a, &ma)?;
    let both_survive = recover(&lab.head(), &lab.snapshots(), &wallets[0])?.counter == 8
        && recover(&lab.head(), &lab.snapshots(), &wallets[3])?.counter == 9;

    let (cw, cc, mut conflicting) = fixture(4);
    let x = cw[1].encrypt(&PlainState {
        version: 1,
        counter: 11,
    });
    let y = cw[1].encrypt(&PlainState {
        version: 1,
        counter: 12,
    });
    let (px, mx) = prepare(&p, 4, 1, delta(x, cc[1]))?;
    let (py, my) = prepare(&p, 4, 1, delta(y, cc[1]))?;
    conflicting.apply(&p, &px, &mx)?;
    conflicting.apply(&p, &py, &my)?;

    let mut public_packets = vec![a, b, px, py];
    let mut exposed_channels = Vec::new();
    // The labels are kept separately from inputs passed to the observer.
    let labels = [2, 2, 0, 1, 0, 3, 3, 1];
    let mut predictions = Vec::new();
    let (trace_wallets, mut trace_cells, mut trace_lab) = fixture(4);
    let mut snapshots = vec![trace_lab.snapshots()];
    let mut heads = vec![trace_lab.head()];
    let mut snapshot_predictions = vec![];
    let mut versions = [0; 4];
    for target in labels {
        versions[target] += 1;
        let next = trace_wallets[target].encrypt(&PlainState {
            version: versions[target],
            counter: 7,
        });
        let (packet, messages) = prepare(&p, 4, target, delta(next, trace_cells[target]))?;
        let channels = messages.each_ref().map(PrivateWrite::wire);
        predictions.push(reconstruct_support(&messages));
        trace_lab.apply(&p, &packet, &messages)?;
        let after = trace_lab.snapshots();
        snapshot_predictions.push(snapshot_support(
            snapshots.last().ok_or("missing initial snapshot")?,
            &after,
        )?);
        snapshots.push(after);
        heads.push(trace_lab.head());
        trace_cells[target] = next;
        exposed_channels.push(channels);
        public_packets.push(packet);
    }
    fs::write(
        format!("{directory}/downloadable-snapshots.json"),
        serde_json::to_vec(&snapshots)?,
    )?;
    fs::write(
        format!("{directory}/heads.json"),
        serde_json::to_vec(&heads)?,
    )?;
    fs::write(
        format!("{directory}/packets.json"),
        serde_json::to_vec(&public_packets)?,
    )?;
    // Synthetic disposable data ONLY. Includes share commitment blinds, never wallet keys.
    // This separate file is the failed public-validation/collusion transcript.
    fs::write(
        format!("{directory}/exposed-channels.json"),
        serde_json::to_vec(&exposed_channels)?,
    )?;
    let report = json!({
        "decision": "REJECT tested two-server private-write composition",
        "shape_proof_packets": public_packets.len(),
        "independent_updates_survive": both_survive,
        "independent_update_order_independent": lab.head().hashes == reverse.head().hashes,
        "conflicting_shape_valid_updates_both_accepted": true,
        "conflicting_cell_authentication_fails": recover(&conflicting.head(), &conflicting.snapshots(), &cw[1]).is_err(),
        "observer_labels_separate_from_inputs": labels,
        "observer_predictions_from_two_share_vectors": predictions,
        "observer_predictions_from_downloadable_snapshots": snapshot_predictions,
        "snapshot_attack_needs_colluding_servers": false,
        "repeated_update_pair_classification_correct": (0..labels.len()).all(|i| (0..labels.len()).all(|j| (labels[i] == labels[j]) == (snapshot_predictions[i] == snapshot_predictions[j]))),
        "authenticated_unrelated_wallet_recovery": recover(&lab.head(), &lab.snapshots(), &wallets[2]).is_ok(),
        "privacy_boundary": "packets.json contains hiding commitments; exposed-channels.json models public channel reconstruction or colluding servers",
        "proof_scope": "write shape only, no owner/current-version/state-head authorization",
        "no_financial_functionality": true
    });
    fs::write(
        format!("{directory}/report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}

fn measure(n: usize, repeats: usize) -> Result<Value, Box<dyn std::error::Error>> {
    if n < 2 || repeats == 0 {
        return Err("n >= 2 and repeats > 0 required".into());
    }
    let p = Parameters::default();
    let mut rows = vec![];
    for _ in 0..repeats {
        let (wallets, cells, mut lab) = fixture(n);
        let next = wallets[n - 1].encrypt(&PlainState {
            version: 1,
            counter: 8,
        });
        let start = Instant::now();
        let (packet, messages) = prepare(&p, n, n - 1, delta(next, cells[n - 1]))?;
        let prepare_us = elapsed_us(start);
        let start = Instant::now();
        verify(&p, &packet)?;
        let verify_us = elapsed_us(start);
        let start = Instant::now();
        for (s, message) in messages.iter().enumerate() {
            verify_share(&p, &packet, s, message)?;
        }
        let share_check_us = elapsed_us(start);
        let start = Instant::now();
        lab.apply(&p, &packet, &messages)?;
        let apply_including_reverification_us = elapsed_us(start);
        let snapshots = lab.snapshots();
        let head = lab.head();
        let start = Instant::now();
        let recovered = recover(&head, &snapshots, &wallets[n - 1])?;
        let recover_us = elapsed_us(start);
        assert_eq!(
            recovered,
            PlainState {
                version: 1,
                counter: 8
            }
        );
        let start = Instant::now();
        let support = reconstruct_support(&messages);
        let observer_us = elapsed_us(start);
        assert_eq!(support, vec![n - 1]);
        rows.push(json!({
            "prepare_including_shape_proof_us": prepare_us, "verify_shape_us": verify_us,
            "check_both_private_messages_us": share_check_us,
            "apply_including_reverification_us": apply_including_reverification_us,
            "authenticated_recovery_us": recover_us,
            "exposed_channels_observer_us": observer_us,
            "json_packet_bytes": serde_json::to_vec(&packet)?.len(),
            "json_both_private_messages_bytes": serde_json::to_vec(&messages.each_ref().map(PrivateWrite::wire))?.len(),
            "json_write_total_bytes": serde_json::to_vec(&packet)?.len() + serde_json::to_vec(&messages.each_ref().map(PrivateWrite::wire))?.len(),
            "json_download_both_snapshots_bytes": serde_json::to_vec(&snapshots)?.len(),
            "json_checkpoint_bytes": serde_json::to_vec(&head)?.len()
        }));
    }
    Ok(json!({
        "vaults": n, "samples": rows,
        "canonical_shape_proof_bytes": proof_bytes(n),
        "canonical_public_packet_bytes": public_packet_bytes(n),
        "private_channel_total_bytes": private_write_bytes(n),
        "write_total_bytes_excluding_transport": public_packet_bytes(n) + private_write_bytes(n),
        "full_download_two_snapshots_bytes": 16 + 128 * n,
        "checkpoint_bytes": 80,
        "wallet_persistent_bytes_key_index_checkpoint_cached_cell": 184,
        "client_dense_write_share_material_bytes": private_write_bytes(n),
        "client_proof_response_challenge_material_bytes": proof_bytes(n),
        "prover_announcements_compressed_equivalent_bytes": 32 * n * (n-1),
        "prover_announcements_native_payload_bytes": std::mem::size_of::<curve25519_dalek::ristretto::RistrettoPoint>() * n * (n-1),
        "per_server_cipher_share_storage_bytes_excluding_replay_set": 64 * n,
        "authentication_scope": "full snapshot hash vs externally pinned head and AEAD, not a consensus transition proof",
        "timings_scope": "single-process release CPU; not network/distributed consensus or payment proof timings"
    }))
}

fn verify_fixture(directory: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let p = Parameters::default();
    let packets: Vec<Packet> =
        serde_json::from_slice(&fs::read(format!("{directory}/packets.json"))?)?;
    for packet in &packets {
        verify(&p, packet)?;
    }
    let channels: Vec<[WirePrivateWrite; 2]> =
        serde_json::from_slice(&fs::read(format!("{directory}/exposed-channels.json"))?)?;
    if packets.len() != channels.len() + 4 {
        return Err("packet/channel count".into());
    }
    let mut positions = vec![];
    for (pair, packet) in channels.iter().zip(&packets[4..]) {
        let messages = [
            PrivateWrite::from_wire(&pair[0])?,
            PrivateWrite::from_wire(&pair[1])?,
        ];
        for (s, message) in messages.iter().enumerate() {
            verify_share(&p, packet, s, message)?;
        }
        positions.push(reconstruct_support(&messages));
    }
    let snapshots: Vec<[Snapshot; 2]> = serde_json::from_slice(&fs::read(format!(
        "{directory}/downloadable-snapshots.json"
    ))?)?;
    let heads: Vec<Head> = serde_json::from_slice(&fs::read(format!("{directory}/heads.json"))?)?;
    if snapshots.len() != heads.len() || snapshots.len() != channels.len() + 1 {
        return Err("snapshot count".into());
    }
    for (pair, head) in snapshots.iter().zip(&heads) {
        if pair.each_ref().map(Snapshot::hash) != head.hashes
            || pair
                .iter()
                .any(|s| s.epoch != head.epoch || s.cells.len() != head.n)
        {
            return Err("fixture snapshot hash".into());
        }
    }
    for (epoch, wire_messages) in channels.iter().enumerate() {
        for (s, wire_message) in wire_messages.iter().enumerate() {
            verify_snapshot_transition(
                &snapshots[epoch][s],
                &snapshots[epoch + 1][s],
                &PrivateWrite::from_wire(wire_message)?,
            )?;
        }
    }
    let snapshot_positions: Vec<_> = snapshots
        .windows(2)
        .map(|pair| snapshot_support(&pair[0], &pair[1]))
        .collect::<Result<_, _>>()?;
    if positions != snapshot_positions {
        return Err("attacks disagree".into());
    }
    Ok(
        json!({"valid_shape_proofs": packets.len(), "exposed_channel_positions": positions,
        "downloadable_snapshot_positions": snapshot_positions,
        "fixture_checkpoint_hashes_checked": heads.len(),
        "disclosed_share_snapshot_transitions_checked": channels.len() * 2,
        "scope": "Checks hashes against fixture heads, not independent finality or ownership/state authorization"}),
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let result = match args.get(1).map(String::as_str) {
        Some("demo") => demo(args.get(2).ok_or("demo DIRECTORY")?)?,
        Some("verify") => verify_fixture(args.get(2).ok_or("verify DIRECTORY")?)?,
        Some("measure") => measure(
            args.get(2).ok_or("measure N REPEATS")?.parse()?,
            args.get(3).ok_or("measure N REPEATS")?.parse()?,
        )?,
        _ => return Err("usage: demo DIRECTORY | verify DIRECTORY | measure N REPEATS".into()),
    };
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
