use serde_json::{json, Value};
use std::{path::Path, time::Instant};
use yamichain_ypp_feasibility::{archive, liveness, merkle::*, polynomial};
fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn rss() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(0)
}
fn archives() -> Vec<Value> {
    [1024usize,65536,100000].into_iter().map(|n|{let mut a=archive::Archive::new();let first=hash(&[&0u64.to_le_bytes()]);let now=Instant::now();for i in 0..n{a.append(hash(&[&(i as u64).to_le_bytes()]));}let append_ms=ms(now);let now=Instant::now();let p=a.proof(0,first).unwrap();let refresh_ms=ms(now);let now=Instant::now();assert!(archive::Archive::verify(a.checkpoint(),&p));let verify_ms=ms(now);json!({"anchors":n,"total_append_ms":append_ms,"public_witness_refresh_ms":refresh_ms,"membership_verification_ms":verify_ms,"sibling_hashes":p.siblings.len(),"sibling_bytes":p.siblings.len()*32,"stored_hash_bytes":a.logical_hash_bytes(),"historical_anchor_verified":true,"private_vault_witness_refreshed":false})}).collect()
}
fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    (s[(s.len() - 1) / 2] + s[s.len() / 2]) / 2.
}
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("verify") {
        let dir = Path::new(args.get(2).ok_or("missing fixture directory")?);
        let now = Instant::now();
        let v = Verifier::decode(&std::fs::read(dir.join("verifier.bin"))?)?;
        let tx = Write::decode(&std::fs::read(dir.join("write.bin"))?)?;
        let load_ms = ms(now);
        let mut samples = Vec::new();
        for _ in 0..20 {
            let now = Instant::now();
            v.verify(&tx)?;
            samples.push(ms(now));
        }
        println!(
            "{}",
            json!({"phase":"verifier_only","load_ms":load_ms,"verification_ms":samples,"peak_rss_kib":rss()})
        );
        return Ok(());
    }
    eprintln!("YPP3 feasibility counterexamples; synthetic value-preserving rotations only. Local trusted setup.");
    let now = Instant::now();
    let (p, v, params, constraints) = setup()?;
    let setup_ms = ms(now);
    eprintln!("Setup {setup_ms:.0} ms; {constraints} constraints.");
    let (a, av) = Wallet::new((1u64 << 48) + 77);
    let (b, bv) = Wallet::new(u64::MAX - 10);
    let initial = Snapshot::new(params, &[av.clone(), bv.clone()])?;
    let pair_start = Instant::now();
    let ((a_result, a_ms), (b_result, b_ms)) = std::thread::scope(|scope| {
        let ta = scope.spawn(|| {
            let now = Instant::now();
            (p.rotate(&a, &initial, &av), ms(now))
        });
        let tb = scope.spawn(|| {
            let now = Instant::now();
            (p.rotate(&b, &initial, &bv), ms(now))
        });
        (ta.join().unwrap(), tb.join().unwrap())
    });
    let pair_wall_ms = ms(pair_start);
    let (ta, an, sa) = a_result?;
    let (tb, bn, sb) = b_result?;
    let mut sequential = Vec::new();
    let now = Instant::now();
    let (conflict, _, _) = p.rotate(&a, &initial, &av)?;
    sequential.push(ms(now));
    let now = Instant::now();
    let (repeat, an2, _) = p.rotate(&a, &sa, &an)?;
    sequential.push(ms(now));
    let now = Instant::now();
    let (fresh_b, _, merged_fresh) = p.rotate(&b, &sa, &bv)?;
    sequential.push(ms(now));
    let writes = [
        ta.clone(),
        tb.clone(),
        conflict.clone(),
        repeat.clone(),
        fresh_b.clone(),
    ];
    let mut verification = Vec::new();
    for t in &writes {
        let now = Instant::now();
        v.verify(t)?;
        verification.push(ms(now));
        assert_eq!(t.encode().len(), PACKET_BYTES);
    }
    let mut strict = Node::new(initial.root(), Policy::Strict);
    strict.apply(&v, ta.clone())?;
    let independent_rejected = strict.apply(&v, tb.clone()).is_err();
    assert!(independent_rejected);
    strict.apply(&v, fresh_b.clone())?;
    assert_eq!(strict.root, merged_fresh.root());
    let merged = sa.replace(1, bn.commitment())?;
    let mut historical = Node::new(initial.root(), Policy::UnsafeHistoricalReplacement);
    historical.apply(&v, ta.clone())?;
    let conflict_rejected = historical.apply(&v, conflict.clone()).is_err();
    assert!(conflict_rejected);
    historical.apply(&v, tb.clone())?;
    let lost_update = historical.root != merged.root() && historical.root == sb.root();
    assert!(lost_update);
    let mut additive = Node::new(initial.root(), Policy::UnsafeFieldDelta);
    additive.apply(&v, ta.clone())?;
    additive.apply(&v, tb.clone())?;
    let field_delta_wrong = additive.root != merged.root();
    assert!(field_delta_wrong);
    let mut replay = Node::new(initial.root(), Policy::UnsafeHistoricalReplacement);
    let now = Instant::now();
    for r in &historical.records {
        replay.replay(&v, r)?;
    }
    replay.require_head(historical.head)?;
    let recovery_ms = ms(now);
    assert_eq!(replay.root, historical.root);
    let located_a = initial.locate_published_write(an.commitment(), ta.after);
    let located_b = initial.locate_published_write(bn.commitment(), tb.after);
    let located_repeat = sa.locate_published_write(an2.commitment(), repeat.after);
    assert_eq!(located_a, located_repeat);
    assert_eq!(located_a, vec![0]);
    assert_eq!(located_b, vec![1]);
    let id = age::x25519::Identity::generate();
    let backup = a.backup(&av, &id.to_public())?;
    let now = Instant::now();
    let (_, restored) = Wallet::restore(&id, &backup)?;
    let backup_restore_ms = ms(now);
    let old_backup_cannot_recover_current_opening = !sa.contains(&restored);
    assert!(old_backup_cannot_recover_current_opening);
    let offline_recipient_update_rejected = p.rotate(&a, &initial, &bv).is_err();
    assert!(offline_recipient_update_rejected);
    let polynomial_results = [4usize, 16, 64, 256]
        .into_iter()
        .map(polynomial::run)
        .collect::<yamichain_ypp_feasibility::Result<Vec<_>>>()?;
    for r in &polynomial_results {
        assert!(
            r.independent_updates_commute
                && r.refreshed_witness_valid
                && r.stale_witness_rejected
                && r.repeated_hidden_write_reidentified
                && r.same_slot_blind_merge_is_not_a_version_update
        );
        assert_eq!(r.revealed_slots, vec![0]);
    }
    let reference: Value = serde_json::from_str(include_str!(
        "../../yamichain-ypp-hidden/measurements-x86_64.json"
    ))?;
    let ref_proof = reference["prover_and_recovery"]["proving_ms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect::<Vec<_>>();
    let ref_verify = reference["verifier_only"]["verification_ms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect::<Vec<_>>();
    let archives = archives();
    if let Some(path) = args.get(2) {
        std::fs::create_dir_all(path)?;
        let dir = Path::new(path);
        std::fs::write(dir.join("verifier.bin"), v.encode())?;
        std::fs::write(dir.join("write.bin"), ta.encode())?;
    }
    eprintln!("Real hidden-write proofs reproduced strict-root rejection, stale overwrite, conflict rejection and patch enumeration. KZG deltas commute, but public refresh reveals changed positions. Decision: reject tested candidates; no general impossibility claim.");
    let transcript=writes.iter().map(|t|json!({"before":hex(&bytes(&t.before)),"after":hex(&bytes(&t.after)),"version_marker":hex(&t.marker),"packet_hex":hex(&t.encode())})).collect::<Vec<_>>();
    let report = json!({"phase":"prover_counterexamples_and_recovery","decision":"REJECT_TESTED_CANDIDATES_AND_HOLD_PRODUCTION","general_private_vault_feasibility":"UNESTABLISHED_NOT_DISPROVED","constraints":constraints,"setup_ms":setup_ms,"native_two_concurrent_proofs_wall_ms":pair_wall_ms,"native_concurrent_individual_proving_ms":[a_ms,b_ms],"sequential_proving_ms":sequential,"verification_ms":verification,"proof_bytes":192,"packet_bytes":PACKET_BYTES,"authenticated_two_record_replay_ms":recovery_ms,"backup_restore_ms":backup_restore_ms,"peak_rss_kib":rss(),"counterexamples":{"strict_root_rejects_unrelated_proof":independent_rejected,"historical_root_overwrites_unrelated_write":lost_update,"same_vault_conflict_rejected":conflict_rejected,"replay_replicas_agree_on_wrong_state":true,"field_delta_not_correct_merged_root":field_delta_wrong,"published_new_leaf_reveals_slots":[located_a,located_b,located_repeat],"old_backup_cannot_recover_current_opening":old_backup_cannot_recover_current_opening,"candidate_cannot_update_offline_recipient":offline_recipient_update_rejected,"correct_merged_root":hex(&bytes(&merged.root())),"incorrect_historical_root":hex(&bytes(&historical.root)),"incorrect_field_delta_root":hex(&bytes(&additive.root))},"polynomial_commitment_experiments":polynomial_results,"public_anchor_archive":archives,"liveness_simulation":{"assumption":"independent remote prover latencies; single serialized verifier; external background roots; unique conflict-free inputs; acceptance simulation only","milestone2_measured_proof_ms":ref_proof,"milestone2_measured_verify_median_ms":median(&ref_verify),"milestone2_latency_grid":liveness::grid(&ref_proof,median(&ref_verify)),"candidate_rotation_latency_grid":liveness::grid(&sequential,median(&verification)),"neither_grid_establishes_hidden_write_safety":true},"public_root_only_transcript":transcript,"logical_node_growth":{"accepted_unsafe_writes":historical.records.len(),"history_packet_bytes":historical.records.len()*PACKET_BYTES,"permanent_marker_bytes":historical.records.len()*32,"root_set_bytes":(historical.records.len()+1)*32}});
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
